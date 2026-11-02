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
        self.assertRegex(steps[cache], r'uses: Swatinem/rust-cache@[0-9a-f]{40}\b')
        self.assertRegex(steps[cache], r'(?m)^\s+shared-key: release\s*$')
        self.assertNotRegex(steps[cache], r'(cache-workspace-crates|cache-on-failure):\s*[\x22\x27]?true')
        cargo_steps = [i for i, step in enumerate(steps) if re.search(r'\bcargo (test|build)\b', step)]
        self.assertEqual(len(cargo_steps), 3, 'behoud tests en de twee afzonderlijke pluginbuilds')
        for i in cargo_steps:
            self.assertLess(cache, i)


if __name__ == '__main__':
    unittest.main()
