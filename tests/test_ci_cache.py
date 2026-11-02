"""Bewaar dependencyhergebruik vóór elke Rust-compilatiestap in de test-CI."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]


def jobs(workflow):
    # Alleen jobgrenzen; dit is geen algemene YAML-parser. actionlint controleert
    # de YAML en Actions-expressies afzonderlijk.
    body = workflow.split('\njobs:\n', 1)[1]
    starts = list(re.finditer(r'^  ([a-zA-Z_][\w-]*):\s*$', body, re.MULTILINE))
    return {m[1]: body[m.end():starts[i + 1].start() if i + 1 < len(starts) else len(body)]
            for i, m in enumerate(starts)}


class CiDependencyCacheTest(unittest.TestCase):
    def test_rust_jobs_restore_dependencies_after_selecting_the_toolchain(self):
        workflow = (ROOT / '.github/workflows/test.yml').read_text(encoding='utf-8')
        rust_jobs = {name: job for name, job in jobs(workflow).items()
                     if re.search(r'\bcargo (test|check|build)\b', job)}
        self.assertEqual(set(rust_jobs), {'offline', 'online'})
        for name, job in rust_jobs.items():
            with self.subTest(job=name):
                steps = re.split(r'^      - ', job, flags=re.MULTILINE)[1:]
                toolchain = [i for i, step in enumerate(steps) if 'uses: dtolnay/rust-toolchain@' in step]
                caches = [i for i, step in enumerate(steps) if 'uses: Swatinem/rust-cache@' in step]
                self.assertEqual(len(caches), 1, 'iedere Rust-job moet dependencies hergebruiken')
                self.assertEqual(len(toolchain), 1)
                cache = caches[0]
                self.assertLess(toolchain[0], cache, 'de sleutel moet de gekozen rustc bevatten')
                self.assertRegex(steps[cache], r'uses: Swatinem/rust-cache@[0-9a-f]{40}\b')
                self.assertRegex(steps[cache], r'(?m)^\s+shared-key: test\s*$')
                self.assertNotRegex(steps[cache], r'(cache-workspace-crates|cache-on-failure):\s*[\x22\x27]?true')
                for i, step in enumerate(steps):
                    if re.search(r'\bcargo (test|check|build)\b', step):
                        self.assertLess(cache, i, 'herstellen moet vóór compileren gebeuren')


if __name__ == '__main__':
    unittest.main()
