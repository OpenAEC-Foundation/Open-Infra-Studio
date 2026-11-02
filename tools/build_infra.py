"""Build the generic parametric catalogue. No source drawing geometry is copied."""
import csv
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ROWS = {int(r['id']): r for r in csv.DictReader((ROOT / 'assets/nlcs/objects.tsv').open(encoding='utf-8'), delimiter='\t')}
ITEMS = []
SOURCES = [
    {'document': 'NLCS objecttabellen', 'revision': '5.0.2', 'location': 'object-ID', 'url': 'https://github.com/nl-digigo/NLCS', 'license': 'CC-BY-4.0'},
    {'document': 'Generieke parametrische geometrie Open Infra', 'revision': '1', 'location': 'tools/build_infra.py', 'url': '', 'license': 'GPL-3.0-only'},
]

def param(key, default, unit='m', minimum=0.001, maximum=1000, label=None):
    return dict(key=key, label=label or key, default=default, unit=unit, min=minimum, max=maximum)

def add(id, nl, en, cat, obj, kind='point', shape='box', length=1, width=.5, height=.5,
        diameter=0, system='', material='', ports=0, conflicts=None, layers=None, components=None):
    row = ROWS[obj]
    element = 'T' if kind == 'annotation' else ('S' if kind == 'point' and 'S' in row['element'].split('/') else 'G')
    ps = [param('length', length, label='Lengte'), param('width', width, label='Breedte'), param('height', height, label='Hoogte')]
    if diameter:
        ps.append(param('diameter', diameter, 'mm', 1, 3000, 'Aansluitdiameter'))
    if kind == 'line' and system not in ('curb', 'fence', ''):
        ps.append(param('slope', 0, 'm/m', -.25, .25, 'Afschot (positief = dalend)'))
    if shape == 'reducer':
        ps.append(param('out_diameter', 63, 'mm', 1, 3000, 'Uitgaande diameter'))
    if shape in ('bend', 'tee'):
        ps.append(param('angle', 90, 'deg', 5, 175, 'Hoek'))
    if id == 'afschotpijl':
        ps.append(param('slope', 0, 'm/m', -.25, .25, 'Afschot'))
    if kind == 'profile':
        ps = [param('length',length,label='Breedte doorsnede')] + [
            param(f'layer_{k+1}',c['thickness'],label=c['material']) for k,c in enumerate(components or [])]
    ITEMS.append(dict(id=id, name_nl=nl, name_en=en, category=cat, object_id=obj,
        nlcs_version='5.0.2', element=element, kind=kind, shape=shape, system=system,
        material=material, revision=1, keywords=[cat, material, system], parameters=ps,
        port_count=ports, sources=SOURCES, value_status='generic', conflicts=conflicts or [],
        layers=layers or [], components=components or [],
        quantity_unit='m2' if kind=='area' else 'm' if kind=='line' else 'st'))

def layer(material, thickness, obj):
    return dict(material=material, thickness=thickness, object_id=obj)

# Material variants: project references are informative, never universal design rules.
for id,nl,en,obj,mat,depth in [
 ('straatbaksteen-kf','Straatbaksteen keiformaat','Clay pavers large',2529,'baksteen-keperverband',.08),
 ('straatbaksteen-df','Straatbaksteen dikformaat','Clay pavers small',2529,'baksteen-halfsteens',.065),
 ('asfalt-rood','Rood asfalt deklaag 30 mm','Red asphalt wearing course',2468,'asfalt-rood',.03),
 ('asfalt-zwart','Asfalt zwart','Black asphalt',2464,'asfalt',.03),
 ('grasbeton-120','Grasbeton 120 mm','Grass concrete 120 mm',2508,'grasbeton',.12),
 ('grasbeton-100','Grasbeton 100 mm','Grass concrete 100 mm',2508,'grasbeton',.10),
 ('betontegel-300','Betontegel 300×300×50','Concrete tile 300×300×50',2531,'beton',.05),
 ('halfverharding','Halfverharding gestabiliseerd','Stabilised aggregate',2511,'halfverharding',.10),
 ('houtsnipperpad','Houtsnipperpad','Wood chip path',2511,'houtsnippers',.10),
 ('grind','Grindverharding','Gravel',2512,'grind',.10),
 ('betonplaat','Betonplaat','Concrete slab',2484,'beton',.14),
 ('betonstraatsteen','Betonstraatsteen','Concrete paver',2488,'beton',.08)]:
    add(id,nl,en,'verharding',obj,'area','surface',5,2.5,depth,material=mat,
        conflicts=['Bronvarianten grasbeton: 100 mm op blad 03 versus 120 mm in TO; kies expliciet.'] if id.startswith('grasbeton') else [],
        layers=[layer(mat,depth,obj)])

for id,nl,obj,w,h in [('opsluitband-150x250','Opsluitband 150×250',2405,.15,.25),
 ('opsluitband-100x200','Opsluitband 100×200',2401,.10,.20),
 ('trottoirband','Trottoirband',2439,.15,.25),('inritband','Inritband',2395,.20,.20),
 ('gazonband','Gazonband',2377,.10,.20),('verloopband','Verloopband',2376,.15,.25)]:
    add(id,nl,nl,'banden',obj,'line','curb',5,w,h,system='curb',material='beton',ports=2)

for d in (63,75):
    add(f'persleiding-pe{d}',f'Persleiding HDPE Ø{d} SDR17',f'Pressure sewer HDPE {d}', 'riolering',2054,
        'line','pipe',10,d/1000,d/1000,d,'pressure-sewer','HDPE-SDR17',2,
        conflicts=['Afschot projectreferentie: TO circa 5 mm/m; blad 02 voor Ø75 3 mm/m. Nul is geen ontwerpkeuze: stel de waarde expliciet in.'])
    for id,nl,shape,n in [('afsluiter','Afsluiter','valve',2),('balkeerklep','Balkeerklep','check',2),
                        ('pig-stuk','PIG-stuk','tee',3),('ontluchter','Ontluchter','vent',1),('bocht','Bocht','bend',2),('t-stuk','T-stuk','tee',3)]:
        add(f'{id}-pe{d}',f'{nl} Ø{d}',f'{nl} {d}','riolering',2068,'point',shape,.4,.25,.25,d,'pressure-sewer','HDPE',n)
add('reductie-75-63','Verloop Ø75 naar Ø63','Reducer 75 to 63','riolering',2068,'point','reducer',.4,.2,.2,75,'pressure-sewer','HDPE',2)
for id,nl,obj,sys,d,mat in [('dwa-pvc315','DWA PVC Ø315',3940,'gravity-sewer',315,'PVC'),
 ('hwa-pvc315','HWA PVC Ø315',3710,'stormwater',315,'PVC'),
 ('drain-pp160','Drain PP Ø160',1638,'drainage',160,'PP'),
 ('duiker-beton300','Duiker beton Ø300',1429,'stormwater',300,'beton')]:
    add(id,nl,nl,'hemelwater' if sys!='gravity-sewer' else 'riolering',obj,'line','pipe',10,d/1000,d/1000,d,sys,mat,2)
for id,nl,obj,sys,d in [('inspectieput-pers','Inspectieput persriool 1250×1250',2074,'pressure-sewer',75),
 ('inspectieput-dwa','Inspectieput DWA',2257,'gravity-sewer',315),
 ('inspectieput-hwa','Inspectieput HWA',1839,'stormwater',315),
 ('drainageput','Drainageput',1656,'drainage',160),
 ('slokop','Slokop roosterput',1832,'stormwater',315),
 ('uitstroombak','Uitstroombak beton',3500,'drainage',160)]:
    add(id,nl,nl,'hemelwater' if sys in ('stormwater','drainage') else 'riolering',obj,'point','chamber',1.25,1.25,1,d,sys,'beton',4)
add('wadi','Wadi','Infiltration swale','hemelwater',3830,'area','surface',8,4,.2,material='infiltratie')

for id,nl,obj,sys,d,mat in [('waterleiding','Waterleiding','1365','water',75,'PE'),
 ('glasvezel','Glasvezel',750,'data',0,'glasvezel'),('laagspanning','Laagspanningskabel',878,'electric',0,'kabel'),
 ('mantelbuis-water','Mantelbuis water Ø160',1358,'duct-water',160,'PVC'),
 ('mantelbuis-data75','Mantelbuis data Ø75',786,'duct-data',75,'PVC'),
 ('mantelbuis-laad75','Mantelbuis laden Ø75',969,'duct-electric',75,'PVC'),
 ('mantelbuis-laad110','Mantelbuis laden Ø110',969,'duct-electric',110,'PVC')]:
    add(id,nl,nl,'nuts',int(obj),'line','pipe',10,max(d/1000,.02),.1,d,sys,mat,2)
for id,nl,obj,shape,l,w,h,sys,d,n in [
 ('watermeterput','Watermeterput 1160×910',1361,'chamber',1.16,.91,.8,'water',75,2),
 ('meterkast','Meterkast',953,'box',.6,.4,1.4,'electric',0,2),
 ('trafostation','Trafostation 3600×2000',984,'box',3.6,2,2.3,'electric',0,4),
 ('hydrant','Brandhydrant',1281,'hydrant',.3,.3,.8,'water',75,1)]:
    add(id,nl,nl,'nuts',obj,'point',shape,l,w,h,d,sys,ports=n)
for id,nl,obj,shape,l,w,h in [('lichtmast','Lichtmast',911,'lamp',.3,.3,6),
 ('bolderlamp','Bolderverlichting',911,'lamp',.2,.2,1),('voedingskast-ov','Voedingskast OV',3548,'box',.8,.4,1.2),
 ('aftakmof-ov','Aftakmof OV',879,'tee',.3,.15,.15),('laadpaal','Laadpaal',953,'charger',.3,.2,1.5)]:
    add(id,nl,nl,'verlichting',obj,'point',shape,l,w,h,system='electric',ports=1)
for section in (2.5,10,16,25):
    add(f'ov-kabel-{str(section).replace(".","-")}',f'OV kabel 4×{section} mm²','Lighting cable','verlichting',901,'line','pipe',10,.02,.02,system='electric',material=f'kabel-4x{section}',ports=2)

for id,nl,obj,kind,shape,l,w,h in [('boom','Boom kroon en stam',360,'point','tree',4,4,5),
 ('boom-bestaand','Bestaande boom',360,'point','tree',6,6,8),
 ('plantvak','Plantvak heesters',2888,'area','surface',5,3,.5),
 ('kruidenrijk-gras','Kruidenrijk gras',2899,'area','surface',5,3,.3),
 ('bosplantsoen','Bosplantsoen',2897,'area','surface',6,4,1),
 ('riet-oever','Riet en waterplanten',2892,'area','surface',8,3,1),
 ('oeverzone','Natuurlijke oeverzone',2901,'area','surface',8,3,.5)]:
    add(id,nl,nl,'groen',obj,kind,shape,l,w,h)
for id,nl,obj,shape,l,w,h in [('zitbank','Zitbank 2 m',3413,'bench',2,.6,.8),
 ('fietsnietje','Fietsnietje',3406,'rack',.9,.06,.8),('afvalbak','Afvalbak',3401,'box',.5,.5,1),
 ('containeropstelling','Containeropstelplaats',3414,'box',4,2,1.5),
 ('poort','Poort 1200 mm',548,'gate',1.2,.08,1.8),('landhoofd','Landhoofd schematisch',1471,'box',4,1,1),
 ('faunapassage','Faunapassage schematisch',244,'box',2,.5,.4),
 ('landart-palen','Landart palen schematisch',3416,'posts',2,2,1.5),
 ('klimboom','Klimboom schematisch',3398,'log',4,.5,.5)]:
    add(id,nl,nl,'inrichting',obj,'point',shape,l,w,h)
add('hekwerk','Hekwerk dubbelstaafmat','Panel fence','inrichting',548,'line','fence',5,.08,1.8,system='fence',ports=2)
add('verkeersbord','Verkeersbord generiek','Generic traffic sign','verkeer',2638,'point','sign',.6,.1,2)
add('fietsstraat-bord','Bord fietsstraat','Cycle street sign','verkeer',2638,'point','sign',.6,.1,2)
add('waterlijn','Waterlijn','Water line','groen',459,'line','line',10,.1,.1)
for id,nl,obj in [('hoogtepunt','Hoogtepunt NAP',417),('afschotpijl','Afschotpijl',417),('profielmarkering','Profielmarkering',34)]:
    add(id,nl,nl,'annotatie',obj,'annotation','label',1,.5,.2)

# Composed sections retain a separate NLCS mapping for every component.
for id,nl,obj,components in [
 ('profiel-fietsstraat','Opbouw fietsstraat',2468,[layer('asfalt-deklaag',.03,2468),layer('asfalt-tussenlaag',.06,2464)]),
 ('profiel-straatbaksteen','Opbouw straatbaksteen',2529,[layer('straatbaksteen',.08,2529),layer('straatlaag',.05,2503),layer('menggranulaat',.30,2497),layer('zandbed',.50,503)]),
 ('profiel-halfverharding','Opbouw halfverharding',2511,[layer('halfverharding',.10,2511),layer('menggranulaat',.25,2497),layer('zandbed',.50,503)]),
 ('profiel-wadi','Wadi bodemopbouw',3830,[layer('compost-drainagezand',.15,480),layer('drainagezand',.15,505),layer('watervoerend-zand',.60,501)]),
 ('profiel-nutssleuf','Nutsprofiel 0,8×1,2 m',414,[layer('sleufaanvulling',1.2,501)]),
 ('detail-waterhof','Aansluitdetail waterhof',2074,[layer('inspectieput',1.25,2074),layer('persleiding',.075,2054)]),
 ('profiel-oever','Oeverprofiel schematisch',427,[layer('oever',.5,427),layer('ondergrond',.5,501)])]:
    add(id,nl,nl,'profielen',obj,'profile','section',4,2,1,components=components)

# Exact provenance is recorded separately from normative/generic catalogue defaults.
for i in ITEMS:
    i['sources'] = [dict(s) for s in SOURCES]
    i['sources'].append({'document': 'TO Openbaar gebied', 'revision': '2026-10-07',
        'location': 'Technische uitgangspunten; thematische referentie, geometrie opnieuw opgebouwd', 'url':'','license':'reference-only'})
    if i['conflicts']:
        i['value_status'] = 'conflict'
        i['sources'].append({'document':'25-308-01-TEK-DO-02/03', 'revision':'6.0/1.0',
            'location':'legenda persleiding / opbouw grasbeton', 'url':'', 'license':'reference-only'})
    for comp in i['components']:
        assert comp['object_id'] in ROWS
    assert i['object_id'] in ROWS, i['id']

if __name__ == '__main__':
    p=ROOT/'assets/library/infra.json'
    p.write_text(json.dumps(ITEMS, ensure_ascii=False, indent=2)+'\n',encoding='utf-8')
    print(f'{len(ITEMS)} items -> {p}')
