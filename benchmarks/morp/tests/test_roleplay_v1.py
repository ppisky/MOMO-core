import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from benchmarks.morp.__main__ import save_dataset, score_run
from benchmarks.morp.common import canonical, digest, load_dataset
from benchmarks.morp.corpus import ROLEPLAY_DIMENSIONS, candidate_case
from benchmarks.morp.metrics import score
from benchmarks.morp.roleplay_v1 import make_roleplay_v1_cases
from benchmarks.morp.runner import (
    ROLEPLAY_OUTPUT_CONTRACT, full_context, judge_plan, momo_roleplay,
    parse_roleplay_output, plan, review_template,
)


CONFIG = {"backend": "momo", "base_url": "http://127.0.0.1:9911/v1", "model": "fixture",
          "revision": "fixture-v1", "temperature": 0, "max_output_tokens": 512,
          "timeout_seconds": 30, "context_window": 8192,
          "memory": False, "semantic_graph": False, "mo_state": False}


class RoleplayV1Tests(unittest.TestCase):
    def setUp(self):
        self.cases = make_roleplay_v1_cases()

    def test_balanced_roleplay_only_dataset(self):
        self.assertEqual(len(self.cases), 64)
        self.assertEqual(len({case["family"] for case in self.cases}), 16)
        self.assertEqual({case["dimension"] for case in self.cases}, set(ROLEPLAY_DIMENSIONS))
        for dimension in ROLEPLAY_DIMENSIONS:
            self.assertEqual(len({case["family"] for case in self.cases
                                  if case["dimension"] == dimension}), 2)
        self.assertTrue(all(case["evaluation_mode"] == "roleplay" for case in self.cases))
        self.assertTrue(all(case["requires_judge"] for case in self.cases))
        self.assertTrue(all(not case["facts_schema"] and not case["expected"]["facts"]
                            for case in self.cases))
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "data"
            manifest = save_dataset(root, self.cases, {}, "1.0.0")
            self.assertEqual(manifest["version"], "1.0.0")
            _, loaded = load_dataset(root)
        self.assertEqual(loaded, self.cases)

    def test_counterfactual_pairs_change_only_final_beat(self):
        for family in {case["family"] for case in self.cases}:
            for language in ("en", "zh"):
                left, right = [case for case in self.cases
                               if case["family"] == family and case["language"] == language]
                self.assertEqual(left["history"][:-1], right["history"][:-1])
                self.assertNotEqual(left["history"][-1], right["history"][-1])
                self.assertEqual(left["query"], right["query"])
                self.assertEqual(left["persona"], right["persona"])

    def test_candidate_contains_script_but_not_judge_material(self):
        public = candidate_case(self.cases[0])
        serialized = canonical(public)
        self.assertEqual(public["evaluation_mode"], "roleplay")
        self.assertTrue(all(event["role"] in ("user", "assistant") for event in public["history"]))
        self.assertNotIn("expected", serialized)
        self.assertNotIn("rubric", serialized)
        self.assertNotIn("challenge", serialized)

    def test_roleplay_plan_counts_one_generation_per_case(self):
        manifest = {"cases_sha256": digest(self.cases)}
        run_plan = plan(self.cases, manifest, CONFIG, 2)
        self.assertEqual(run_plan["candidate_calls"], 128)
        self.assertEqual(run_plan["protocol"], "momo-roleplay-sessions/1")

    def test_roleplay_score_is_entirely_judged(self):
        case = self.cases[0]
        prediction = {"case_id": case["id"], "case_sha256": digest(case), "status": "ok",
                      "answer": "A scene-native turn.", "facts": {}, "evidence_ids": []}
        row = score([case], [prediction])["cases"][0]
        self.assertNotIn("facts_only_diagnostic", row)
        self.assertIsNone(row["score"])
        self.assertEqual(row["status"], "unjudged")

    def test_roleplay_output_prefers_plain_prose_and_accepts_legacy_envelope(self):
        self.assertEqual(parse_roleplay_output("  *She closes the ledger.*  ")["answer"],
                         "*She closes the ledger.*")
        self.assertEqual(parse_roleplay_output('{"answer":"She stays."}')["answer"],
                         "She stays.")
        with self.assertRaises(ValueError):
            parse_roleplay_output("   ")

    def _scored_report(self, cases):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "data"
            manifest = save_dataset(root, cases, {}, "1.0.0")
            run_plan = plan(cases, manifest, CONFIG, 1)
            predictions = []
            votes = []
            for case in cases:
                prediction = {
                    "case_id": case["id"], "case_sha256": digest(case), "repeat": 0,
                    "plan_sha256": run_plan["plan_sha256"], "status": "ok",
                    "answer": "A scene-native turn.", "facts": {}, "evidence_ids": [],
                }
                predictions.append(prediction)
                votes.append({
                    "case_id": case["id"], "repeat": 0,
                    "prediction_sha256": digest(prediction),
                    "rubric_sha256": digest(case["expected"]["rubric"]),
                    "judge": "reviewer", "source": "reviewer",
                    "reviewer": "codex:test", "score": 3, "quote": "scene-native",
                    "reason": "The turn satisfies the scene-specific rubric.",
                })
            return score_run(root, run_plan, predictions, votes)

    def test_review_template_binds_one_reviewer_to_frozen_jobs(self):
        case = self.cases[0]
        prediction = {"case_id": case["id"], "case_sha256": digest(case),
                      "repeat": 0, "status": "ok", "answer": "A scene-native turn."}
        jobs = judge_plan([case], [prediction])
        rows = review_template(jobs, "codex:rc2")
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["source"], "reviewer")
        self.assertEqual(rows[0]["reviewer"], "codex:rc2")
        self.assertEqual(rows[0]["status"], "pending")
        self.assertIsNone(rows[0]["score"])

    def test_complete_report_exposes_roleplay_score(self):
        report = self._scored_report(self.cases)
        self.assertEqual(report["macro_score"], 0.75)
        self.assertEqual(report["score_summary"]["roleplay_score"], 75.0)
        self.assertEqual(report["score_summary"]["status"], "complete")

    def test_partial_dimension_is_not_a_full_roleplay_score(self):
        cases = [case for case in self.cases if case["dimension"] == "agency"]
        report = self._scored_report(cases)
        self.assertIsNone(report["macro_score"])
        self.assertIsNone(report["score_summary"]["roleplay_score"])
        self.assertEqual(report["score_summary"]["selected_score"], 75.0)
        self.assertEqual(report["score_summary"]["status"], "partial_roleplay")

    def test_missing_execution_never_publishes_a_roleplay_score(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "data"
            manifest = save_dataset(root, self.cases, {}, "1.0.0")
            run_plan = plan(self.cases, manifest, CONFIG, 1)
            report = score_run(root, run_plan, [], [])
        self.assertIsNone(report["macro_score"])
        self.assertIsNone(report["score_summary"]["roleplay_score"])
        self.assertEqual(report["score_summary"]["status"], "incomplete_execution")

    @patch("benchmarks.morp.runner.openai_complete")
    def test_full_context_replays_authored_roles(self, complete):
        complete.return_value = ('{"answer":"ok"}', {})
        case = self.cases[0]
        full_context({**CONFIG, "backend": "openai"}, case)
        messages = complete.call_args.args[1]
        self.assertEqual([message["role"] for message in messages[1:-1]],
                         [event["role"] for event in case["history"]])
        self.assertEqual(messages[-1], {"role": "user", "content": case["query"]})

    @patch("benchmarks.morp.runner.post")
    def test_native_protocol_stages_script_without_memory_or_placeholder_turns(self, post):
        calls = []
        def fake(config, path, payload):
            calls.append((path, payload))
            if path == "/characters":
                return {"id": "character"}
            if path == "/conversations":
                return {"id": "conversation"}
            if path == "/messages":
                return {"id": "message"}
            if path == "/momo/responses":
                return {"status": "completed", "output": [{"type": "message", "content": [
                        {"type": "output_text", "text": '{"answer":"performed"}'}]}],
                        "usage": {}, "momo": {}}
            raise AssertionError(path)
        post.side_effect = fake
        with tempfile.TemporaryDirectory() as temp:
            answer, _ = momo_roleplay(CONFIG, self.cases[0], Path(temp) / "checkpoint.json", "identity")
        self.assertEqual(answer, '{"answer":"performed"}')
        staged = [payload for path, payload in calls if path == "/messages"]
        self.assertEqual([payload["role"] for payload in staged],
                         [event["role"] for event in self.cases[0]["history"]])
        probe = next(payload for path, payload in calls if path == "/momo/responses")
        self.assertEqual(probe["instructions"], ROLEPLAY_OUTPUT_CONTRACT)
        self.assertEqual(probe["momo"]["memory_sources"], [])
        self.assertFalse(probe["momo"]["mo_state"])


if __name__ == "__main__":
    unittest.main()
