"""Cross-check of the IDS checker in rekenkern (CIV_IFC_IDS) against IfcTester.

`cargo test` writes target/ifc-test/voorbeeld.ifc and, from the same
sample drawing, rekenkern's IDS report target/ifc-test/voorbeeld_ids.json
for tests/data/ifc43-export.ids. IfcTester checks the same IFC against the
same IDS here; per specification both must give the same verdict and the
same number of applicable elements. Skipped without IfcOpenShell or before
`cargo test` has written the files.
"""

import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "ifc-test"

try:
    import ifcopenshell
    from ifc_suite import load_ids
except ImportError:  # pragma: no cover - IfcOpenShell is installed in CI only
    ifcopenshell = None


@unittest.skipIf(ifcopenshell is None, "IfcOpenShell/IfcTester not installed")
class IdsCrossCheck(unittest.TestCase):
    def test_same_verdict_per_specification(self):
        ifc, report = OUT / "voorbeeld.ifc", OUT / "voorbeeld_ids.json"
        if not ifc.is_file() or not report.is_file():
            self.skipTest("run cargo test first (writes target/ifc-test)")
        ours = {s["name"]: s for s in json.loads(report.read_text(encoding="utf-8"))["specifications"]}
        contract = load_ids()
        contract.validate(ifcopenshell.open(str(ifc)), should_filter_version=True)
        self.assertEqual(set(ours), {s.name for s in contract.specifications})
        for spec in contract.specifications:
            mine = ours[spec.name]
            self.assertEqual(mine["status"] == "Pass", bool(spec.status), spec.name)
            applicable = getattr(spec, "applicable_entities", None)
            if applicable is not None:
                self.assertEqual(mine["applicable"], len(applicable), spec.name)


if __name__ == "__main__":
    unittest.main()
