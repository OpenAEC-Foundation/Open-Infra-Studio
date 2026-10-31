"""Check an IFC 4.3 export of Open Infra Studio with IfcOpenShell.

    python tests/ifc_validate.py target/ifc-test/voorbeeld.ifc [--max-errors 20]

1. The schema (IFC4X3_ADD2): entity attributes and types, and the EXPRESS
   WHERE rules and functions.
2. The alignments: IfcOpenShell's geometry kernel evaluates the horizontal
   composite curve and the vertical gradient curve; the points must lie on
   the layout segments (start points, start heights, and the parabola
   between them).
3. A summary of the classes, georeference and property sets.

Exits 1 when anything fails. Needs `pip install ifcopenshell pytest` (IfcOpenShell
0.8 or later; its EXPRESS rule checker uses pytest).
"""

import argparse
import collections
import math
import sys

import ifcopenshell
import ifcopenshell.geom
import ifcopenshell.ifcopenshell_wrapper as wrapper
import ifcopenshell.validate


def schema_errors(f, max_errors):
    log = ifcopenshell.validate.json_logger()
    ifcopenshell.validate.validate(f, log, express_rules=True)
    errors = [s for s in log.statements if s.get("level", "ERROR").upper() == "ERROR"]
    kinds = collections.Counter()
    for s in errors:
        msg = str(s.get("message", s))
        kinds[" ".join(msg.split())[:160]] += 1
    for msg, n in kinds.most_common(max_errors):
        print(f"  schema ({n}x): {msg}")
    return len(errors)


def evaluator(settings, curve):
    # IfcOpenShell 0.8 maps the wrapped instance; later versions take the instance itself.
    fn = wrapper.map_shape(settings, getattr(curve, "wrapped_data", curve))
    return wrapper.function_item_evaluator(settings, fn)


def nested(obj, kind):
    for rel in obj.IsNestedBy or []:
        for o in rel.RelatedObjects:
            if o.is_a(kind):
                yield o


def check_alignment(settings, al):
    """Kernel points against the layout; returns the number of misfits."""
    bad = 0
    rep = {r.RepresentationIdentifier: r.Items[0] for r in al.Representation.Representations}
    horizontal = next(nested(al, "IfcAlignmentHorizontal"), None)
    vertical = next(nested(al, "IfcAlignmentVertical"), None)
    plan = rep.get("FootPrint") or rep.get("Axis")
    if horizontal is not None:
        ev = evaluator(settings, plan)
        s = 0.0
        for seg in [o for o in nested(horizontal, "IfcAlignmentSegment")]:
            dp = seg.DesignParameters
            m = ev.evaluate(s)
            x, y = m[0][3], m[1][3]
            want = dp.StartPoint.Coordinates
            if math.hypot(x - want[0], y - want[1]) > 1e-3:
                print(f"  {al.Name}: horizontal at {s:.3f} m: ({x:.3f}, {y:.3f}), layout ({want[0]:.3f}, {want[1]:.3f})")
                bad += 1
            s += dp.SegmentLength
    if vertical is not None:
        ev = evaluator(settings, rep["Axis"])
        for seg in nested(vertical, "IfcAlignmentSegment"):
            dp = seg.DesignParameters
            for t in (0.0, 0.5, 1.0) if dp.HorizontalLength > 0 else (0.0,):
                u = dp.StartDistAlong + t * dp.HorizontalLength
                x = t * dp.HorizontalLength
                want = dp.StartHeight + dp.StartGradient * x
                if dp.PredefinedType == "PARABOLICARC":
                    want += (dp.EndGradient - dp.StartGradient) / (2.0 * dp.HorizontalLength) * x * x
                z = ev.evaluate(u)[2][3]
                if abs(z - want) > 1e-3:
                    print(f"  {al.Name}: height at {u:.3f} m: {z:.4f}, layout {want:.4f} ({dp.PredefinedType})")
                    bad += 1
    return bad


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("path")
    ap.add_argument("--max-errors", type=int, default=20)
    a = ap.parse_args()
    f = ifcopenshell.open(a.path)
    print(f"{a.path}: {f.schema_identifier}, {len(list(f))} instances")
    failures = 0
    if f.schema_identifier != "IFC4X3_ADD2":
        print("  schema identifier is not IFC4X3_ADD2")
        failures += 1
    n = schema_errors(f, a.max_errors)
    print(f"schema and EXPRESS rules: {n} error(s)")
    failures += n
    settings = ifcopenshell.geom.settings()
    misfits = 0
    for al in f.by_type("IfcAlignment"):
        misfits += check_alignment(settings, al)
    print(f"alignments: {len(f.by_type('IfcAlignment'))}, {misfits} misfit(s) between geometry and layout")
    failures += misfits
    counts = collections.Counter(e.is_a() for e in f.by_type("IfcProduct"))
    print("products:", ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))
    for mc in f.by_type("IfcMapConversion"):
        print(f"georeference: {mc.TargetCRS.Name}, base point {mc.Eastings:.3f} {mc.Northings:.3f}")
    psets = collections.Counter(p.Name for p in f.by_type("IfcPropertySet"))
    print("property sets:", ", ".join(f"{k} {v}" for k, v in sorted(psets.items())))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
