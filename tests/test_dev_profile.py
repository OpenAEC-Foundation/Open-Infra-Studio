"""Behoud bronregels in ontwikkelbuilds en laat distributie-instellingen intact."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]


def profiles(manifest):
    # Alleen de profieltabellen; de echte Cargo-build valideert het manifest.
    sections = re.split(r'(?m)^\[([^\n]+)\]\s*$', manifest)
    return {sections[i]: [line.split('#', 1)[0].strip()
                         for line in sections[i + 1].splitlines()
                         if line.split('#', 1)[0].strip()]
            for i in range(1, len(sections), 2) if sections[i].startswith('profile.')}


class DevelopmentProfileTest(unittest.TestCase):
    def test_development_keeps_line_numbers_without_full_type_debug_information(self):
        manifest = (ROOT / 'Cargo.toml').read_text(encoding='utf-8')
        profile = profiles(manifest)
        self.assertEqual(profile.get('profile.dev'), ['debug = "line-tables-only"'])
        # Geen eigen testprofiel: tests erven dev, inclusief Cargo's standaard
        # voor assertions, overflowchecks en incrementeel compileren.
        self.assertEqual(set(profile), {'profile.dev', 'profile.release'})
        self.assertEqual(profile['profile.release'], ['opt-level = 3'])


if __name__ == '__main__':
    unittest.main()
