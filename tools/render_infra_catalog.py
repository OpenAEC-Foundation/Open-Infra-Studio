"""Render the Rust-generated geometry as a catalogue overview; no CAD parser is needed."""
import html
import json
import math
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]

def font(size):
    for path in [Path('C:/Windows/Fonts/segoeui.ttf'),Path('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf')]:
        if path.exists():
            return ImageFont.truetype(str(path),size)
    return ImageFont.load_default()

def render(folder):
    data=json.loads((folder/'catalogus.json').read_text(encoding='utf-8'))
    cell_w,cell_h,columns=360,235,4
    width,height=cell_w*columns,110+math.ceil(len(data)/columns)*cell_h
    image=Image.new('RGB',(width,height),'#f3f5f7')
    draw=ImageDraw.Draw(image)
    draw.text((24,18),'Open Infra Studio',font=font(30),fill='#142b40')
    draw.text((24,59),f'{len(data)} infra-objecten | NLCS-classificatie 5.0.2 | schematische catalogus',font=font(18),fill='#486176')
    svg=[f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">',
         '<rect width="100%" height="100%" fill="#f3f5f7"/>',
         '<g font-family="Segoe UI, sans-serif" fill="#142b40">',
         '<text x="24" y="42" font-size="30">Open Infra Studio</text>',
         f'<text x="24" y="78" font-size="18">{len(data)} infra-objecten · NLCS-classificatie 5.0.2 · schematische catalogus</text>']
    for index,item in enumerate(data):
        x,y=(index%columns)*cell_w,110+(index//columns)*cell_h
        draw.rounded_rectangle((x+8,y+8,x+cell_w-8,y+cell_h-8),radius=8,fill='white',outline='#d8e0e6')
        svg.append(f'<rect x="{x+8}" y="{y+8}" width="{cell_w-16}" height="{cell_h-16}" rx="8" fill="white" stroke="#d8e0e6"/>')
        primitives=[p['geometry'] for p in item['built']['parts']]
        wires=[]
        for p in primitives:
            if p['type']=='poly':
                points=p['points'][:]
                if p['closed']:points.append(points[0])
                wires.append(points)
            elif p['type']=='circle':
                c,r=p['center'],p['radius']
                wires.append([[c[0]+r*math.cos(t*math.tau/64),c[1]+r*math.sin(t*math.tau/64)] for t in range(65)])
            elif p['type']=='hatch':
                wires.extend(p['strokes'])
        points=[p for wire in wires for p in wire]
        if points:
            xmin,xmax=min(p[0] for p in points),max(p[0] for p in points)
            ymin,ymax=min(p[1] for p in points),max(p[1] for p in points)
            scale=min((cell_w-90)/max(xmax-xmin,.1),110/max(ymax-ymin,.1))
            def pos(p):return (x+cell_w/2+(p[0]-(xmin+xmax)/2)*scale,y+100-(p[1]-(ymin+ymax)/2)*scale)
            for wire in wires:
                ps=[pos(p) for p in wire]
                draw.line(ps,fill='#24495e',width=2)
                svg.append('<polyline fill="none" stroke="#24495e" stroke-width="2" points="'+' '.join(f'{a:.2f},{b:.2f}' for a,b in ps)+'"/>')
            for port in item['built']['ports']:
                a,b=pos(port['at'])
                draw.ellipse((a-3,b-3,a+3,b+3),fill='#169ac1')
                svg.append(f'<circle cx="{a:.2f}" cy="{b:.2f}" r="3" fill="#169ac1"/>')
        else:
            label='NAP ?' if item['id']=='hoogtepunt' else '0.0 ‰ →' if item['id']=='afschotpijl' else 'A — A'
            draw.text((x+100,y+80),label,font=font(25),fill='#24495e')
            svg.append(f'<text x="{x+100}" y="{y+100}" font-size="25">{html.escape(label)}</text>')
        for offset,text,size,color in [(164,item['name'],16,'#142b40'),(188,item['id'],13,'#486176'),(208,item['category'],12,'#708695')]:
            # Keep long captions inside their cells.
            shown=text if len(text)<39 else text[:36]+'…'
            draw.text((x+20,y+offset),shown,font=font(size),fill=color)
            svg.append(f'<text x="{x+20}" y="{y+offset+size}" font-size="{size}" fill="{color}">{html.escape(shown)}</text>')
    svg.append('</g></svg>')
    (folder/'catalogus.svg').write_text('\n'.join(svg),encoding='utf-8')
    image.save(folder/'catalogus.png')
    image.crop((0,0,width,min(height,110+3*cell_h))).save(folder/'voorvertoning.png')
    print(f'{len(data)} cells rendered to {folder}')

if __name__=='__main__':
    render(Path(sys.argv[1]) if len(sys.argv)>1 else ROOT/'examples/infra-bibliotheek')
