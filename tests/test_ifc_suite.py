"""Bewijs dat het lokale IDS-contract ontbrekende exportinformatie afkeurt."""

import socket
import unittest
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


if __name__ == "__main__":
    unittest.main()
