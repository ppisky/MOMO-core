import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.morp.common import canonical, digest, read_json, read_jsonl
from benchmarks.morp.corpus import make_cases
from benchmarks.morp.momo_presets import make_momo_preset_cases
from benchmarks.morp.runner import momo_online, validate_config, execute
from benchmarks.morp.scenarios import (create, run, comparison, causal_comparison,
                                      SCENARIOS, CORE_SCENARIOS, CAUSAL_SCENARIOS)
from benchmarks.morp.performance import report
from benchmarks.morp.__main__ import main, save_dataset, score_run

CONFIG = {"backend": "momo", "base_url": "http://127.0.0.1:9911/v1", "model": "fixture",
          "revision": "fixture-v1", "pricing": {"currency": "USD", "input_per_million": 2,
                                                   "output_per_million": 8}}


class ScenarioTests(unittest.TestCase):
    def setUp(self):
        self.case = next(c for c in make_cases((10,), 1) if c["family"] == "private")
        self.manifest = {"cases_sha256": digest([self.case])}

    def test_plan_cli_offline_and_flags(self):
        with tempfile.TemporaryDirectory() as temp, patch("benchmarks.morp.runner.post", side_effect=AssertionError("network")):
            root = Path(temp)
            save_dataset(root / "data", [self.case], {})
            (root / "config.json").write_text(canonical(CONFIG), encoding="utf-8")
            main(["scenario-plan", str(root / "data"), "--config", str(root / "config.json"),
                  "--split", "all", "--repeats", "1", "--out", str(root / "plans")])
            plans = [read_json(root / "plans" / f"{name}.plan.json") for name in SCENARIOS]
            self.assertEqual(plans[0]["requests"], plans[1]["requests"])
            self.assertEqual(plans[0]["protocol"], plans[1]["protocol"])
            self.assertFalse(plans[0]["config"]["memory"])
            self.assertTrue(plans[1]["config"]["mo_state"])
            with self.assertRaises(ValueError):
                run(root / "plans", self.manifest, [self.case], root / "runs")
            self.assertFalse((root / "runs").exists())

    def test_same_probe_no_private_data_and_maintenance_barrier(self):
        calls = []
        def fake_post(config, route, payload):
            calls.append((config["scenario"], route, payload))
            if route == "/characters":
                return {"id": "card"}
            if route.endswith("/drain"):
                return {"completed": True}
            return {"status": "completed", "momo": {"conversation_id": "conv"},
                    "usage": {"input_tokens": 10, "output_tokens": 2},
                    "output": [{"type": "message", "content": [{"type": "output_text", "text": "ok"}]}]}
        with tempfile.TemporaryDirectory() as temp, patch("benchmarks.morp.runner.post", side_effect=fake_post):
            for scenario in SCENARIOS:
                enabled = scenario == "all_enabled"
                config = {**CONFIG, "scenario": scenario, "memory": enabled, "semantic_graph": enabled, "mo_state": enabled}
                path = Path(temp) / f"{scenario}.json"
                _, usage = momo_online(config, self.case, path, scenario)
                self.assertEqual(len(usage["timings"]), 11)
                count = len(calls)
                momo_online(config, self.case, path, scenario)
                self.assertEqual(count, len(calls), "resume must not bill twice")
        probes = [p for _, route, p in calls if route == "/momo/responses" and p["input"].startswith("[probe]")]
        self.assertEqual(probes[0]["input"], probes[1]["input"])
        self.assertNotIn(self.case["expected"]["forbidden"][0], probes[0]["input"])
        self.assertEqual(probes[0]["momo"]["memory_sources"], [])
        self.assertNotIn("memory_write_space_id", probes[0]["momo"])
        self.assertNotEqual(probes[0]["momo"]["personal_space_id"], probes[1]["momo"]["personal_space_id"])
        all_calls = [(route, p) for scenario, route, p in calls if scenario == "all_enabled"]
        self.assertTrue(all_calls[-3][0].endswith("/drain"))
        self.assertTrue(all_calls[-1][0].endswith("/drain"))
        self.assertFalse(any(route.endswith("/drain") for s, route, _ in calls if s == "basic_context"))

    def test_dependency_presets_do_not_replay_history_and_use_the_intended_session(self):
        presets = make_momo_preset_cases((12,), 1)
        context = next(c for c in presets if c["momo_preset"]["dependency"] == "context")
        extracted = next(c for c in presets if c["momo_preset"]["dependency"] == "extracted")
        calls = []

        def fake_post(config, route, payload):
            calls.append((route, payload))
            if route == "/characters":
                return {"id": "card"}
            if route.endswith("/drain"):
                return {"completed": True}
            return {"status": "completed", "momo": {"conversation_id": payload["momo"].get("conversation_id", "new-conv")},
                    "usage": {}, "output": [{"type": "message", "content": [{"type": "output_text", "text": "ok"}]}]}

        with tempfile.TemporaryDirectory() as temp, patch("benchmarks.morp.runner.post", side_effect=fake_post):
            for case in (context, extracted):
                momo_online({**CONFIG, "scenario": "basic_context", "memory": False,
                             "semantic_graph": False, "mo_state": False}, case,
                            Path(temp) / f"{case['momo_preset']['dependency']}.json", case["id"])

        probes = [payload for route, payload in calls
                  if route == "/momo/responses" and payload["input"].startswith("[probe]")]
        self.assertEqual(len(probes), 2)
        self.assertNotIn('"history"', probes[0]["input"])
        self.assertNotIn('"history"', probes[1]["input"])
        self.assertIn("conversation_id", probes[0]["momo"])
        self.assertNotIn("conversation_id", probes[1]["momo"])

    def test_scenario_run_counterbalances_order(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            create([self.case], self.manifest, CONFIG, 2, root / "plans")
            with patch("benchmarks.morp.scenarios.execute") as execute_mock:
                run(root / "plans", self.manifest, [self.case], root / "runs", True)
            self.assertEqual([c.args[0]["config"]["scenario"] for c in execute_mock.call_args_list],
                             ["basic_context", "all_enabled", "all_enabled", "basic_context"])

    def test_causal_matrix_changes_one_component_group_at_a_time(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            create([self.case], self.manifest, CONFIG, 1, root / "plans", matrix="causal")
            experiment = read_json(root / "plans/experiment.json")
            self.assertEqual(tuple(experiment["arms"]), CAUSAL_SCENARIOS)
            for name, expected in {
                "basic_context": (False, False, False), "dmw_only": (True, False, False),
                "nsg_only": (False, True, False), "dmw_state": (True, False, True),
                "nsg_state": (False, True, True), "dmw_nsg": (True, True, False),
                "all_enabled": (True, True, True),
            }.items():
                config = read_json(root / "plans" / f"{name}.plan.json")["config"]
                self.assertEqual(tuple(config[k] for k in ("memory", "semantic_graph", "mo_state")), expected)

    def test_core_matrix_answers_memory_then_state_question(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            create([self.case], self.manifest, CONFIG, 1, root / "plans", matrix="core")
            experiment = read_json(root / "plans/experiment.json")
            self.assertEqual(tuple(experiment["arms"]), CORE_SCENARIOS)

    def test_failed_barrier_never_runs_probe(self):
        inputs = []
        def fake_post(config, route, payload):
            if route == "/characters":
                return {"id": "card"}
            if route.endswith("/drain"):
                return {"completed": False}
            inputs.append(payload["input"])
            return {"status": "completed", "momo": {"conversation_id": "conv"}, "usage": {},
                    "output": [{"type": "message", "content": [{"type": "output_text", "text": "ok"}]}]}
        with tempfile.TemporaryDirectory() as temp, patch("benchmarks.morp.runner.post", side_effect=fake_post):
            with self.assertRaises(ValueError):
                momo_online({**CONFIG, "scenario": "all_enabled", "memory": True,
                             "semantic_graph": True, "mo_state": True}, self.case,
                            Path(temp) / "checkpoint.json", "failed-barrier")
        self.assertEqual(len(inputs), len(self.case["history"]))
        self.assertFalse(any(value.startswith("[probe]") for value in inputs))

    def test_invalid_flags_and_rates_rejected(self):
        for extra in ({"scenario": "all_enabled", "memory": False},
                      {"thinking": "sometimes"},
                      {"history_mode": "synthetic_magic"},
                      {"pricing": {"currency": "USD", "input_per_million": -1, "output_per_million": 1}},
                      {"pricing": {"currency": "USD", "input_per_million": float("nan"), "output_per_million": 1}}):
            with self.assertRaises(ValueError):
                validate_config({**CONFIG, **extra})

    def test_recorded_history_uses_one_candidate_call_and_queues_real_maintenance(self):
        calls = []

        def fake_post(config, route, payload):
            calls.append((route, payload))
            if route == "/characters":
                return {"id": "card"}
            if route == "/conversations":
                return {"id": f"conversation-{len(calls)}"}
            if route in ("/messages", "/momo/maintenance/turns"):
                return {"recorded": True}
            if route.endswith("/drain"):
                return {"completed": True}
            return {"status": "completed", "momo": {"conversation_id": "probe-conversation"},
                    "usage": {"input_tokens": 10, "output_tokens": 2},
                    "output": [{"type": "message", "content": [{"type": "output_text", "text": "{}"}]}]}

        config = {**CONFIG, "scenario": "all_enabled", "history_mode": "recorded",
                  "memory": True, "semantic_graph": True, "mo_state": True}
        with tempfile.TemporaryDirectory() as temp, patch("benchmarks.morp.runner.post", side_effect=fake_post):
            _, usage = momo_online(config, self.case, Path(temp) / "recorded.json", "recorded")
            planned = __import__("benchmarks.morp.runner", fromlist=["plan"]).plan(
                [self.case], self.manifest, config, 1)

        response_calls = [call for call in calls if call[0] == "/momo/responses"]
        maintenance_turns = [call for call in calls if call[0] == "/momo/maintenance/turns"]
        self.assertEqual(len(response_calls), 1)
        self.assertEqual(len(maintenance_turns), len(self.case["history"]))
        self.assertEqual(usage["native_responses"], 1)
        self.assertEqual(planned["candidate_calls"], 1)
        self.assertEqual(planned["protocol"], "momo-recorded-sessions/1")

    def test_usage_cost_and_missing_data_are_not_zero(self):
        row = {"case_id": "x", "repeat": 0, "status": "ok", "attempt_seconds": 5,
               "usage": {"per_response": [{"input_tokens": 1000, "output_tokens": 100}],
                         "timings": [{"event_id": "probe", "seconds": 2}], "drain_seconds": 1}}
        result = report([row], CONFIG, 1)
        self.assertAlmostEqual(result["cost"]["observed_response_estimate"], .0028)
        self.assertIsNone(result["cost"]["total_estimate"])
        self.assertFalse(result["cost"]["complete"])
        self.assertEqual(result["probe_seconds"]["p95"], 2)
        self.assertEqual(result["probe_seconds"]["p99"], 2)
        result = report([{**row, "usage": {}}], CONFIG, 1)
        self.assertIsNone(result["cost"]["observed_response_estimate"])
        self.assertEqual(result["tokens"]["calls_with_usage"], 0)
        row["usage"]["per_response"].append({})
        result = report([row], CONFIG, 1)
        self.assertAlmostEqual(result["cost"]["observed_response_estimate"], .0028)
        self.assertFalse(result["cases"][0]["response_usage_complete"])

    def test_malformed_answer_preserves_billed_usage(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            create([self.case], self.manifest, CONFIG, 1, root / "plans")
            p = read_json(root / "plans/basic_context.plan.json")
            with patch("benchmarks.morp.runner.momo_online", return_value=("invalid", {"input_tokens": 10, "output_tokens": 2})):
                output = execute(p, [self.case], root / "runs", True)
            row = read_jsonl(output)[0]
            self.assertEqual(row["status"], "error")
            self.assertEqual(row["usage"]["input_tokens"], 10)
            self.assertGreaterEqual(row["attempt_seconds"], 0)

    def test_reports_compare_quality_latency_and_cost(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            manifest = save_dataset(root / "data", [self.case], {})
            create([self.case], manifest, CONFIG, 1, root / "plans")
            reports = []
            for i, scenario in enumerate(SCENARIOS):
                p = read_json(root / "plans" / f"{scenario}.plan.json")
                row = {"case_id": self.case["id"], "case_sha256": digest(self.case), "repeat": 0,
                       "plan_sha256": p["plan_sha256"], "status": "ok", "answer": "Unknown",
                       "facts": self.case["expected"]["facts"], "evidence_ids": [],
                       "usage": {"per_response": [{"input_tokens": 100, "output_tokens": 10}],
                                 "timings": [{"event_id": "probe", "seconds": i + 1}]}}
                reports.append(score_run(root / "data", p, [row], []))
            result = comparison(*reports)
            self.assertEqual(result["quality"]["paired_family_macro_delta"], 0)
            self.assertEqual(result["performance"]["probe_seconds"]["family_mean_delta"], 1)
            bad = copy.deepcopy(reports[1])
            bad["experiment"]["config"]["model"] = "different"
            with self.assertRaises(ValueError):
                comparison(reports[0], bad)

    def test_dependency_slices_expose_separate_context_and_extracted_effects(self):
        presets = make_momo_preset_cases((12,), 1)
        cases = [
            next(c for c in presets if c["momo_preset"]["dependency"] == dependency
                 and not c["requires_judge"])
            for dependency in ("context", "extracted")
        ]
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            manifest = save_dataset(root / "data", cases, {})
            create(cases, manifest, CONFIG, 1, root / "plans")
            reports = []
            for scenario in SCENARIOS:
                run_plan = read_json(root / "plans" / f"{scenario}.plan.json")
                rows = []
                for case in cases:
                    facts = case["expected"]["facts"] if scenario == "all_enabled" else {
                        key: (not value if isinstance(value, bool) else "wrong")
                        for key, value in case["expected"]["facts"].items()
                    }
                    rows.append({
                        "case_id": case["id"], "case_sha256": digest(case), "repeat": 0,
                        "plan_sha256": run_plan["plan_sha256"], "status": "ok", "answer": "answer",
                        "facts": facts, "evidence_ids": [],
                        "usage": {"per_response": [{"input_tokens": 100, "output_tokens": 10}],
                                  "timings": [{"event_id": "probe", "seconds": 1}]},
                    })
                reports.append(score_run(root / "data", run_plan, rows, []))

            for dependency in ("context", "extracted"):
                self.assertEqual(reports[0]["slices"]["dependency"][dependency]["cases"], 1)
                self.assertEqual(reports[1]["slices"]["dependency"][dependency]["case_mean_diagnostic"], 1)
            result = comparison(*reports)
            self.assertEqual(result["schema"], "morp.scenario-comparison/2")
            self.assertEqual(set(result["quality_by_dependency"]), {"context", "extracted"})
            self.assertEqual(result["quality_by_dependency"]["context"]["family_mean_delta"], 1)
            self.assertEqual(result["quality_by_dependency"]["extracted"]["family_mean_delta"], 1)

    def test_causal_report_rejects_model_changes_and_names_component_contrasts(self):
        reports = {}
        for name in CAUSAL_SCENARIOS:
            memory, graph, state = __import__("benchmarks.morp.runner", fromlist=["MOMO_SCENARIO_FLAGS"]).MOMO_SCENARIO_FLAGS[name]
            reports[name] = {
                "policy_sha256": "policy", "macro_score": 1,
                "experiment": {"dataset_sha256": "data", "protocol": "recorded",
                               "config": {**CONFIG, "scenario": name, "memory": memory,
                                          "semantic_graph": graph, "mo_state": state}},
                "cases": [{"case_id": "case@repeat-0", "family": "family", "primary": True,
                           "score": 1}],
                "performance": {"probe_seconds": {"p95": 1}},
            }
        result = causal_comparison(reports)
        self.assertEqual(set(result["effects"]), {
            "dmw_vs_context", "nsg_vs_context", "dmw_nsg_vs_context",
            "state_given_dmw", "state_given_nsg", "state_given_dmw_nsg",
            "all_enabled_vs_context",
        })
        reports["nsg_only"]["experiment"]["config"]["model"] = "different"
        with self.assertRaisesRegex(ValueError, "changed model"):
            causal_comparison(reports)


if __name__ == "__main__":
    unittest.main()
