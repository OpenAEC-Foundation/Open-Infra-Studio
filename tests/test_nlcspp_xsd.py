"""The NLCS++ V12.1 files Open Infra writes, validated against the operators'
XSDs (tests/fixtures/nlcspp/xsd, from mijnaansluiting/NLCSpp).

The files come from `cargo test --test nlcspp` (target/nlcspp/*-v12.1.xml);
without them, without xmlschema or without the GML schemas of
schemas.opengis.net (network), the test is skipped.
"""

import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
XSD = ROOT / "tests" / "fixtures" / "nlcspp" / "xsd"
OUT = ROOT / "target" / "nlcspp"
CASES = {"alliander-demo": "Alliander", "enexis-voorbeeld": "Enexis"}


class NlcsppXsdTest(unittest.TestCase):
    def test_written_v12_1_files_are_valid(self):
        try:
            import xmlschema
        except ImportError:
            self.skipTest("xmlschema not installed")
        files = {n: OUT / f"{n}-v12.1.xml" for n in CASES}
        if not all(f.exists() for f in files.values()):
            self.skipTest("run cargo test --test nlcspp first")
        for name, op in CASES.items():
            try:
                schema = xmlschema.XMLSchema(str(XSD / f"NLCS_Netbeheer{op}V12.1Import_Keuzelijst.xsd"))
            except Exception as e:  # the GML schemas come over the network
                self.skipTest(f"schema not loadable: {e}")
            errors = list(schema.iter_errors(str(files[name])))
            # The example data (V11) has values the V12.1 code lists dropped:
            # those are content, and our own check must find the same ones.
            value = [e for e in errors if "value must be one of" in str(e.reason)]
            other = [f"{e.path}: {e.reason}" for e in errors if e not in value]
            self.assertEqual([], other[:10], f"{name}: {len(other)} structural errors")
            ours = json.loads((OUT / f"{name}-keuzelijstfouten.json").read_text())
            self.assertEqual(len(value), len(ours), f"{name}: XSD {len(value)} code-list errors, Open Infra's check {len(ours)}")

    def test_relations_survive_import_and_export(self):
        """A cable with linked attachments, imported and exported again
        (`import_then_export_keeps_relations_and_their_references`): every
        reference (IDREF) must name an object of the new delivery."""
        try:
            import xmlschema
        except ImportError:
            self.skipTest("xmlschema not installed")
        path = OUT / "relaties-v12.1.xml"
        if not path.exists():
            self.skipTest("run cargo test --test nlcspp first")
        try:
            schema = xmlschema.XMLSchema(str(XSD / "NLCS_NetbeheerEnexisV12.1Import_Keuzelijst.xsd"))
        except Exception as e:  # the GML schemas come over the network
            self.skipTest(f"schema not loadable: {e}")
        errors = [f"{e.path}: {e.reason}" for e in schema.iter_errors(str(path))]
        self.assertEqual([], errors)


if __name__ == "__main__":
    unittest.main()
