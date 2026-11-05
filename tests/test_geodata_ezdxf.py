"""The GEODATA object Open Infra writes (GIS_GEODATA), read by ezdxf: the
coordinate system, the axis order and the drawing → CRS transformation.

The DXF comes from `cargo test --test geodata_export` (target/geodata);
without it or without ezdxf the test is skipped.
"""

import unittest
from pathlib import Path

DXF = Path(__file__).resolve().parents[1] / "target" / "geodata" / "test.dxf"


class GeodataEzdxfTest(unittest.TestCase):
    def test_geodata_is_readable_by_ezdxf(self):
        try:
            import ezdxf
        except ImportError:
            self.skipTest("ezdxf not installed")
        if not DXF.exists():
            self.skipTest("run cargo test --test geodata_export first")
        g = ezdxf.readfile(str(DXF)).modelspace().get_geodata()
        self.assertIsNotNone(g, "no GEODATA in the model space")
        self.assertEqual(g.get_crs(), (28992, True))
        m, epsg = g.get_crs_transformation()
        self.assertEqual(epsg, 28992)
        x, y, _ = m.transform((10, 10, 0))
        self.assertAlmostEqual(x, 104895.0, places=6)
        self.assertAlmostEqual(y, 425340.0, places=6)


if __name__ == "__main__":
    unittest.main()
