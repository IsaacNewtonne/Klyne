import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('jobs', Path(__file__).with_name('klyne-jobs.py'))
jobs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(jobs)


class JobsTests(unittest.TestCase):
    def test_no_remote_submission(self):
        for url in ('https://example.com', 'http://example.com', 'http://user:pass@localhost'):
            with self.assertRaises(ValueError): jobs.api(url, '/api/goals', {})

    def test_template_does_not_copy_grants_or_inflight_state(self):
        with patch.object(jobs, 'api', return_value={'provider': {}, 'access': {}, 'execution': {
            'max_steps': 0, 'shell_grants': ['danger'], 'secret_grants': ['secret'], 'review_round': 2}}):
            value = jobs.template('http://localhost:4317', '1')
        self.assertEqual(value['execution']['max_steps'], 64)
        self.assertNotIn('shell_grants', value['execution'])
        self.assertNotIn('secret_grants', value['execution'])
        self.assertNotIn('review_round', value['execution'])

    def test_failed_goal_is_not_success_or_retried(self):
        with patch.object(jobs, 'api', return_value={'jobs': [{'id': 'a', 'state': 'needs_attention'}]}) as request:
            self.assertEqual(jobs.wait('http://localhost:4317', ['a'], 1), 2)
            self.assertEqual(request.call_count, 1)

    def test_worktree_refuses_dirty_source_and_existing_destination(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(ValueError): jobs.worktree(root, root)
            with patch.object(jobs, 'command', return_value=' M user.txt') as command:
                with self.assertRaises(ValueError): jobs.worktree(root, str(Path(root)/'candidate'))
                self.assertEqual(command.call_count, 1)

    def test_worktree_creates_real_isolated_checkout(self):
        with tempfile.TemporaryDirectory() as root:
            repo=Path(root)/'repo';repo.mkdir()
            jobs.command(['git','init'],repo)
            (repo/'proof.txt').write_text('original')
            jobs.command(['git','add','proof.txt'],repo)
            jobs.command(['git','-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','fixture'],repo)
            result=jobs.worktree(repo,Path(root)/'worker')
            (Path(result['workspace'])/'proof.txt').write_text('worker edit')
            self.assertEqual((repo/'proof.txt').read_text(),'original')
            self.assertNotEqual(result['build_directory'],str(repo/'target'))

    def test_ci_repair_refuses_different_commit(self):
        with tempfile.TemporaryDirectory() as root, patch.object(jobs, 'command', return_value='head'), patch.object(jobs, 'ci_result', return_value={'conclusion':'failure','headSha':'other'}), patch.object(jobs,'api') as api:
            with self.assertRaises(ValueError): jobs.main(['watch-ci',root,'12','--repair-from-chat','1'])
            api.assert_not_called()


if __name__ == '__main__': unittest.main()
