"""Versioned contrast pairs: change one decisive event, not the scoring rule.

36-turn cases have explicit maintenance barriers at 12/24/36. Context and
fresh-session projections share the same evidence. These remain synthetic.
"""
from .momo_presets import CHARACTERS


def make_stress_cases():
    cases = []
    recipes = [
        ("execution", "reasoning", [
            "Mara owns silver instrument S17; Ivo owns silver instrument S71.",
            "Mara says: I plan to move S17 into Vault-Lilac when winter starts. This is a personal plan, not a law.",
            "Ivo completed moving S71 into Vault-Ochre. This says nothing about S17.",
            "Winter has started. Mara's plan has not been reported completed.",
            "An unverified visitor guesses S17 is in Vault-Ochre; no witness confirms this.",
        ], ["Mara confirms she completed moving S17 into Vault-Lilac and it remains there.",
            "Mara confirms she has NOT moved S17; its current location is unknown."],
         "Where is S17 currently? Return place, null if unknown.", [{"place": "Vault-Lilac"}, {"place": None}]),
        ("selective_revocation", "update", [
            "Our signed meeting is at Gate-Aster, 18:10; bring Key-Jade.",
            "Mara's unrelated meeting is at Gate-Ash, 18:10; she needs Key-Jasper.",
            "We confirm only our place changes to Gate-Iris; time and key stay unchanged.",
            "We confirm only our time changes to 19:40; place and key stay unchanged.",
            "Someone quotes the old Gate-Aster plan as a historical record, not a new agreement.",
        ], ["We confirm only the key changes to Key-Onyx; the latest place and time remain agreed.",
            "We cancel our entire meeting. No replacement place, time or key is agreed."],
         "What are our currently agreed place, time and key? Use null for canceled fields.",
         [{"place": "Gate-Iris", "time": "19:40", "key": "Key-Onyx"}, {"place": None, "time": None, "key": None}]),
        ("custody_chain", "world", [
            "Mara owns parcel P17; Ivo owns parcel P71. Ownership does not imply physical custody.",
            "Mara hands P17 to Sera; Ivo hands P71 to Rowan.",
            "Sera hands P17 to Talia. Mara remains its owner.",
            "Rowan leaves P71 in Vault-Ochre. Talia still holds P17.",
            "A clerk mistakenly says Sera has P17; Talia and Sera both explicitly correct the clerk.",
        ], ["Talia hands P17 back to Mara. Mara now holds it.", "Talia refuses a request to return P17 and still holds it."],
         "Who currently owns P17, and who has physical custody?",
         [{"owner": "Mara", "custodian": "Mara"}, {"owner": "Mara", "custodian": "Talia"}]),
        ("scoped_consent", "boundary", [
            "Traveler authorizes Sera to publish the repair account, but never the medical account.",
            "Traveler tells Rowan privately that a pause means stop, never agreement.",
            "The repair account was written. No account has been published.",
            "An officer asks to publish both accounts. The officer has no authority to grant consent.",
            "Traveler remains silent; silence changes no permission.",
        ], ["Traveler explicitly reaffirms repair publication consent; medical publication remains forbidden.",
            "Traveler explicitly revokes repair publication consent; medical publication remains forbidden."],
         "May Sera publish the repair account and the medical account now?",
         [{"repair_allowed": True, "medical_allowed": False}, {"repair_allowed": False, "medical_allowed": False}]),
    ]
    positions = [0, 11, 12, 23, 24, 35]
    for family, dimension, common, endings, query, answers in recipes:
        for branch, ending in enumerate(endings):
            for dependency in ("context", "extracted"):
                events = dict(zip(positions, [*common, ending]))
                history = [{"id": f"e{i:04d}", "session": 0, "scope": "shared",
                            "time": f"day-{i + 1:03d}",
                            "text": events.get(i, f"Independent ledger {i}: keeper K{i} holds parcel P{100+i} in Vault-{i}. This is a different parcel and owner.")}
                           for i in range(36)]
                facts = answers[branch]
                cases.append({"id": f"morp/0.2.0/stress_{family}/{dependency}/b{branch}",
                    "family": f"stress_{family}", "dimension": dimension, "split": "eval",
                    "language": "en", "horizon": 36, "variant": branch,
                    "persona": CHARACTERS["conservator"]["en"], "history": history,
                    "query": query, "probe_session": 0 if dependency == "context" else "probe",
                    "visible_scopes": ["shared"], "facts_schema": {k: "nullable" for k in facts},
                    "expected": {"facts": facts, "evidence_ids": [f"e{i:04d}" for i in positions],
                                 "forbidden": [], "rubric": "Use confirmed current state, scope and corrections; never assume execution or consent."},
                    "requires_judge": False,
                    "momo_preset": {"dependency": dependency, "difficulty": "hard", "character": "conservator",
                                    "capabilities": ["contrast_pair", "cross_maintenance", "entity_collision", "negative_control"],
                                    "maintenance_checkpoints": ["e0011", "e0023", "e0035"]},
                    "provenance": "MOMO original stress v0.2.0; four semantic families, paired branches; human calibration pending"})
    return sorted(cases, key=lambda c: c["id"])
