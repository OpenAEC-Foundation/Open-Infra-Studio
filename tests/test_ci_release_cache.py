"""Behoud dependencyhergebruik voor iedere native releasecompilatie."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ReleaseDependencyCacheTest(unittest.TestCase):
    def test_restore_follows_the_pinned_toolchain_and_precedes_all_cargo_steps(self):
        workflow = (ROOT / '.github/workflows/release.yml').read_text(encoding='utf-8')
        steps = re.split(r'^      - ', workflow, flags=re.MULTILINE)[1:]
        toolchains = [i for i, step in enumerate(steps) if 'uses: dtolnay/rust-toolchain@' in step]
        caches = [i for i, step in enumerate(steps) if 'uses: Swatinem/rust-cache@' in step]
        self.assertEqual(len(toolchains), 1)
        self.assertEqual(len(caches), 1, 'alle native matrixjobs moeten dependencies kunnen herstellen')
        cache = caches[0]
        self.assertLess(toolchains[0], cache)
        self.assertNotRegex(steps[cache], r'(?m)^\s+if:')
        self.assertNotRegex(steps[cache], r'cache-targets:\s*[\x22\x27]?false')
        self.assertRegex(steps[cache], r'uses: Swatinem/rust-cache@[0-9a-f]{40}\b')
        self.assertRegex(steps[cache], r'(?m)^\s+shared-key: release\s*$')
        self.assertNotRegex(steps[cache], r'(cache-workspace-crates|cache-on-failure):\s*[\x22\x27]?true')
        cargo_steps = [i for i, step in enumerate(steps) if re.search(r'\bcargo (test|build)\b', step)]
        self.assertEqual(len(cargo_steps), 3, 'behoud tests en de twee afzonderlijke pluginbuilds')
        for i in cargo_steps:
            self.assertLess(cache, i)


    def test_default_branch_prepares_the_cache_for_future_tags(self):
        path = ROOT / '.github/workflows/native-cache.yml'
        self.assertTrue(path.exists(), 'nieuwe tags kunnen caches van andere tags niet lezen')
        workflow = path.read_text(encoding='utf-8')
        self.assertRegex(workflow, r'(?m)^  push:\n    branches: \[main\]$')
        self.assertNotRegex(workflow, r'(?m)^  (pull_request|pull_request_target|schedule):')
        self.assertNotRegex(workflow, r'(?m)^    tags:')
        for path in ['Cargo.toml', 'Cargo.lock', 'civiel/Cargo.toml',
                     '.github/workflows/release.yml', '.github/workflows/native-cache.yml']:
            self.assertIn(f'      - "{path}"', workflow)
        self.assertIn('permissions:\n  contents: read', workflow)
        self.assertNotRegex(workflow, r'secrets\.|action-gh-release|signing-action|notarytool|codesign')
        release = (ROOT / '.github/workflows/release.yml').read_text(encoding='utf-8')
        versions = [re.findall(r'^  RUST_TOOLCHAIN: "([^"]+)"$', text, re.MULTILINE)
                    for text in [workflow, release]]
        self.assertEqual(len(versions[0]), 1)
        self.assertEqual(versions[0], versions[1])
        self.assertEqual(set(re.findall(r'os: ([\w.-]+)', workflow)),
                         set(re.findall(r'os: ([\w.-]+)', release)))
        self.assertEqual(re.findall(r'^      - run: (cargo .+)$', workflow, re.MULTILINE),
                         re.findall(r'^      - run: (cargo .+)$', release, re.MULTILINE))
        steps = re.split(r'^      - ', workflow, flags=re.MULTILINE)[1:]
        cache = [i for i, step in enumerate(steps) if 'uses: Swatinem/rust-cache@' in step]
        self.assertEqual(len(cache), 1)
        self.assertNotRegex(steps[cache[0]], r'(?m)^\s+if:')
        self.assertNotRegex(steps[cache[0]], r'(cache-targets|save-if):\s*[\x22\x27]?false')
        self.assertIn('id: cache', steps[cache[0]])
        self.assertRegex(steps[cache[0]], r'Swatinem/rust-cache@[0-9a-f]{40}\b')
        self.assertRegex(steps[cache[0]], r'(?m)^\s+shared-key: release\s*$')
        self.assertNotRegex(steps[cache[0]], r'(cache-workspace-crates|cache-on-failure):\s*[\x22\x27]?true')
        toolchain = next(i for i, step in enumerate(steps) if 'dtolnay/rust-toolchain@' in step)
        self.assertLess(toolchain, cache[0])
        for i, step in enumerate(steps):
            if re.search(r'\bcargo (test|build)\b', step):
                self.assertLess(cache[0], i)
                self.assertIn("if: steps.cache.outputs.cache-hit != 'true'", step)


if __name__ == '__main__':
    unittest.main()
