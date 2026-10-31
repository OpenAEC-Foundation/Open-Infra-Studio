#!/usr/bin/env python3
"""Validate the LandXML and IFC exports of Open Infra offline.

LandXML: against the LandXML 1.2 XSD (lxml), plus counts.
IFC: ifcopenshell opens the file; ``ifcopenshell.validate`` (schema and
EXPRESS rules) must report no errors; the alignments are read back (lengths
from the business logic and from the geometry, start stations from
Pset_Stationing) and the georeferencing (IfcMapConversion, IfcProjectedCRS).

Needs a venv with lxml and ifcopenshell (Python packages from PyPI)::

    python3 -m venv target/e2e-uitwisseling/venv
    target/e2e-uitwisseling/venv/bin/pip install lxml ifcopenshell
    target/e2e-uitwisseling/venv/bin/python tests/e2e/uitwisseling_check.py --landxml f.xml --ifc f.ifc

Prints a JSON report; the exit code is 0 when everything is valid.
Nothing is uploaded anywhere.
"""

import argparse
import json
import math
import sys
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
XSD = REPO / "target" / "e2e-uitwisseling" / "LandXML-1.2.xsd"
XSD_URL = "http://www.landxml.org/schema/LandXML-1.2/LandXML-1.2.xsd"
NS = {"l": "http://www.landxml.org/schema/LandXML-1.2"}


def check_landxml(path):
    from lxml import etree

    if not XSD.exists():
        XSD.parent.mkdir(parents=True, exist_ok=True)
        req = urllib.request.Request(XSD_URL, headers={"User-Agent": "Open-Infra"})
        XSD.write_bytes(urllib.request.urlopen(req, timeout=60).read())
    schema = etree.XMLSchema(etree.parse(str(XSD)))
    doc = etree.parse(str(path))
    ok = schema.validate(doc)
    errors = [f"{e.line}: {e.message}" for e in schema.error_log][:20]
    r = doc.getroot()
    out = {
        "valid": ok,
        "errors": errors,
        "cgpoints": len(r.findall(".//l:CgPoint", NS)),
        "surfaces": [
            {
                "name": s.get("name"),
                "state": s.get("state"),
                "points": len(s.findall(".//l:P", NS)),
                "faces": len(s.findall(".//l:F", NS)),
                "breaklines": len(s.findall(".//l:Breakline", NS)),
            }
            for s in r.findall(".//l:Surface", NS)
        ],
        "alignments": [
            {
                "name": a.get("name"),
                "length": float(a.get("length")),
                "staStart": float(a.get("staStart")),
                "lines": len(a.findall("l:CoordGeom/l:Line", NS)),
                "curves": len(a.findall("l:CoordGeom/l:Curve", NS)),
                "profsurf": len(a.findall(".//l:ProfSurf", NS)),
                "pvi": len(a.findall(".//l:ProfAlign/l:PVI", NS)) + len(a.findall(".//l:ProfAlign/l:ParaCurve", NS)),
            }
            for a in r.findall(".//l:Alignment", NS)
        ],
        "planfeatures": len(r.findall(".//l:PlanFeature", NS)),
        "epsg": (r.find("l:CoordinateSystem", NS).get("epsgCode") if r.find("l:CoordinateSystem", NS) is not None else None),
    }
    return out


def curve_length(settings, curve):
    """Length of a composite or gradient curve evaluated by ifcopenshell."""
    import ifcopenshell.geom
    import ifcopenshell.ifcopenshell_wrapper as w

    fn = w.map_shape(settings, curve.wrapped_data)
    return fn.end() - fn.start() if hasattr(fn, "end") else fn.length()


def check_ifc(path):
    import ifcopenshell
    import ifcopenshell.geom
    import ifcopenshell.util.element
    import ifcopenshell.validate

    f = ifcopenshell.open(str(path))
    logger = ifcopenshell.validate.json_logger()
    ifcopenshell.validate.validate(f, logger, express_rules=True)
    issues = [s for s in logger.statements]
    errors = [i for i in issues if str(i.get("level", "")).lower() in ("error", "40")]
    out = {"schema": f.schema, "issues": [str(i.get("message"))[:300] for i in issues][:30], "error_count": len(errors), "issue_count": len(issues)}

    # Georeferencing.
    mc = f.by_type("IfcMapConversion")
    if mc:
        m = mc[0]
        out["map_conversion"] = {
            "eastings": m.Eastings, "northings": m.Northings, "height": m.OrthogonalHeight,
            "x_abscissa": m.XAxisAbscissa, "x_ordinate": m.XAxisOrdinate, "scale": m.Scale,
            "crs": m.TargetCRS.Name, "vertical": m.TargetCRS.VerticalDatum,
        }

    # Alignments: business logic, geometry, stationing.
    settings = ifcopenshell.geom.settings()
    als = []
    for a in f.by_type("IfcAlignment"):
        info = {"name": a.Name}
        for rel in a.IsNestedBy:
            for o in rel.RelatedObjects:
                if o.is_a("IfcAlignmentHorizontal"):
                    segs = [s.DesignParameters for s in o.IsNestedBy[0].RelatedObjects]
                    info["horizontal_types"] = [s.PredefinedType for s in segs]
                    info["horizontal_length"] = sum(s.SegmentLength for s in segs)
                    info["zero_length_end"] = segs[-1].SegmentLength == 0.0
                elif o.is_a("IfcAlignmentVertical"):
                    segs = [s.DesignParameters for s in o.IsNestedBy[0].RelatedObjects]
                    info["vertical_types"] = [s.PredefinedType for s in segs]
                    info["vertical_length"] = sum(s.HorizontalLength for s in segs)
                elif o.is_a("IfcReferent"):
                    st = ifcopenshell.util.element.get_pset(o, "Pset_Stationing")
                    info["start_station"] = st.get("Station") if st else None
        # The geometry, evaluated: start and end of the composite curve.
        for rep in a.Representation.Representations:
            item = rep.Items[0]
            try:
                shape = ifcopenshell.geom.create_shape(settings, item)
                verts = shape.verts
                pts = [verts[i:i + 3] for i in range(0, len(verts), 3)]
                length = sum(math.dist(pts[i], pts[i + 1]) for i in range(len(pts) - 1))
                info[f"{rep.RepresentationIdentifier}_{rep.RepresentationType}"] = {
                    "start": [round(c, 4) for c in pts[0]], "end": [round(c, 4) for c in pts[-1]],
                    "polyline_length": round(length, 3), "points": len(pts),
                }
            except Exception as e:  # noqa: BLE001
                info[f"{rep.RepresentationIdentifier}_{rep.RepresentationType}"] = f"geometry failed: {e}"
        als.append(info)
    out["alignments"] = als

    # Terrain, roads, earthworks, buildings.
    out["terrain"] = [
        {"name": e.Name, "triangles": len(e.Representation.Representations[0].Items[0].CoordIndex)}
        for e in f.by_type("IfcGeographicElement")
    ]
    out["roads"] = [r.Name for r in f.by_type("IfcRoad")]
    out["road_parts"] = [p.PredefinedType for p in f.by_type("IfcRoadPart")]
    out["courses"] = [c.Name for c in f.by_type("IfcCourse")]
    vols = {}
    for e in f.by_type("IfcEarthworksFill") + f.by_type("IfcEarthworksCut"):
        p = ifcopenshell.util.element.get_pset(e, "OpenInfra_Earthworks") or {}
        fs = e.Representation.Representations[0].Items[0]
        vols[f"{e.is_a()}:{e.Name}"] = {"volume": p.get("Volume"), "closed": fs.Closed, "voids": [r.RelatingBuildingElement.Name for r in getattr(e, "VoidsElements", [])]}
    out["earthworks"] = vols
    out["buildings"] = len(f.by_type("IfcBuilding"))
    # Geometry of every product with a body.
    failed = 0
    bodies = 0
    for p in f.by_type("IfcProduct"):
        if p.is_a("IfcAlignment") or p.is_a("IfcAlignmentSegment") or not p.Representation:
            continue
        bodies += 1
        try:
            ifcopenshell.geom.create_shape(settings, p)
        except Exception:  # noqa: BLE001
            failed += 1
    out["bodies"] = bodies
    out["bodies_failed"] = failed
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--landxml")
    ap.add_argument("--ifc")
    a = ap.parse_args()
    report = {}
    ok = True
    if a.landxml:
        report["landxml"] = check_landxml(a.landxml)
        ok &= report["landxml"]["valid"]
    if a.ifc:
        report["ifc"] = check_ifc(a.ifc)
        ok &= report["ifc"]["error_count"] == 0 and report["ifc"]["bodies_failed"] == 0
    print(json.dumps(report, indent=1, default=str))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
