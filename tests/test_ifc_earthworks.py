"""Volume en geslotenheid van de daadwerkelijk geëxporteerde grondlichamen."""
import collections
import math
import unittest
from pathlib import Path

import ifcopenshell
from ifcopenshell.util.element import get_psets


class EarthworksExportTest(unittest.TestCase):
    def check_body(self, element, expected):
        representations = element.Representation.Representations
        self.assertEqual(len(representations), 1)
        items = representations[0].Items
        self.assertEqual(len(items), 1)
        body = items[0]
        self.assertTrue(body.is_a("IfcTriangulatedFaceSet"))
        self.assertTrue(body.Closed)
        points = body.Coordinates.CoordList
        triangles = body.CoordIndex
        edges = collections.Counter()
        terms = []
        origin = points[0]
        for triangle in triangles:
            a, b, c = triangle
            edges.update(((a, b), (b, c), (c, a)))
            # Een eigen berekening op de geschreven geometrie, rond een lokaal
            # punt om verlies van precisie bij grote coördinaten te voorkomen.
            a, b, c = [tuple(v - o for v, o in zip(points[i - 1], origin)) for i in triangle]
            terms.append((
                a[0] * (b[1] * c[2] - b[2] * c[1])
                - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0])
            ) / 6)
        for (a, b), count in edges.items():
            self.assertEqual(count, edges[b, a], f"Open rand {a}–{b}")
        volume = math.fsum(terms)
        self.assertAlmostEqual(volume, expected, delta=1e-6)
        properties = get_psets(element)["OpenInfra_Earthworks"]
        self.assertAlmostEqual(properties["Volume"], volume, delta=0.0005)

    def check_case(self, name, fill, cut):
        directory = Path(__file__).resolve().parent.parent / "target/ifc-test"
        for unit in ("m", "mm"):
            for location in ("lokaal", "rd"):
                with self.subTest(unit=unit, location=location):
                    path = directory / f"grondlichaam-{name}-{unit}-{location}.ifc"
                    self.assertTrue(path.is_file(), f"De Rust-exportfixture ontbreekt: {path}")
                    model = ifcopenshell.open(str(path))
                    fills = model.by_type("IfcEarthworksFill")
                    cuts = model.by_type("IfcEarthworksCut")
                    self.assertEqual(len(fills), 1)
                    self.assertEqual(len(cuts), int(cut > 0))
                    self.check_body(fills[0], fill)
                    if cuts:
                        self.check_body(cuts[0], cut)

    def test_ground_peak(self):
        self.check_case("top", 500 / 3, 0)

    def test_ground_valley(self):
        self.check_case("dal", 700 / 3, 0)

    def test_crossing_surfaces(self):
        self.check_case("snijding", 125 / 6, 25 / 6)


if __name__ == "__main__":
    unittest.main()
