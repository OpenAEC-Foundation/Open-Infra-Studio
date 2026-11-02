"""Behoud van ontgravingsgeometrie en eigenschappen bij weggevallen terrein."""

import unittest
from pathlib import Path

import ifcopenshell
from ifcopenshell.util.element import get_psets
from ifcopenshell.util.placement import get_local_placement


class ExcavationExportTest(unittest.TestCase):
    def check_excavation(self, model):
        objects = model.by_type("IfcGeographicElement")
        self.assertEqual(len(objects), 1)
        excavation = objects[0]
        self.assertEqual(excavation.PredefinedType, "USERDEFINED")
        self.assertIsNotNone(excavation.Representation)
        self.assertTrue(excavation.Representation.is_a("IfcProductDefinitionShape"))
        representations = excavation.Representation.Representations
        self.assertEqual(len(representations), 1)
        footprint = representations[0]
        self.assertEqual((footprint.RepresentationIdentifier, footprint.RepresentationType), ("FootPrint", "Curve2D"))
        self.assertEqual(len(footprint.Items), 1)
        curve = footprint.Items[0]
        self.assertTrue(curve.is_a("IfcIndexedPolyCurve"))
        self.assertTrue(curve.Points.is_a("IfcCartesianPointList2D"))
        self.assertEqual(curve.Points.CoordList, ((0., 10.), (5., 10.), (5., 15.), (0., 15.)))
        self.assertEqual(len(curve.Segments), 1)
        self.assertTrue(curve.Segments[0].is_a("IfcLineIndex"))
        self.assertEqual(curve.Segments[0].wrappedValue, (1, 2, 3, 4, 1))
        self.assertIsNotNone(excavation.ObjectPlacement)
        self.assertEqual(get_local_placement(excavation.ObjectPlacement).tolist(), [
            [1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [0., 0., 0., 1.],
        ])
        properties = get_psets(excavation, psets_only=True, should_inherit=False)
        nlcs = properties.get("OpenInfra_NLCS", {})
        self.assertEqual({k: v for k, v in nlcs.items() if k != "id"}, {
            "Laag": "N-XX-GW-ONTGRAVINGSVAK-WERK-G",
            "Status": "N (N — nieuw)",
            "Discipline": "XX",
            "Hoofdgroep": "GW (GrondWerk)",
            "Object": "ONTGRAVINGSVAK-WERK",
            "Element": "G",
        })
        self.assertEqual(
            properties.get("OpenInfra_ObjectData", {}).get("controle_id"),
            "ontgraving-behoud-017",
        )

    def check_file(self, name):
        # De Rust-regressies schrijven deze bestanden ook in de offline CI.
        path = Path(__file__).resolve().parent.parent / "target/ifc-test" / name
        self.assertTrue(path.is_file(), f"Voer eerst de Rust-ontgravingstests uit: {path}")
        self.check_excavation(ifcopenshell.open(str(path)))

    def test_collapsed_terrain_preserves_geometry_and_properties(self):
        self.check_file("ontgraving-ingeklapt-terrein.ifc")

    def test_terrain_outline_preserves_geometry_and_properties(self):
        self.check_file("ontgraving-terreinomtrek.ifc")


if __name__ == "__main__":
    unittest.main()
