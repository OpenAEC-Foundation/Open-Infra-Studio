"""Regressies voor de exitcode van de IFC-controle, inclusief parserfouten."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import ifcopenshell


class IfcValidationTest(unittest.TestCase):
    def check_file(self, record="", schema="IFC4X3_ADD2"):
        model = ifcopenshell.file(schema=schema)
        model.create_entity("IfcCartesianPoint", (0.0, 0.0, 0.0))
        data = model.to_string().replace(
            "ENDSEC;\nEND-ISO", record + "\nENDSEC;\nEND-ISO"
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "model.ifc"
            path.write_text(data, encoding="utf-8")
            return subprocess.run(
                [sys.executable, str(Path(__file__).with_name("ifc_validate.py")), str(path)],
                capture_output=True,
                text=True,
                timeout=60,
            )

    def test_valid_ifc43_succeeds(self):
        result = self.check_file()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_parser_errors_fail(self):
        for record in (
            "#2=IFCNO_SUCH_ENTITY();",
            "#1=IFCCARTESIANPOINT((1.,2.,3.));",
            "#2=IFCCARTESIANPOINT((1.,2.,3.),99);",
        ):
            with self.subTest(record=record):
                result = self.check_file(record)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn("schema (", result.stdout)

    def test_schema_errors_fail(self):
        result = self.check_file("#2=IFCAXIS2PLACEMENT3D(#999,$,$);")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)

    def test_wrong_schema_fails(self):
        result = self.check_file(schema="IFC4")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("schema identifier is not IFC4X3_ADD2", result.stdout)


if __name__ == "__main__":
    unittest.main()
