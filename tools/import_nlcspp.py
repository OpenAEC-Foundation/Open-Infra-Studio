#!/usr/bin/env python3
"""NLCS++ Netbeheer model for Open Infra, from the official sources of
mijnaansluiting.nl (https://github.com/mijnaansluiting/NLCSpp, Apache-2.0).

    python3 tools/import_nlcspp.py <checkout of mijnaansluiting/NLCSpp>

Writes nlcspp/assets/:
  model-v12.1.json   the information model of NLCS_Netbeheer.xsd V12.1: the
                     project reference and the 56 object types with their
                     fields (type, required, maximum length, code list) and
                     geometry; the general code lists; the code lists of each
                     network operator (their *_Keuzelijst.xsd); the position
                     of attribute values in layer and symbol names (from
                     docs/InformatiemodelNetbeheerv11.xlsx, column "Deel van
                     NLCS laagnaam").
  lagen-<operator>.txt  the layer names of the operator's DWG template (V11),
                     when its template can be read (needs the Open Infra test
                     binary; see --lagen).
"""

import argparse
import html
import json
import re
import sys
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

XS = "{http://www.w3.org/2001/XMLSchema}"
OUT = Path(__file__).resolve().parents[1] / "nlcspp" / "assets"
OPERATORS = {"Alliander": "Alliander", "Enexis": "Enexis", "Stedin": "Stedin"}


def restriction(el):
    """(base, max length, enumeration) of an inline simpleType."""
    r = el.find(f"{XS}simpleType/{XS}restriction")
    if r is None:
        return None, None, None
    ml = r.find(f"{XS}maxLength")
    enum = [e.get("value") for e in r.findall(f"{XS}enumeration")]
    return r.get("base"), int(ml.get("value")) if ml is not None else None, enum or None


GEOMETRY = {
    "gml:PointPropertyType": "point",
    "gml:CurvePropertyType": "curve",
    "gml:SurfacePropertyType": "surface",
    "gml:GeometryPropertyType": "any",
    "gml:MultiPointPropertyType": "multipoint",
    "gml:MultiCurvePropertyType": "multicurve",
    "gml:MultiSurfacePropertyType": "multisurface",
}


def fields(obj, named):
    out, geometry = [], None
    for f in obj.find(f".//{XS}sequence").findall(f"{XS}element"):
        name, typ = f.get("name"), f.get("type")
        required = (f.get("minOccurs") or "1") != "0"
        base, max_len, enum = restriction(f)
        if typ in GEOMETRY:
            geometry = GEOMETRY[typ]
            out.append({"name": name, "type": "geometry", "required": required})
            continue
        entry = {"name": name, "required": required}
        if typ and typ.startswith("xs:"):
            entry["type"] = typ[3:]
        elif typ in named:
            entry["type"] = "list"
            entry["list"] = typ
        elif typ:
            entry["type"] = typ
        else:
            entry["type"] = (base or "xs:string")[3:]
            if max_len:
                entry["max_length"] = max_len
            if enum:
                entry["enum"] = enum
        out.append(entry)
    return out, geometry


def lists_of(root):
    out = {}
    for s in root.iter(f"{XS}simpleType"):
        name = s.get("name")
        if not name:
            continue
        r = s.find(f"{XS}restriction")
        if r is None:
            continue
        out[name] = [e.get("value") for e in r.findall(f"{XS}enumeration")]
    return out


def xlsx_rows(path, sheet):
    z = zipfile.ZipFile(path)
    ss = []
    x = z.read("xl/sharedStrings.xml").decode("utf-8")
    for si in re.findall(r"<si>(.*?)</si>", x, re.S):
        ss.append(html.unescape("".join(re.findall(r"<t[^>]*>(.*?)</t>", si, re.S))))
    wb = z.read("xl/workbook.xml").decode()
    rels = z.read("xl/_rels/workbook.xml.rels").decode()
    targets = {}
    for m in re.finditer(r"<Relationship ([^>]*)/>", rels):
        a = dict(re.findall(r'(\w+)="([^"]*)"', m.group(1)))
        targets[a["Id"]] = a["Target"]
    for name, rid in re.findall(r'<sheet [^>]*name="([^"]+)"[^>]*r:id="([^"]+)"', wb):
        if html.unescape(name) != sheet:
            continue
        x = z.read("xl/" + targets[rid].lstrip("/").replace("xl/", "")).decode("utf-8")
        for r in re.findall(r"<row[^>]*>(.*?)</row>", x, re.S):
            row = {}
            for c in re.finditer(r'<c r="([A-Z]+)\d+"([^>]*?)(?:/>|>(.*?)</c>)', r, re.S):
                col, attrs, body = c.group(1), c.group(2), c.group(3) or ""
                v = re.search(r"<v>(.*?)</v>", body, re.S)
                t = re.search(r"<t[^>]*>(.*?)</t>", body, re.S)
                if 't="s"' in attrs and v:
                    row[col] = ss[int(v.group(1))]
                elif t:
                    row[col] = html.unescape(t.group(1))
                elif v:
                    row[col] = v.group(1)
            yield row


def layer_positions(path):
    """Per object: attribute → position in the layer name, and in the symbol name."""
    layer, symbol = {}, {}
    for r in xlsx_rows(path, "Informatiemodel"):
        obj, attr = (r.get("AV") or "").strip(), (r.get("AW") or "").strip()
        if not obj or not attr:
            continue
        for col, out in (("BB", layer), ("BC", symbol)):
            v = (r.get(col) or "").strip()
            if v.isdigit():
                out.setdefault(obj, {})[attr] = int(v)
    return layer, symbol


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("repo", type=Path, help="checkout of mijnaansluiting/NLCSpp")
    ap.add_argument("--lagen", type=Path, help="folder with <Operator>-lagen.txt dumps of the V11 DWG templates")
    a = ap.parse_args()
    base = a.repo / "schemas" / "V12.1" / "NLCS_Netbeheer.xsd"
    root = ET.parse(base).getroot()
    named = {s.get("name") for s in root.findall(f"{XS}simpleType")} | {c.get("name") for c in root.findall(f"{XS}complexType")}
    top = next(e for e in root.findall(f"{XS}element") if e.get("name") == "NLCSnetbeheer")
    project = top.find(f".//{XS}sequence/{XS}element[@name='AprojectReferentie']")
    model = {
        "versie": "12.1",
        "bron": "mijnaansluiting/NLCSpp schemas/V12.1 (Apache-2.0)",
        "root": "NLCSnetbeheer",
        "namespace": root.get("targetNamespace") or "NS_NLCSnetbeheer",
        "project": fields(project, named)[0],
        "objecten": {},
        "keuzelijsten": lists_of(root),
        "netbeheerders": {},
    }
    for obj in top.find(f".//{XS}choice").findall(f"{XS}element"):
        f, g = fields(obj, named)
        model["objecten"][obj.get("name")] = {"geometrie": g, "velden": f}
    for name, file_part in OPERATORS.items():
        p = a.repo / "schemas" / "V12.1" / f"NLCS_Netbeheer{file_part}V12.1Import_Keuzelijst.xsd"
        if p.exists():
            k = ET.parse(p).getroot()
            model["netbeheerders"][name] = {"xsd": p.name, "namespace": k.get("targetNamespace"), "keuzelijsten": lists_of(k)}
    # The general lists are filled in the base; the operator lists only there.
    model["keuzelijsten"] = {k: v for k, v in model["keuzelijsten"].items() if v}
    lp, sp = layer_positions(a.repo / "docs" / "InformatiemodelNetbeheerv11.xlsx")
    model["laagnaam_posities"] = lp
    model["symboolnaam_posities"] = sp
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "model-v12.1.json").write_text(json.dumps(model, ensure_ascii=False, indent=0, sort_keys=True) + "\n", encoding="utf-8")
    n_fields = sum(len(o["velden"]) for o in model["objecten"].values())
    print(f"{len(model['objecten'])} objecten, {n_fields} velden, {len(model['keuzelijsten'])} algemene keuzelijsten, netbeheerders: {', '.join(model['netbeheerders'])}")
    if a.lagen:
        for name in OPERATORS:
            src = a.lagen / f"{name}-lagen.txt"
            if src.exists():
                names = sorted({l.strip() for l in src.read_text(encoding="utf-8").splitlines() if "-OI-KL-" in l})
                (OUT / f"lagen-{name.lower()}.txt").write_text("\n".join(names) + "\n", encoding="utf-8")
                print(f"lagen-{name.lower()}.txt: {len(names)} lagen")


if __name__ == "__main__":
    sys.exit(main())
