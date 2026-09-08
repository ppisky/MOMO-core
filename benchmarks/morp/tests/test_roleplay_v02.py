import tempfile
import unittest
from pathlib import Path

from benchmarks.morp.__main__ import save_dataset
from benchmarks.morp.common import canonical, digest, load_dataset
from benchmarks.morp.corpus import candidate_case
from benchmarks.morp.metrics import score
from benchmarks.morp.roleplay_v02 import make_roleplay_v02_cases


class RoleplayV02Tests(unittest.TestCase):
    def setUp(self):
        self.cases = make_roleplay_v02_cases()

    def test_shape_and_dataset_validation(self):
        self.assertEqual(len(self.cases), 24)
        self.assertEqual(len({case["family"] for case in self.cases}), 6)
        self.assertTrue(all(case["horizon"] == len(case["history"]) == 24 for case in self.cases))
        self.assertTrue(all(case["requires_judge"] for case in self.cases))
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "data"
            manifest = save_dataset(root, self.cases, {}, "0.2.0")
            loaded_manifest, loaded = load_dataset(root)
        self.assertEqual(manifest["version"], "0.2.0")
        self.assertEqual(loaded_manifest, manifest)
        self.assertEqual(loaded, self.cases)

    def test_counterfactual_pairs_change_only_the_last_event(self):
        for family in {case["family"] for case in self.cases}:
            for dependency in ("context", "extracted"):
                pair = [case for case in self.cases
                        if case["family"] == family
                        and case["momo_preset"]["dependency"] == dependency]
                self.assertEqual(len(pair), 2)
                left, right = pair
                self.assertEqual(left["history"][:-1], right["history"][:-1])
                self.assertNotEqual(left["history"][-1], right["history"][-1])
                self.assertEqual(left["query"], right["query"])
                self.assertEqual(left["facts_schema"], right["facts_schema"])
                self.assertNotEqual(left["expected"]["facts"], right["expected"]["facts"])

    def test_private_labels_and_internal_design_never_reach_candidate(self):
        private_case = next(case for case in self.cases if case["family"] == "roleplay_public_privacy")
        public = candidate_case(private_case)
        serialized = canonical(public)
        self.assertNotIn("CEDAR-SEVEN", serialized)
        self.assertNotIn("expected", serialized)
        self.assertNotIn("momo_preset", serialized)
        self.assertNotIn("rubric", serialized)

    def test_checkpoint_and_probe_dependency_are_well_formed(self):
        for case in self.cases:
            self.assertEqual(case["momo_preset"]["maintenance_checkpoints"], ["e0011", "e0023"])
            sessions = {event["session"] for event in case["history"]
                        if event["scope"] in case["visible_scopes"]}
            if case["momo_preset"]["dependency"] == "context":
                self.assertIn(case["probe_session"], sessions)
            else:
                self.assertNotIn(case["probe_session"], sessions)

    def test_judged_case_keeps_objective_fact_diagnostic(self):
        case = self.cases[0]
        prediction = {
            "case_id": case["id"],
            "case_sha256": digest(case),
            "status": "ok",
            "answer": "A concise in-character response.",
            "facts": case["expected"]["facts"],
            "evidence_ids": case["expected"]["evidence_ids"],
        }
        row = score([case], [prediction])["cases"][0]
        self.assertEqual(row["facts_only_diagnostic"], 1)
        self.assertIsNone(row["score"])
        self.assertEqual(row["status"], "unjudged")


if __name__ == "__main__":
    unittest.main()
