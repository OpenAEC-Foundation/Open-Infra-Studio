"""Bewijs dat het lokale IDS-contract ontbrekende exportinformatie afkeurt."""

import json
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import ifcopenshell
from ifc_suite import check_ids, load_ids


class ExportContractTest(unittest.TestCase):
    def model(self, name="Voorbeeld", layer=True):
        model = ifcopenshell.file(schema="IFC4X3_ADD2")
        model.create_entity("IfcProject", GlobalId=ifcopenshell.guid.new(), Name=name)
        obj = model.create_entity("IfcAnnotation", GlobalId=ifcopenshell.guid.new(), Name="Object")
        if layer:
            prop = model.create_entity(
                "IfcPropertySingleValue", Name="Laag",
                NominalValue=model.create_entity("IfcLabel", "N-XX-KL-GAS_LD-G"),
            )
            pset = model.create_entity(
                "IfcPropertySet", GlobalId=ifcopenshell.guid.new(),
                Name="OpenInfra_NLCS", HasProperties=[prop],
            )
            model.create_entity(
                "IfcRelDefinesByProperties", GlobalId=ifcopenshell.guid.new(),
                RelatedObjects=[obj], RelatingPropertyDefinition=pset,
            )
        return model

    def test_contract_passes_without_network(self):
        with patch.object(socket, "socket", side_effect=AssertionError("Netwerk verboden")):
            contract = load_ids()
            self.assertEqual(check_ids(self.model(), contract), [])

    def test_missing_project_name_fails(self):
        self.assertTrue(check_ids(self.model(name=None), load_ids()))

    def test_missing_layer_property_fails(self):
        self.assertTrue(check_ids(self.model(layer=False), load_ids()))

    def test_missing_project_fails(self):
        self.assertTrue(check_ids(ifcopenshell.file(schema="IFC4X3_ADD2"), load_ids()))


class SuiteExportTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tests = Path(__file__).resolve().parent
        # cargo test --test ifc_examples schrijft deze echte productie-export.
        # Een ontbrekende fixture is een fout, geen overgeslagen controle.
        cls.export = (cls.tests.parent / "target/ifc-test/millimeters.ifc").read_text(encoding="utf-8")
        cls.expected = json.loads((cls.tests / "data/ifc43-cases.json").read_text(encoding="utf-8"))["millimeters.ifc"]

    def run_export(self, data):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            # De ongewijzigde CLI's met alleen de millimetercase in de manifest.
            # Zo controleren we ook de proces-exitcode en het geschreven rapport.
            for script in ("ifc_suite.py", "ifc_validate.py"):
                shutil.copyfile(self.tests / script, root / script)
            (root / "data").mkdir()
            shutil.copyfile(self.tests / "data/ifc43-export.ids", root / "data/ifc43-export.ids")
            (root / "data/ifc43-cases.json").write_text(
                json.dumps({"millimeters.ifc": self.expected}), encoding="utf-8"
            )
            exports = root / "exports"
            exports.mkdir()
            (exports / "millimeters.ifc").write_text(data, encoding="utf-8")
            report = root / "validatie.json"
            result = subprocess.run(
                [sys.executable, str(root / "ifc_suite.py"), str(exports), "--report", str(report)],
                capture_output=True, text=True, timeout=60,
            )
            self.assertTrue(report.is_file(), result.stdout + result.stderr)
            return result, json.loads(report.read_text(encoding="utf-8"))["modellen"][0]

    def add_record(self, record):
        self.assertEqual(self.export.count("ENDSEC;\nEND-ISO"), 1)
        return self.export.replace("ENDSEC;\nEND-ISO", record + "\nENDSEC;\nEND-ISO")

    def assert_parser_failure(self, data):
        result, report = self.run_export(data)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(report["schema_exitcode"], 1, report)
        self.assertIn("Schema-, EXPRESS- of geometriecontrole mislukt", report["fouten"])

    def test_valid_millimeter_export_succeeds(self):
        result, report = self.run_export(self.export)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(report["fouten"], [])

    def test_unknown_entity_fails_the_suite(self):
        self.assert_parser_failure(self.add_record("#999999=IFCNO_SUCH_ENTITY();"))

    def test_extra_attribute_fails_the_suite(self):
        data, count = re.subn(r"(?m)^(#\d+=IFCCARTESIANPOINT\(.+)\);$", r"\1,99);", self.export, count=1)
        self.assertEqual(count, 1, "de mutatie moet werkelijk een attribuut toevoegen")
        self.assert_parser_failure(data)

    def test_duplicate_step_id_fails_the_suite(self):
        record = re.search(r"(?m)^#\d+=[^\r\n]+;$", self.export)
        self.assertIsNotNone(record)
        self.assert_parser_failure(self.add_record(record.group(0)))

    def test_two_projects_pass_ids_but_fail_the_suite(self):
        model = ifcopenshell.file.from_string(self.export)
        project = model.by_type("IfcProject")[0]
        model.create_entity(
            "IfcProject", GlobalId=ifcopenshell.guid.new(), Name="Tweede project",
            RepresentationContexts=project.RepresentationContexts, UnitsInContext=project.UnitsInContext,
        )
        self.assertEqual(check_ids(model, load_ids()), [], "IDS telt het maximumaantal projecten niet")
        result, report = self.run_export(model.to_string())
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        # De EXPRESS-regel IfcSingleProjectInstance keurt dit al af voordat
        # de aanvullende telling uit het modellenoverzicht wordt uitgevoerd.
        self.assertEqual(report["schema_exitcode"], 1, report)
        self.assertIn("sizeof(IfcProject)", report["schema_uitvoer"])


if __name__ == "__main__":
    unittest.main()
