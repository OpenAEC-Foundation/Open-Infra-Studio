"""Exercise the publication gate with the actual manifests and lockfile."""
import importlib.util
from pathlib import Path
import shutil
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('host_api', ROOT / 'scripts/check_host_api.py')
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
REV = 'fb0a667e269b6c924233ecd450a11dd7cba40eea'


class HostApiTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for name in ('Cargo.toml', 'civiel/Cargo.toml', 'Cargo.lock'):
            dest = self.root / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, dest)
        lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
        self.host = {'package': [p for p in lock['package'] if p['name'] in
                     ('opencaddocument', 'serde-reflection') or
                     (p['name'] == 'acadrust' and p['version'] == '0.6.8')]}

    def test_matching(self):
        gate.check(self.root, REV, self.host)

    def test_new_host_release(self):
        with self.assertRaisesRegex(ValueError, 'Cargo.toml'):
            gate.check(self.root, 'a' * 40, self.host)

    def test_either_plugin_pin(self):
        for name in ('Cargo.toml', 'civiel/Cargo.toml'):
            with self.subTest(name=name):
                p = self.root / name
                original = p.read_text()
                p.write_text(original.replace(REV, 'b' * 40))
                with self.assertRaisesRegex(ValueError, name):
                    gate.check(self.root, REV, self.host)
                p.write_text(original)

    def test_stale_lock(self):
        p = self.root / 'Cargo.lock'
        p.write_text(p.read_text().replace(REV, 'b' * 40))
        with self.assertRaisesRegex(ValueError, 'resolved ocs_plugin_api'):
            gate.check(self.root, REV, self.host)

    def test_transitive_drift(self):
        for package in self.host['package']:
            with self.subTest(name=package['name']):
                original = package['source']
                package['source'] = original + '-changed'
                with self.assertRaisesRegex(ValueError, package['name']):
                    gate.check(self.root, REV, self.host)
                package['source'] = original
