"""Controleer de buildaanroep zonder een compiler te starten."""
import json
import os
from pathlib import Path
import subprocess
import shutil
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.name == 'posix' and shutil.which('bash') and shutil.which('dirname'),
                     'Deze shellcontrole vereist POSIX met Bash en dirname')
class CachedBuildTest(unittest.TestCase):
    def invoke(self, with_cache, explicit_wrapper=None, client=None, fallback=None, exit_code=0,
               with_cdpath=False, release=False):
        with tempfile.TemporaryDirectory(prefix='ois-build-script-') as directory:
            temp = Path(directory)
            bindir = temp / 'bin'
            bindir.mkdir()
            capture = temp / 'cargo.json'
            cargo = bindir / 'cargo'
            cargo.write_text('#!' + sys.executable + '\n' + '''import json,os,sys
from pathlib import Path
Path(os.environ['BUILD_CAPTURE']).write_text(json.dumps({
 'args':sys.argv[1:],'cwd':os.getcwd(),
 'wrapper':os.environ.get('RUSTC_WRAPPER'),
 'client':os.environ.get('SCCACHE_CLIENT_SIDE'),
 'fallback':os.environ.get('SCCACHE_IGNORE_SERVER_IO_ERROR')}))
raise SystemExit(int(os.environ['BUILD_EXIT']))
''')
            cargo.chmod(0o755)
            (bindir / 'dirname').symlink_to(shutil.which('dirname'))
            if with_cache:
                cache = bindir / 'sccache'
                cache.write_text('#!/bin/sh\nexit 0\n')
                cache.chmod(0o755)
            env = {key: value for key, value in os.environ.items()
                   if key not in {'RUSTC_WRAPPER', 'SCCACHE_CLIENT_SIDE', 'SCCACHE_IGNORE_SERVER_IO_ERROR'}}
            env.update(PATH=str(bindir),
                       BUILD_CAPTURE=str(capture), BUILD_EXIT=str(exit_code))
            if explicit_wrapper is not None:
                env['RUSTC_WRAPPER'] = explicit_wrapper
            if client is not None:
                env['SCCACHE_CLIENT_SIDE'] = client
            if fallback is not None:
                env['SCCACHE_IGNORE_SERVER_IO_ERROR'] = fallback
            script = str(ROOT / 'scripts/build.sh')
            cwd = temp
            if with_cdpath:
                other = temp / 'andere-checkout'
                (other / 'scripts').mkdir(parents=True)
                env['CDPATH'] = str(other)
                script = 'scripts/build.sh'
                cwd = ROOT
            args = ['--locked', '-p', 'open-infra', '--target-dir', 'target met spaties']
            if release:
                args.insert(0, '--release')
            process = subprocess.run([shutil.which('bash'), script, *args],
                                     cwd=cwd, env=env, capture_output=True, text=True)
            self.assertTrue(capture.exists(), process.stderr)
            return process.returncode, json.loads(capture.read_text())

    def test_without_cache_keeps_the_plain_cargo_build(self):
        code, result = self.invoke(False)
        self.assertEqual(code, 0)
        self.assertEqual(result['args'], ['build', '--locked', '-p', 'open-infra', '--target-dir', 'target met spaties'])
        self.assertEqual(result['cwd'], str(ROOT))
        self.assertIsNone(result['wrapper'])

    def test_available_cache_uses_client_mode_and_falls_back_on_server_io_errors(self):
        code, result = self.invoke(True)
        self.assertEqual(code, 0)
        self.assertEqual((result['wrapper'], result['client'], result['fallback']), ('sccache', '1', '1'))

    def test_cdpath_cannot_redirect_a_relative_script_invocation(self):
        code, result = self.invoke(False, with_cdpath=True)
        self.assertEqual(code, 0)
        self.assertEqual(result['cwd'], str(ROOT))

    def test_release_builds_use_the_cache_and_respect_the_explicit_opt_out(self):
        for wrapper, expected in [(None, 'sccache'), ('', '')]:
            with self.subTest(wrapper=wrapper):
                code, result = self.invoke(True, explicit_wrapper=wrapper, release=True)
                self.assertEqual(code, 0)
                self.assertEqual(result['args'], ['build', '--release', '--locked', '-p', 'open-infra',
                                                 '--target-dir', 'target met spaties'])
                self.assertEqual(result['wrapper'], expected)

    def test_an_explicit_wrapper_or_opt_out_is_preserved(self):
        for wrapper in ['', '/opt/compiler-wrapper']:
            with self.subTest(wrapper=wrapper):
                code, result = self.invoke(True, explicit_wrapper=wrapper)
                self.assertEqual(code, 0)
                self.assertEqual(result['wrapper'], wrapper)
                self.assertIsNone(result['client'])

    def test_an_existing_cache_mode_is_preserved(self):
        code, result = self.invoke(True, client='0', fallback='0')
        self.assertEqual(code, 0)
        self.assertEqual((result['client'], result['fallback']), ('0', '0'))

    def test_a_cargo_failure_remains_a_failure(self):
        code, _ = self.invoke(True, exit_code=42)
        self.assertEqual(code, 42)


if __name__ == '__main__':
    unittest.main()
