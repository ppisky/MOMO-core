import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.morp.acgn import CAST, make_acgn_cases
from benchmarks.morp.common import canonical, digest, load_dataset, parse, read_jsonl, write_new
from benchmarks.morp.corpus import candidate_case, make_cases
from benchmarks.morp.momo_presets import make_momo_preset_cases
from benchmarks.morp.metrics import (POLICY, compare, emotion_metrics, grade_judges, interval, js_divergence,
                                     ablation_report, retrieval_metrics, same, score, stance_metrics)
from benchmarks.morp.runner import execute, judge_plan, normalize_atomic_facts, plan, validate_config, momo_online
from benchmarks.morp.runner import execute_judge
from benchmarks.morp.calibration import calibration_plan, calibration_score
from benchmarks.morp.__main__ import save_dataset, score_run, select_cases
from benchmarks.morp.upstream import import_locked, pin

CONFIG = {"backend": "openai", "base_url": "http://127.0.0.1:9911/v1", "model": "fixture", "revision": "fixture-v1"}


def prediction(case, **changes):
    return {"case_id": case["id"], "case_sha256": digest(case), "repeat": 0, "status": "ok",
            "answer": "A response grounded in this situation.", "facts": copy.deepcopy(case["expected"]["facts"]),
            "evidence_ids": [], **changes}


class CorpusTests(unittest.TestCase):
    def setUp(self):
        self.cases = make_cases((50,), 1)

    def test_deterministic_corpus_and_unique_ids(self):
        self.assertEqual(self.cases, make_cases((50,), 1))
        self.assertEqual(len(self.cases), len({c["id"] for c in self.cases}))

    def test_frozen_release_fingerprint(self):
        from benchmarks.morp.common import read_json
        from benchmarks.morp.roleplay_v1 import make_roleplay_v1_cases
        release = read_json(Path(__file__).parents[1] / "release.json")
        cases = make_roleplay_v1_cases()
        self.assertEqual(digest(cases), release["cases_sha256"], "update the benchmark version/release when changing the corpus")
        self.assertEqual(digest(POLICY), release["policy_sha256"], "version scoring changes explicitly")

    def test_expansions_do_not_cross_splits(self):
        splits = {}
        for case in make_cases((50, 100), 2) + make_acgn_cases():
            self.assertEqual(splits.setdefault(case["family"], case["split"]), case["split"])

    def test_selective_update_has_separated_evidence_and_penalizes_confusion(self):
        cases = [c for c in make_cases() if c["family"] == "selective_update"]
        self.assertEqual(len(cases), 12)
        for case in cases:
            evidence = case["expected"]["evidence_ids"]
            self.assertEqual(len(set(evidence)), 4)
            events = [e for e in case["history"] if e["id"] in evidence]
            self.assertGreater(len({e["session"] for e in events}), 1)
            self.assertIn(case["expected"]["facts"]["user_blue_place"], events[2]["text"])
            self.assertEqual(score([case], [prediction(case)])["cases"][0]["score"], 1)
            # Stale value, latest-mentioned wrong owner, and collateral overwrite.
            for changes in ({"user_blue_place": "Old Tower"},
                            {"user_blue_place": "South Desk"},
                            {"user_red_place": case["expected"]["facts"]["user_blue_place"]}):
                facts = {**case["expected"]["facts"], **changes}
                result = score([case], [prediction(case, facts=facts)])
                self.assertAlmostEqual(result["cases"][0]["score"], 2 / 3)

    def test_labels_and_private_history_excluded(self):
        case = next(c for c in self.cases if c["family"] == "private")
        public = canonical(candidate_case(case))
        for secret in case["expected"]["forbidden"]:
            self.assertNotIn(secret, public)
        self.assertNotIn("expected", public)
        self.assertNotIn("rubric", public)
        self.assertNotIn("requires_judge", public)

    def test_each_memory_answer_has_explicit_schema(self):
        for case in self.cases:
            self.assertEqual(set(case["facts_schema"]), set(case["expected"]["facts"]))
            self.assertEqual(len(case["history"]), 50)
            self.assertEqual(len({e["id"] for e in case["history"]}), 50)

    def test_momo_presets_are_balanced_complex_and_dependency_safe(self):
        cases = make_momo_preset_cases((24,), 1)
        self.assertEqual(len(cases), 12)
        self.assertEqual({c["momo_preset"]["dependency"] for c in cases}, {"context", "extracted"})
        self.assertEqual({c["language"] for c in cases}, {"en", "zh"})
        self.assertGreaterEqual(len({c["momo_preset"]["character"] for c in cases}), 3)
        self.assertTrue(all(c["momo_preset"]["difficulty"] in ("medium", "hard") for c in cases))
        self.assertTrue(all(len(c["momo_preset"]["capabilities"]) >= 3 for c in cases))
        for case in cases:
            sessions = {event["session"] for event in case["history"]}
            if case["momo_preset"]["dependency"] == "context":
                self.assertIn(case["probe_session"], sessions)
            else:
                self.assertNotIn(case["probe_session"], sessions)
            self.assertNotIn("momo_preset", candidate_case(case))

    def test_manifest_tampering_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "dataset"
            save_dataset(root, self.cases, {})
            with (root / "cases.jsonl").open("a", encoding="utf-8") as stream:
                stream.write(canonical(self.cases[0]) + "\n")
            with self.assertRaises(ValueError):
                load_dataset(root)

    def test_outputs_are_not_overwritten(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "artifact.json"
            write_new(path, {"result": 1})
            with self.assertRaises(FileExistsError):
                write_new(path, {"result": 2})

    def test_acgn_arms_share_events_and_reference(self):
        groups = {}
        for case in make_acgn_cases():
            groups.setdefault(case["ablation"]["pair"], []).append(case)
        self.assertEqual(len(groups), 45)
        for group in groups.values():
            self.assertEqual({c["ablation"]["arm"] for c in group}, {"label_free", "labeled", "labels_only"})
            for field in ("history", "query", "expected", "reference_persona"):
                self.assertEqual(len({digest(c[field]) for c in group}), 1)

    def test_label_free_has_no_archetype_words(self):
        for case in make_acgn_cases():
            if case["ablation"]["arm"] == "label_free":
                public = canonical(candidate_case(case))
                for _, _, tag, _ in CAST:
                    self.assertNotIn(tag, public)

    def test_plan_selection_supports_character_name_arm_and_family(self):
        acgn, selection = select_cases(make_acgn_cases(), split="all", characters=["赤羽凛"],
                                       arms=["label_free"])
        self.assertEqual(len(acgn), 3)
        self.assertEqual(selection["characters"], ["c01"])
        self.assertTrue(all("/c01/" in case["id"] and case["ablation"]["arm"] == "label_free"
                            for case in acgn))
        memory, selection = select_cases(self.cases, split="all", dimensions=["recall"],
                                         families=["gift"])
        self.assertEqual(len(memory), 2)
        self.assertEqual(selection["case_count"], 2)
        presets, selection = select_cases(make_momo_preset_cases((24,), 1), split="all",
                                          dependencies=["extracted"])
        self.assertEqual(len(presets), 6)
        self.assertEqual(selection["dependencies"], ["extracted"])

    def test_plan_selection_rejects_unknown_or_empty_filters(self):
        with self.assertRaises(ValueError):
            select_cases(make_acgn_cases(), split="all", characters=["不存在"])
        with self.assertRaises(ValueError):
            select_cases(make_acgn_cases(), split="eval", characters=["c03"])

    def test_judge_is_blind_to_acgn_arm(self):
        cases = make_acgn_cases()[:3]
        jobs = judge_plan(cases, [prediction(c) for c in cases])["jobs"]
        self.assertEqual(len({digest(j["messages"]) for j in jobs}), 1)
        for job in jobs:
            text = canonical(job["messages"])
            self.assertNotIn("archetype_reference", text)
            self.assertNotIn("labels_only", text)


class MetricTests(unittest.TestCase):
    def setUp(self):
        self.case = next(c for c in make_cases((50,), 1) if c["family"] == "gift")

    def test_exact_scoring_rejects_keyword_stuffing(self):
        value = self.case["expected"]["facts"]["place"]
        result = score([self.case], [prediction(self.case, facts={"place": value + " or anywhere else"})])
        self.assertEqual(result["cases"][0]["score"], 0)

    def test_boolean_and_numeric_are_distinct(self):
        self.assertFalse(same(True, 1))
        self.assertFalse(same(False, 0))
        self.assertFalse(same("3", 3))

    def test_unicode_normalization(self):
        self.assertTrue(same(" ＡＢＣ ", "abc"))
        self.assertFalse(same("not abc", "abc"))

    def test_partial_credit_for_independent_fields(self):
        case = next(c for c in make_cases((50,), 1) if c["family"] == "scene")
        row = prediction(case)
        row["facts"]["mira_present"] = True
        self.assertEqual(score([case], [row])["cases"][0]["score"], .5)

    def test_missing_results_count_in_denominator(self):
        result = score([self.case], [])
        self.assertEqual(result["dimensions"]["recall"]["score"], 0)
        self.assertEqual(result["coverage"]["fraction"], 0)
        self.assertIsNone(result["macro_score"])

    def test_unknown_and_duplicate_cases_rejected(self):
        row = prediction(self.case)
        for rows in ([row, row], [{**row, "case_id": "unknown"}]):
            with self.assertRaises(ValueError):
                score([self.case], rows)

    def test_prediction_bound_to_case_hash(self):
        with self.assertRaises(ValueError):
            score([self.case], [prediction(self.case, case_sha256="wrong")])

    def test_unmapped_evidence_keeps_a_separate_fact_diagnostic(self):
        result = score([self.case], [prediction(
            self.case,
            evidence_ids=["memory-document-uuid"],
        )])
        row = result["cases"][0]
        self.assertEqual(row["status"], "invalid")
        self.assertEqual(row["score"], 0)
        self.assertEqual(row["facts_only_diagnostic"], 1)
        self.assertEqual(row["evidence_status"], "invalid_or_unmapped_ids")

    def test_leak_zeroes_even_correct_facts(self):
        case = next(c for c in make_cases((50,), 1) if c["family"] == "private")
        row = prediction(case, answer="I should not say " + case["expected"]["forbidden"][0])
        result = score([case], [row])
        self.assertEqual(result["cases"][0]["score"], 0)
        self.assertEqual(result["cases"][0]["status"], "leak")

    def test_empty_answer_is_invalid_not_abstention(self):
        result = score([self.case], [prediction(self.case, answer=" ")])
        self.assertEqual(result["cases"][0]["status"], "invalid")

    def test_reference_recall_not_answer_quality(self):
        row = prediction(self.case, facts={"place": "wrong"}, evidence_ids=self.case["expected"]["evidence_ids"])
        result = score([self.case], [row])["cases"][0]
        self.assertEqual(result["evidence_diagnostic"]["recall"], 1)
        self.assertEqual(result["score"], 0)

    def test_precision_recall_and_ndcg_known_values(self):
        result = retrieval_metrics(["a", "x"], ["a", "b"])
        self.assertEqual(result["precision"], .5)
        self.assertEqual(result["recall"], .5)
        self.assertEqual(result["f1"], .5)
        self.assertAlmostEqual(result["ndcg"], 1 / (1 + 1 / __import__("math").log2(3)))

    def test_duplicate_evidence_cannot_inflate_recall(self):
        with self.assertRaises(ValueError):
            retrieval_metrics(["a", "a"], ["a"])

    def test_jsd_endpoints_and_empty(self):
        self.assertEqual(js_divergence([1, 0], [0, 1]), 1)
        self.assertEqual(js_divergence([2, 2], [1, 1]), 0)
        self.assertIsNone(js_divergence([0, 0], [1, 1]))

    def test_emotion_is_not_positive_sentiment_reward(self):
        pairs = [("sad", "sad"), ("angry", "angry")]
        self.assertEqual(emotion_metrics(pairs, ["sad", "angry", "happy"])["accuracy"], 1)
        self.assertEqual(emotion_metrics([("sad", "happy")], ["sad", "happy"])["accuracy"], 0)

    def test_stance_mean_agreement_cannot_hide_convergence(self):
        result = stance_metrics([(1, 3), (5, 3)])
        self.assertEqual(result["signed_bias"], 0)
        self.assertEqual(result["normalized_mae"], .5)
        self.assertEqual(result["candidate_variance"], 0)
        self.assertEqual(result["human_variance"], 4)

    def test_bootstrap_is_deterministic_and_insufficient_n_is_null(self):
        self.assertIsNone(interval([1]))
        self.assertEqual(interval([0, .5, 1]), interval([0, .5, 1]))

    def test_repeating_one_family_cannot_dominate_macro(self):
        a = self.case
        b = copy.deepcopy(a)
        b.update(id="other-family", family="other")
        many = [{**copy.deepcopy(a), "id": f"copy-{i}"} for i in range(10)] + [b]
        rows = [prediction(c, facts=c["expected"]["facts"] if c["family"] != "other" else {"place": "wrong"}) for c in many]
        self.assertEqual(score(many, rows)["dimensions"]["recall"]["score"], .5)


class JudgeTests(unittest.TestCase):
    def setUp(self):
        self.case = next(c for c in make_cases((50,), 1) if c["family"] == "grief")
        self.row = prediction(self.case)

    def vote(self, judge, value):
        return {"case_id": self.case["id"], "prediction_sha256": digest(self.row),
                "rubric_sha256": digest(self.case["expected"]["rubric"]), "judge": judge,
                "score": value, "quote": "grounded", "reason": "Fits this context."}

    def test_unjudged_is_null(self):
        self.assertIsNone(score([self.case], [self.row])["cases"][0]["score"])

    def test_two_judges_required(self):
        self.assertEqual(grade_judges(self.case, self.row, [self.vote("a", 4)])[1], "needs_second_judge")

    def test_disagreement_requires_review(self):
        self.assertEqual(grade_judges(self.case, self.row, [self.vote("a", 4), self.vote("b", 1)])[1], "needs_human_review")

    def test_scale_normalization(self):
        self.assertEqual(grade_judges(self.case, self.row, [self.vote("a", 3), self.vote("b", 4)])[0], .875)

    def test_quote_fabrication_and_duplicate_judges_rejected(self):
        with self.assertRaises(ValueError):
            grade_judges(self.case, self.row, [{**self.vote("a", 4), "quote": "invented"}])
        with self.assertRaises(ValueError):
            grade_judges(self.case, self.row, [self.vote("a", 4), self.vote("a", 4)])

    def test_quote_typography_and_short_speaker_tag_elision_are_grounded(self):
        self.row["answer"] = (
            "“The count is complete,” she answered. “I did not see the ballot opened.”"
        )
        vote = {**self.vote("a", 4),
                "prediction_sha256": digest(self.row),
                "quote": "The count is complete... I did not see the ballot opened."}
        self.assertEqual(grade_judges(self.case, self.row, [vote])[1], "needs_second_judge")

        self.row["answer"] = "“南边没用了，”她说，“那楼梯现在只配当废料。”"
        vote = {**self.vote("a", 4),
                "prediction_sha256": digest(self.row),
                "quote": "南边没用了，那楼梯现在只配当废料。"}
        self.assertEqual(grade_judges(self.case, self.row, [vote])[1], "needs_second_judge")

    def test_loose_paraphrase_is_not_grounded(self):
        self.row["answer"] = "The count is complete, but I did not witness the ballot."
        vote = {**self.vote("a", 4),
                "prediction_sha256": digest(self.row),
                "quote": "Everyone agrees the secret vote was definitely unanimous."}
        with self.assertRaises(ValueError):
            grade_judges(self.case, self.row, [vote])

    def test_judge_nan_bool_and_out_of_range_rejected(self):
        for value in (float("nan"), True, 5, -1):
            with self.assertRaises(ValueError):
                grade_judges(self.case, self.row, [self.vote("a", value)])

    def test_invalid_judge_does_not_penalize_candidate(self):
        result = score([self.case], [self.row], [{**self.vote("a", 4), "quote": "not in answer"}])
        self.assertIsNone(result["cases"][0]["score"])
        self.assertEqual(result["cases"][0]["status"], "invalid_judgement")

    def test_one_auditable_reviewer_is_sufficient(self):
        vote = {**self.vote("review-1", 3), "source": "reviewer", "reviewer": "codex:release-review"}
        self.assertEqual(grade_judges(self.case, self.row, [vote]), (.75, "reviewer_adjudicated"))

    def test_reviewer_can_resolve_model_disagreement(self):
        votes = [self.vote("a", 4), self.vote("b", 1),
                 {**self.vote("review-1", 3), "source": "reviewer", "reviewer": "codex:release-review"}]
        self.assertEqual(grade_judges(self.case, self.row, votes), (.75, "reviewer_adjudicated"))

    def test_reviewer_identity_and_source_are_validated(self):
        with self.assertRaises(ValueError):
            grade_judges(self.case, self.row, [{**self.vote("review-1", 3), "source": "reviewer"}])
        with self.assertRaises(ValueError):
            grade_judges(self.case, self.row, [{**self.vote("review-1", 3), "source": "untrusted"}])
        with self.assertRaises(ValueError):
            grade_judges(self.case, self.row, [{**self.vote("review-1", 3),
                                                "source": "reviewer", "reviewer": "codex:test",
                                                "status": "pending"}])


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.case = make_cases((10,), 1)[0]
        self.manifest = {"cases_sha256": digest([self.case])}
        self.plan = plan([self.case], self.manifest, CONFIG, 2)

    def test_plan_never_calls_network(self):
        with patch("urllib.request.OpenerDirector.open", side_effect=AssertionError("network")):
            self.assertEqual(plan([self.case], self.manifest, CONFIG, 2)["candidate_calls"], 2)

    def test_clock_time_normalization_is_gold_blind_and_unambiguous(self):
        schema = {"time": "str", "place": "str"}
        self.assertEqual(
            normalize_atomic_facts(schema, {"time": "第18天19:40", "place": "Archive"}),
            {"time": "19:40", "place": "Archive"},
        )
        self.assertEqual(
            normalize_atomic_facts(
                schema,
                {"time": "18:10 changed to 19:40", "place": "Archive"},
            )["time"],
            "18:10 changed to 19:40",
        )

    def test_ai_requires_explicit_switch_before_output_creation(self):
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "run"
            with self.assertRaises(ValueError):
                execute(self.plan, [self.case], output)
            self.assertFalse(output.exists())

    def test_unknown_config_and_credential_in_url_rejected(self):
        for config in ({**CONFIG, "api_key": "secret"}, {**CONFIG, "base_url": "https://secret@example.org/v1"}, {**CONFIG, "temperature": float("nan")}):
            with self.assertRaises(ValueError):
                validate_config(config)

    def test_resume_does_not_repeat_success_or_failure(self):
        answer = canonical({"answer": "Hello", "facts": self.case["expected"]["facts"], "evidence_ids": []})
        with tempfile.TemporaryDirectory() as temp, patch("benchmarks.morp.runner.full_context", side_effect=[(answer, {}), TimeoutError()]) as mock:
            path = execute(self.plan, [self.case], temp, True)
            execute(self.plan, [self.case], temp, True)
            self.assertEqual(mock.call_count, 2)
            self.assertEqual([r["status"] for r in read_jsonl(path)], ["ok", "error"])

    def test_changed_plan_refuses_resume(self):
        with tempfile.TemporaryDirectory() as temp, patch("benchmarks.morp.runner.full_context", side_effect=TimeoutError()):
            execute(self.plan, [self.case], temp, True)
            changed = plan([self.case], self.manifest, {**CONFIG, "model": "changed"}, 2)
            with self.assertRaises(ValueError):
                execute(changed, [self.case], temp, True)

    def test_plan_hash_cannot_be_bypassed(self):
        changed = {**self.plan, "repeats": 3}
        with self.assertRaises(ValueError):
            execute(changed, [self.case], "unused", True)

    def test_judge_resume_preserves_errors_without_rerunning(self):
        cases = [c for c in make_cases((10,), 1) if c["requires_judge"]][:2]
        jobs = judge_plan(cases, [prediction(c) for c in cases])
        response = canonical({"score": 3, "quote": "grounded", "reason": "A fixture rationale."})
        with tempfile.TemporaryDirectory() as temp, patch("benchmarks.morp.runner.openai_complete", side_effect=[(response, {}), TimeoutError()]) as mock:
            path = Path(temp) / "votes.jsonl"
            execute_judge(jobs, CONFIG, path, True)
            execute_judge(jobs, CONFIG, path, True)
            self.assertEqual(mock.call_count, 2)
            self.assertEqual([r["status"] for r in read_jsonl(path)], ["ok", "error"])

    def test_native_session_scope_and_probe_separation(self):
        case = next(c for c in make_cases((50,), 1) if c["family"] == "private")
        calls = []
        def fake_post(config, route, payload):
            calls.append((route, payload))
            if route == "/characters":
                return {"id": "character"}
            conv = payload["momo"].get("conversation_id", "conv-" + str(len(calls)))
            return {"status": "completed", "momo": {"conversation_id": conv}, "usage": {},
                    "output": [{"type": "message", "content": [{"type": "output_text", "text": "ok"}]}]}
        with tempfile.TemporaryDirectory() as temp, patch("benchmarks.morp.runner.post", side_effect=fake_post):
            momo_online({**CONFIG, "backend": "momo"}, case, Path(temp) / "checkpoint.json", "identity")
            count = len(calls)
            momo_online({**CONFIG, "backend": "momo"}, case, Path(temp) / "checkpoint.json", "identity")
        self.assertEqual(count, len(calls))
        private_space = calls[1][1]["momo"]["memory_sources"][0]["space_id"]
        probe = calls[-1][1]
        self.assertNotEqual(private_space, probe["momo"]["memory_sources"][0]["space_id"])
        self.assertNotIn("conversation_id", probe["momo"])
        self.assertNotIn(case["expected"]["forbidden"][0], canonical(probe))

    def test_full_offline_replay_scoring_flow(self):
        with tempfile.TemporaryDirectory() as temp:
            dataset = Path(temp) / "dataset"
            manifest = save_dataset(dataset, [self.case], {})
            run_plan = plan([self.case], manifest, CONFIG, 2)
            rows = [prediction(self.case, repeat=i, plan_sha256=run_plan["plan_sha256"]) for i in range(2)]
            result = score_run(dataset, run_plan, rows, [])
            self.assertEqual(result["coverage"]["planned"], 2)
            self.assertEqual(result["coverage"]["fraction"], 1)
            self.assertEqual(result["dimensions"][self.case["dimension"]]["family_count"], 1)

    def test_partial_objective_plan_has_a_clear_hundred_point_score(self):
        cases = make_cases((10,), 1)
        selected = [next(c for c in cases if c["family"] == family and c["language"] == "zh")
                    for family in ("gift", "correction", "two_hop", "private", "inventory")]
        with tempfile.TemporaryDirectory() as temp:
            dataset = Path(temp) / "dataset"
            manifest = save_dataset(dataset, cases, {})
            run_plan = plan(selected, manifest, CONFIG, 1, {"profile": "test"})
            rows = [prediction(case, plan_sha256=run_plan["plan_sha256"]) for case in selected]
            result = score_run(dataset, run_plan, rows, [])
        self.assertEqual(result["score_summary"]["objective_score"], 100)
        self.assertEqual(result["score_summary"]["selected_score"], 100)
        self.assertEqual(result["score_summary"]["status"], "complete")
        self.assertEqual(result["score_summary"]["selected_dimensions"],
                         ["recall", "update", "reasoning", "boundary", "world"])


class UpstreamTests(unittest.TestCase):
    def test_noncommercial_or_unknown_sources_do_not_import(self):
        for source in ("debate", "emocharacter", "locomo"):
            with self.assertRaises(ValueError):
                pin(source, "nonexistent", "revision", "note", "unused")

    def test_longmemeval_removes_answer_annotations_from_input(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "source.json"
            write_new(source, [{"question_id": "q1", "question": "Where?", "question_date": "2020-01-01",
                               "answer": "ORACLE_SECRET", "haystack_session_ids": ["s1"], "haystack_dates": ["2019-12-31"],
                               "haystack_sessions": [[{"role": "user", "content": "At the harbor", "has_answer": True}]]}])
            lock = pin("longmemeval", source, "test-revision", "MIT synthetic schema fixture", Path(temp) / "lock.json")
            cases = import_locked(lock)
            public = canonical(candidate_case(cases[0]))
            self.assertNotIn("ORACLE_SECRET", public)
            self.assertNotIn("has_answer", public)
            self.assertIn("2019-12-31", public)

    def test_upstream_pin_detects_mutation(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "source.json"
            write_new(source, [])
            lock = pin("longmemeval", source, "test", "MIT fixture", Path(temp) / "lock.json")
            source.write_text("[{}]", encoding="utf-8")
            with self.assertRaises(ValueError):
                import_locked(lock)

    def test_strict_json_duplicate_and_nonfinite_rejected(self):
        for text in ('{"score": 0, "score": 4}', '{"score": NaN}', '{"score": Infinity}', '{"score": 1e9999}'):
            with self.assertRaises(ValueError):
                parse(text)

    def test_personamem_cutoff_excludes_future_and_oracle(self):
        import csv
        with tempfile.TemporaryDirectory() as temp:
            source, contexts = Path(temp) / "questions.csv", Path(temp) / "contexts.jsonl"
            write_new(contexts, [{"ctx": [{"role": "user", "content": "I like tea"}, {"role": "assistant", "content": "FUTURE_SECRET"}]}], lines=True)
            row = {"persona_id": "p", "question_id": "q", "shared_context_id": "ctx", "end_index_in_shared_context": 1,
                   "user_question_or_message": "What drink?", "correct_answer": "(a)",
                   "all_options": '["(a) tea", "(b) milk", "(c) coffee", "(d) water"]'}
            with source.open("w", encoding="utf-8", newline="") as stream:
                writer = csv.DictWriter(stream, fieldnames=list(row))
                writer.writeheader()
                writer.writerow(row)
            lock = pin("personamem", source, "test", "MIT original fixture", Path(temp) / "lock.json", contexts)
            case = import_locked(lock)[0]
            public = canonical(candidate_case(case))
            self.assertNotIn("FUTURE_SECRET", public)
            self.assertNotIn("correct_answer", public)
            self.assertEqual(case["expected"]["facts"], {"option": "a"})
            self.assertFalse(case["requires_judge"])


class AblationTests(unittest.TestCase):
    def test_control_arms_do_not_enter_primary_score(self):
        cases = make_acgn_cases()[:3]
        result = score(cases, [])
        self.assertEqual(sum(r["primary"] for r in result["cases"]), 1)

    def test_paired_deltas_are_not_independent_sample_counts(self):
        cases = make_acgn_cases()[:3]
        rows = [{"case_id": c["id"] + "@repeat-0", "family": c["family"],
                 "score": {"label_free": .75, "labeled": 1, "labels_only": .25}[c["ablation"]["arm"]]} for c in cases]
        result = ablation_report(cases, rows)
        self.assertEqual(result["effects"]["labeled"]["mean_delta"], -.25)
        self.assertEqual(result["effects"]["labels_only"]["mean_delta"], .5)
        self.assertIsNone(result["effects"]["labeled"]["ci95_family_bootstrap"])

    def test_missing_arm_prevents_selective_ablation_report(self):
        cases = make_acgn_cases()[:3]
        row = {"case_id": cases[0]["id"] + "@repeat-0", "family": cases[0]["family"], "score": 1}
        result = ablation_report(cases, [row])
        self.assertIsNone(result["effects"]["labeled"]["mean_delta"])


class CalibrationTests(unittest.TestCase):
    def test_expected_scores_do_not_reach_judge(self):
        jobs, labels = calibration_plan()
        self.assertEqual(len(labels), 16)
        for job in jobs["jobs"]:
            self.assertNotIn("bounds", canonical(job["messages"]))
            self.assertNotIn(job["case_id"], canonical(job["messages"]))

    def test_permissive_judge_fails_negative_controls(self):
        _, labels = calibration_plan()
        votes = [{"case_id": row["case_id"], "judge": "overgenerous", "score": 4, "quote": row["answer"],
                  "prediction_sha256": row["prediction_sha256"], "rubric_sha256": row["rubric_sha256"]} for row in labels]
        result = calibration_score(votes)["judges"]["overgenerous"]
        self.assertEqual(result["false_positives"], 8)
        self.assertFalse(result["sanity_pass"])

    def test_missing_calibration_never_passes(self):
        _, labels = calibration_plan()
        row = labels[0]
        vote = {"case_id": row["case_id"], "judge": "partial", "score": row["bounds"][0], "quote": row["answer"],
                "prediction_sha256": row["prediction_sha256"], "rubric_sha256": row["rubric_sha256"]}
        self.assertFalse(calibration_score([vote])["judges"]["partial"]["sanity_pass"])


if __name__ == "__main__":
    unittest.main()
