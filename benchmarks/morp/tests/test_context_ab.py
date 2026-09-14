import unittest

from benchmarks.morp.context_ab import CASES, blind_bundle, make_plan


class ContextAbTests(unittest.TestCase):
    def test_frozen_cases_have_twelve_complete_rounds(self):
        self.assertEqual(len(CASES), 8)
        self.assertEqual(sum(case["suite"] == "morp" for case in CASES), 4)
        self.assertEqual(sum(case["suite"] == "acgn" for case in CASES), 4)
        self.assertTrue(all(len(case["turns"]) == 12 for case in CASES))
        self.assertTrue(all(set(turn) == {"user", "assistant"} for case in CASES for turn in case["turns"]))

    def test_plan_is_single_model_and_embedding_is_separately_priced(self):
        plan = make_plan()
        self.assertEqual(plan["model"], "qwen3.8-flash")
        self.assertEqual(plan["candidate_calls"], 16)
        self.assertEqual(plan["maintenance_calls"], 16)
        self.assertEqual(plan["suite_counts"], {"morp": 4, "acgn": 4})
        self.assertIn("bge_m3_usd_per_million", plan["pricing"])

    def test_plan_can_freeze_a_focused_case_without_changing_the_case(self):
        selected = CASES[3]
        plan = make_plan([selected])
        self.assertEqual(plan["cases"], [selected])
        self.assertEqual(plan["candidate_calls"], 2)
        self.assertEqual(plan["maintenance_calls"], 2)

    def test_blind_bundle_is_deterministic_and_computes_probe_saving(self):
        cases = []
        for case in CASES:
            cases.append({"case_id": case["id"], "suite": case["suite"], "title": case["title"],
                          "dimension": case["dimension"],
                          "persona": case["persona"], "query": case["query"], "rubric": case["rubric"],
                          "direct_context": {"answer": "direct", "usage": {"prompt_tokens": 100}},
                          "momo_12_turn": {"answer": "momo", "usage": {"input_tokens": 40}}})
        results = {"results_sha256": "x", "model": "qwen3.8-flash", "thresholds": {}, "cases": cases}
        left, right = blind_bundle(results), blind_bundle(results)
        self.assertEqual(left, right)
        self.assertEqual(left["observed"]["probe_input_saving_percent"], 60.0)


if __name__ == "__main__":
    unittest.main()
