import tempfile
import unittest
from urllib.error import HTTPError
from pathlib import Path
from unittest.mock import patch

from benchmarks.morp.stress import make_stress_cases
from benchmarks.morp.common import load_dataset, digest, read_jsonl
from benchmarks.morp.__main__ import save_dataset, score_run
from benchmarks.morp.runner import momo_online, plan, execute
from benchmarks.morp.metrics import score
from benchmarks.morp.performance import report


class StressTests(unittest.TestCase):
    def test_http_failure_records_status_without_remote_error_body(self):
        cases = make_stress_cases()[:1]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            manifest = save_dataset(root / 'data', cases, {})
            planned = plan(cases, manifest, {'backend': 'openai', 'base_url': 'http://127.0.0.1/v1',
                                            'model': 'fixture', 'revision': 'test'}, 1)
            error = HTTPError('http://127.0.0.1/v1', 504, 'REMOTE_PRIVATE_TEXT', None, None)
            with patch('benchmarks.morp.runner.full_context', side_effect=error):
                output = execute(planned, cases, root / 'run', allow_ai=True)
            row = read_jsonl(output)[0]
        self.assertEqual(row['http_status'], 504)
        self.assertNotIn('REMOTE_PRIVATE_TEXT', str(row))

    def test_missing_runs_are_penalized_without_claiming_completion(self):
        cases = make_stress_cases()[:2]
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp) / 'data'
            manifest = save_dataset(directory, cases, {})
            planned = plan(cases, manifest, {'backend': 'openai', 'base_url': 'http://127.0.0.1/v1',
                                            'model': 'fixture', 'revision': 'test'}, 1)
            result = score_run(directory, planned, [], [])
        self.assertEqual(result['score_summary']['status'], 'incomplete_execution')
        self.assertEqual(result['score_summary']['selected_score'], 0)
        self.assertIsNone(result['score_summary']['facts_only_diagnostic'])

    def test_always_unknown_cannot_pass_positive_negative_pair(self):
        cases = [c for c in make_stress_cases() if c['family'] == 'stress_execution'
                 and c['momo_preset']['dependency'] == 'extracted']
        predictions = [{'case_id': c['id'], 'case_sha256': digest(c), 'repeat': 0, 'status': 'ok',
                        'answer': 'Unknown', 'facts': {'place': None}, 'evidence_ids': []}
                       for c in cases]
        result = score(cases, predictions)
        self.assertEqual(result['dimensions']['reasoning']['score'], 0.5)

    def test_failed_run_preserves_cycle_wait_separately_from_reply_time(self):
        result = report([{'case_id': 'failed', 'repeat': 0, 'status': 'error',
                          'usage': {'per_response': [], 'cycle_barriers': {'e0011': 91}}}],
                        {'backend': 'momo'}, 1)
        self.assertEqual(result['cycle_barrier_seconds']['mean'], 91)
        self.assertIsNone(result['probe_seconds']['mean'])
        self.assertFalse(result['cost']['complete'])

    def test_pairs_change_only_decisive_event_without_schema_answer_leak(self):
        cases = make_stress_cases()
        self.assertEqual(len(cases), 16)
        self.assertEqual(len({c['family'] for c in cases}), 4)
        with tempfile.TemporaryDirectory() as tmp:
            save_dataset(Path(tmp) / 'data', cases, {})
            load_dataset(Path(tmp) / 'data')
        for family in {c['family'] for c in cases}:
            for dependency in ('context', 'extracted'):
                a, b = [c for c in cases if c['family'] == family and c['momo_preset']['dependency'] == dependency]
                self.assertEqual(a['facts_schema'], b['facts_schema'])
                self.assertEqual(a['query'], b['query'])
                self.assertEqual(a['history'][:-1], b['history'][:-1])
                self.assertNotEqual(a['expected']['facts'], b['expected']['facts'])

    def test_cycle_failure_blocks_next_turn_and_resume_does_not_reingest(self):
        case = make_stress_cases()[0]
        calls = []
        fail = True
        def post(config, route, payload):
            if route == '/characters':
                return {'id': 'card'}
            if route.endswith('/drain'):
                calls.append('drain')
                return {'completed': not fail}
            calls.append(payload['input'].split(']')[0] + ']')
            return {'status': 'completed', 'momo': {'conversation_id': 'conv'}, 'usage': {},
                    'output': [{'type': 'message', 'content': [{'type': 'output_text', 'text': 'ok'}]}]}
        config = {'scenario': 'all_enabled', 'model': 'fixture'}
        with tempfile.TemporaryDirectory() as tmp, patch('benchmarks.morp.runner.post', side_effect=post):
            checkpoint = Path(tmp) / 'checkpoint.json'
            with self.assertRaisesRegex(ValueError, 'cycle maintenance'):
                momo_online(config, case, checkpoint, 'stress')
            self.assertNotIn('[e0012]', calls)
            fail = False
            _, usage = momo_online(config, case, checkpoint, 'stress')
        self.assertEqual(calls.count('[e0000]'), 1)
        self.assertEqual(len(usage['cycle_barriers']), 3)
        self.assertLess(calls.index('drain'), calls.index('[e0012]'))


if __name__ == '__main__':
    unittest.main()
