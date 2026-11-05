"""Onderteken bestaande macOS-distributies zonder de applicatie uit te voeren."""
import os,pathlib,json,subprocess,hashlib,plistlib,time,shutil
asset=json.loads(os.environ['SIGNING_ASSET_JSON']);root=pathlib.Path('macos-signing');root.mkdir(exist_ok=True)
identity=os.environ['APPLE_SIGNING_IDENTITY']
if not identity.startswith('Developer ID Application:'):raise RuntimeError('Developer ID Application vereist')
if pathlib.Path(asset['name']).name!=asset['name']:raise RuntimeError('Ongeldige bestandsnaam')
def run(args,timeout=300,input=None,check=True):
 p=subprocess.run(args,input=input,text=True,capture_output=True,timeout=timeout)
 if check and p.returncode:raise RuntimeError('Opdracht mislukt: '+args[0]+'\n'+p.stdout[-10000:]+'\n'+p.stderr[-10000:])
 return p
source=root/asset['name']
run(['gh','release','download',asset['tag'],'--repo',asset['repo'],'--pattern',asset['name'],'--dir',str(root)],timeout=600)
assert hashlib.file_digest(open(source,'rb'),'sha256').hexdigest()==asset['sha256'],'Oorspronkelijke SHA256 wijkt af'
if asset.get('kind')=='plugin':
 out=root/'output';out.mkdir();library=out/asset['name'];shutil.copy2(source,library)
 run(['codesign','--force','--timestamp','--sign',identity,str(library)])
 verification=run(['codesign','--verify','--strict','--verbose=2',str(library)])
 archive=out/(asset['name']+'.notarized.zip')
 run(['ditto','-c','-k','--keepParent',str(library),str(archive)])
 response=run(['xcrun','notarytool','submit',str(archive),'--key',os.environ['APPLE_API_KEY_PATH'],'--key-id',os.environ['APPLE_API_KEY'],'--issuer',os.environ['APPLE_API_ISSUER'],'--wait','--timeout','25m','--output-format','json'],timeout=1700)
 submission=json.loads(response.stdout);(out/'notarization.json').write_text(json.dumps(submission,indent=2))
 if submission.get('status')!='Accepted':raise RuntimeError('Apple accepteert de plugin niet: '+submission.get('status','onbekend'))
 result=asset|{'status':'Plugin met Developer ID ondertekend; Apple Accepted','signed_sha256':hashlib.file_digest(open(library,'rb'),'sha256').hexdigest(),'notarization_id':submission['id'],'verification_output':verification.stdout+verification.stderr,'note':'Losse dylib: geen stapling ondersteund; Apple heeft de bibliotheek in de ZIP beoordeeld.'}
 (out/'signature-results.json').write_text(json.dumps(result,indent=2));print(asset['repo'],asset['tag'],result['status'],flush=True)
 raise SystemExit(0)
mount=(root/'mount').resolve();mount.mkdir();payload=(root/'payload').resolve()
run(['hdiutil','attach',str(source),'-readonly','-nobrowse','-noautoopen','-mountpoint',str(mount)],input='Y\n')
try:run(['ditto',str(mount),str(payload)],timeout=600)
finally:run(['hdiutil','detach',str(mount)])
apps=list(payload.glob('*.app'))
if not apps:raise RuntimeError('Geen applicatie in de DMG gevonden')
magic={b'\xfe\xed\xfa\xce',b'\xce\xfa\xed\xfe',b'\xfe\xed\xfa\xcf',b'\xcf\xfa\xed\xfe',b'\xca\xfe\xba\xbe',b'\xca\xfe\xba\xbf',b'\xbe\xba\xfe\xca',b'\xbf\xba\xfe\xca'}
signed=[]
def sign(path):
 args=['codesign','--force','--timestamp','--options','runtime','--preserve-metadata=identifier','--sign',identity]
 ent=run(['codesign','-d','--entitlements',':-',str(path)],check=False)
 if ent.stdout.strip().startswith('<?xml') or ent.stdout.strip().startswith('<!DOCTYPE'):
  try:
   obj=plistlib.loads(ent.stdout.encode());obj.pop('com.apple.security.get-task-allow',None)
   ep=root/('entitlements-'+str(len(signed))+'.plist');ep.write_bytes(plistlib.dumps(obj));args+=['--entitlements',str(ep)]
  except plistlib.InvalidFileException:raise RuntimeError('Bestaande entitlements konden niet worden behouden')
 run(args+[str(path)],timeout=300);signed.append(str(path.relative_to(payload)))
for app in apps:
 for path in sorted(app.rglob('*'),key=lambda p:len(p.parts),reverse=True):
  if path.is_symlink() or not path.is_file():continue
  with open(path,'rb') as f:head=f.read(4)
  if head in magic:sign(path)
 bundles=[p for p in app.rglob('*') if p.is_dir() and not p.is_symlink() and p.suffix in ['.framework','.app','.xpc','.appex']]
 for path in sorted(bundles,key=lambda p:len(p.parts),reverse=True):sign(path)
 sign(app);run(['codesign','--verify','--deep','--strict','--verbose=2',str(app)])
out=root/'output';out.mkdir();dmg=out/asset['name']
run(['hdiutil','create','-volname',apps[0].stem,'-srcfolder',str(payload),'-format','UDZO','-ov',str(dmg)],timeout=900)
run(['codesign','--force','--timestamp','--sign',identity,str(dmg)])
notary=run(['xcrun','notarytool','submit',str(dmg),'--key',os.environ['APPLE_API_KEY_PATH'],'--key-id',os.environ['APPLE_API_KEY'],'--issuer',os.environ['APPLE_API_ISSUER'],'--wait','--timeout','25m','--output-format','json'],timeout=1700)
submission=json.loads(notary.stdout);(out/'notarization.json').write_text(json.dumps(submission,indent=2))
if submission.get('status')!='Accepted':
 log=run(['xcrun','notarytool','log',submission['id'],'--key',os.environ['APPLE_API_KEY_PATH'],'--key-id',os.environ['APPLE_API_KEY'],'--issuer',os.environ['APPLE_API_ISSUER']],check=False)
 (out/'notarization-failure-log.json').write_text(log.stdout)
 raise RuntimeError('Apple heeft notarization niet geaccepteerd: '+submission.get('status','onbekend'))
run(['xcrun','stapler','staple',str(dmg)]);staple=run(['xcrun','stapler','validate',str(dmg)])
run(['codesign','--verify','--verbose=2',str(dmg)])
gatekeeper=[]
for app in apps:
 for attempt in range(4):
  p=run(['spctl','--assess','--type','execute','--verbose=4',str(app)],check=False)
  if p.returncode==0:break
  if attempt<3:time.sleep(10)
 if p.returncode:raise RuntimeError('Gatekeeper accepteert de applicatie niet: '+p.stderr)
 gatekeeper.append({'app':app.name,'output':p.stdout+p.stderr})
assessment=run(['spctl','--assess','--type','open','--context','context:primary-signature','--verbose=4',str(dmg)])
result=asset|{'status':'Developer ID ondertekend; Apple Accepted; DMG-ticket geldig; Gatekeeper geaccepteerd','signed_sha256':hashlib.file_digest(open(dmg,'rb'),'sha256').hexdigest(),'signed_files':signed,'notarization_id':submission['id'],'gatekeeper_apps':gatekeeper,'gatekeeper_dmg':assessment.stdout+assessment.stderr,'stapler_output':staple.stdout+staple.stderr}
(out/'signature-results.json').write_text(json.dumps(result,indent=2))
print(asset['repo'],asset['tag'],asset['name'],result['status'],flush=True)
