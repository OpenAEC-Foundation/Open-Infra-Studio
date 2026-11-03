"""Behoud bronregels in ontwikkelbuilds en laat distributie-instellingen intact."""
from pathlib import Path
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]

PROFILE_EXAMPLE = '''[profile.dev]
debug = "line-tables-only"

[profile.release]
opt-level = 3
'''


class DevelopmentProfileTest(unittest.TestCase):
    def assert_expected_profiles(self, manifest):
        profile = tomllib.loads(manifest).get('profile', {})
        # Geen eigen testprofiel: tests erven dev, inclusief Cargo's standaard
        # voor assertions, overflowchecks en incrementeel compileren. Het
        # profiel ui-test (release met regelnummers) is alleen voor de UI-test.
        ui_test = profile.pop('ui-test', None)
        if ui_test is not None:
            self.assertEqual(ui_test, {'inherits': 'release', 'debug': 'line-tables-only'})
        self.assertEqual(profile, {'dev': {'debug': 'line-tables-only'},
                                   'release': {'opt-level': 3}})
        self.assertIs(type(profile['release']['opt-level']), int)

    def test_development_keeps_line_numbers_without_full_type_debug_information(self):
        manifest = (ROOT / 'Cargo.toml').read_text(encoding='utf-8')
        self.assert_expected_profiles(manifest)

    def test_equivalent_toml_strings_are_accepted(self):
        for value in ['"line-tables-only"', "'line-tables-only'"]:
            with self.subTest(value=value):
                self.assert_expected_profiles(PROFILE_EXAMPLE.replace('"line-tables-only"', value))

    def test_changed_profile_settings_are_rejected(self):
        changed = {
            'volledige debuginformatie': PROFILE_EXAMPLE.replace('"line-tables-only"', '"full"'),
            'eigen testprofiel': PROFILE_EXAMPLE + '\n[profile.test]\ndebug = "line-tables-only"\n',
            'ander release-optimalisatieniveau': PROFILE_EXAMPLE.replace('opt-level = 3', 'opt-level = 2'),
            'release-optimalisatie als tekst': PROFILE_EXAMPLE.replace('opt-level = 3', 'opt-level = "3"'),
            'release-optimalisatie als kommagetal': PROFILE_EXAMPLE.replace('opt-level = 3', 'opt-level = 3.0'),
        }
        for name, manifest in changed.items():
            with self.subTest(setting=name):
                with self.assertRaises(AssertionError):
                    self.assert_expected_profiles(manifest)


if __name__ == '__main__':
    unittest.main()
