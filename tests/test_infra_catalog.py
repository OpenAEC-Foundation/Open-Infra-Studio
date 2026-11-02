"""Independent checks of the distributable infra catalogue."""
import csv
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]

class CatalogueTest(unittest.TestCase):
    def test_mappings_provenance_and_conflicts(self):
        items = json.loads((ROOT / 'assets/library/infra.json').read_text(encoding='utf-8'))
        rows = {int(r['id']): r for r in csv.DictReader((ROOT / 'assets/nlcs/objects.tsv').read_text(encoding='utf-8').splitlines(), delimiter='\t')}
        self.assertGreaterEqual(len(items), 60)
        self.assertEqual(len(items), len({i['id'] for i in items}))
        for i in items:
            with self.subTest(item=i['id']):
                self.assertIn(i['object_id'], rows)
                self.assertTrue(i['sources'])
                self.assertEqual(i['nlcs_version'], '5.0.2')
                self.assertIn(i['element'], rows[i['object_id']]['element'].split('/') + ['G', 'T'])
                for p in i['parameters']:
                    self.assertLessEqual(p['min'], p['default'])
                    self.assertLessEqual(p['default'], p['max'])
        pipe = next(i for i in items if i['id'] == 'persleiding-pe75')
        self.assertTrue(pipe['conflicts'])
        self.assertEqual(next(p for p in pipe['parameters'] if p['key'] == 'slope')['default'], 0)

if __name__ == '__main__':
    unittest.main()
