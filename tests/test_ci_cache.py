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
    def setUp(self):
        self.workflow = (ROOT / '.github/workflows/test.yml').read_text(encoding='utf-8')

    def rust_jobs(self):
        return {name: job for name, job in jobs(self.workflow).items()
                if re.search(r'\bcargo (test|check|build)\b', job)}

    def assert_job_restores_dependencies(self, name, job):
        steps = re.split(r'^      - ', job, flags=re.MULTILINE)[1:]
        toolchain = [i for i, step in enumerate(steps) if 'uses: dtolnay/rust-toolchain@' in step]
        caches = [i for i, step in enumerate(steps) if 'uses: Swatinem/rust-cache@' in step]
        self.assertEqual(len(caches), 1, f'{name}: iedere Rust-job moet dependencies hergebruiken')
        self.assertEqual(len(toolchain), 1)
        cache = caches[0]
        self.assertLess(toolchain[0], cache, 'de sleutel moet de gekozen rustc bevatten')
        self.assertNotRegex(steps[cache], r'(?m)^\s+if:',
                            f'{name}: de cachestap moet onvoorwaardelijk herstellen')
        self.assertNotRegex(steps[cache], r'(?m)^\s+cache-targets:\s*[\x22\x27]?false\b',
                            f'{name}: gecompileerde dependencies moeten worden bewaard')
        self.assertRegex(steps[cache], r'uses: Swatinem/rust-cache@[0-9a-f]{40}\b')
        self.assertRegex(steps[cache], r'(?m)^\s+shared-key: test\s*$')
        self.assertNotRegex(steps[cache], r'(cache-workspace-crates|cache-on-failure):\s*[\x22\x27]?true')
        for i, step in enumerate(steps):
            if re.search(r'\bcargo (test|check|build)\b', step):
                self.assertLess(cache, i, 'herstellen moet vóór compileren gebeuren')

    def test_rust_jobs_restore_dependencies_after_selecting_the_toolchain(self):
        rust_jobs = self.rust_jobs()
        self.assertEqual(set(rust_jobs), {'offline', 'online'})
        for name, job in rust_jobs.items():
            with self.subTest(job=name):
                self.assert_job_restores_dependencies(name, job)

    def test_disabled_cache_steps_are_rejected(self):
        for name, job in self.rust_jobs().items():
            for condition in ['false', '${{ false }}']:
                with self.subTest(job=name, condition=condition):
                    disabled, count = re.subn(
                        r'(      - uses: Swatinem/rust-cache@[^\n]+\n)',
                        lambda match: match[1] + f'        if: {condition}\n', job)
                    self.assertEqual(count, 1)
                    with self.assertRaises(AssertionError):
                        self.assert_job_restores_dependencies(name, disabled)

    def test_caches_without_compiled_targets_are_rejected(self):
        for name, job in self.rust_jobs().items():
            for value in ['false', '"false"', "'false'"]:
                with self.subTest(job=name, value=value):
                    registry_only = job.replace('          shared-key: test\n',
                                                f'          shared-key: test\n          cache-targets: {value}\n')
                    self.assertNotEqual(registry_only, job)
                    with self.assertRaises(AssertionError):
                        self.assert_job_restores_dependencies(name, registry_only)

    def test_explicitly_enabled_compiled_targets_are_accepted(self):
        for name, job in self.rust_jobs().items():
            for value in ['true', '"true"', "'true'"]:
                with self.subTest(job=name, value=value):
                    enabled = job.replace('          shared-key: test\n',
                                          f'          shared-key: test\n          cache-targets: {value}\n')
                    self.assertNotEqual(enabled, job)
                    self.assert_job_restores_dependencies(name, enabled)


if __name__ == '__main__':
    unittest.main()
