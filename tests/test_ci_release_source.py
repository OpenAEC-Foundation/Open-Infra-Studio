"""Controleer de releasebron met echte, tijdelijke git-repositories."""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / '.github/workflows/release.yml'


class ReleaseSourceTest(unittest.TestCase):
    def test_release_needs_a_tag_and_the_checked_commit(self):
        workflow = WORKFLOW.read_text(encoding='utf-8')
        self.assertNotIn('workflow_dispatch:', workflow)
        self.assertNotIn('inputs.tag', workflow)
        self.assertIn("if: github.event_name == 'push' && github.ref_type == 'tag'", workflow)
        self.assertIn('  build:\n    needs: source\n', workflow)
        self.assertIn('ref: ${{ needs.source.outputs.commit }}', workflow)
        self.assertFalse((ROOT / '.github/workflows/ecosystem-macos-signing.yml').exists())
        for action in re.findall(r'uses: ([^\s]+)', workflow):
            self.assertRegex(action, r'@[0-9a-f]{40}$')

    def check_commit(self, merged):
        workflow = WORKFLOW.read_text(encoding='utf-8')
        match = re.search(r'      - name: Require a release commit from main\n.*?        run: \|\n(.*?)(?=\n  build:)', workflow, re.S)
        self.assertIsNotNone(match)
        script = '\n'.join(line[10:] for line in match.group(1).splitlines())
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            remote = root / 'origin.git'
            repo = root / 'work'
            subprocess.run(['git', 'init', '--bare', '-q', str(remote)], check=True)
            subprocess.run(['git', 'init', '-q', '-b', 'main', str(repo)], check=True)
            def git(*args):
                return subprocess.run(['git', '-C', str(repo), *args], check=True,
                                      capture_output=True, text=True, env=dict(os.environ,
                                          GIT_AUTHOR_NAME='Release source test',
                                          GIT_AUTHOR_EMAIL='test@example.invalid',
                                          GIT_COMMITTER_NAME='Release source test',
                                          GIT_COMMITTER_EMAIL='test@example.invalid')).stdout.strip()
            git('remote', 'add', 'origin', str(remote))
            git('commit', '--allow-empty', '-m', 'main')
            git('push', '-q', 'origin', 'main')
            git('switch', '-q', '-c', 'feature')
            git('commit', '--allow-empty', '-m', 'release candidate')
            commit = git('rev-parse', 'HEAD')
            if merged:
                git('push', '-q', 'origin', 'HEAD:main')
            git('switch', '-q', '--detach', commit)
            output = root / 'output'
            env = dict(os.environ, GITHUB_OUTPUT=str(output))
            result = subprocess.run(['bash', '-c', script], cwd=repo, env=env,
                                    capture_output=True, text=True)
            return result, output.read_text() if output.exists() else '', commit

    def test_merged_detached_release_commit_is_allowed(self):
        result, output, commit = self.check_commit(True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output, f'commit={commit}\n')

    def test_unmerged_detached_release_commit_is_rejected(self):
        result, output, _ = self.check_commit(False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Release tags must point to a commit already merged into main', result.stdout)
        self.assertEqual(output, '')


if __name__ == '__main__':
    unittest.main()
