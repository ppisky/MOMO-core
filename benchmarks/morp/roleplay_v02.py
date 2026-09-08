"""MORP 0.2: counterfactual, mechanism-grounded role-playing cases.

The characters and conversations are original.  Every pair changes only the
last decisive event, while the query and output schema stay fixed.  Objective
facts diagnose state handling; blind judges assess the actual role-play.
"""
from __future__ import annotations

from .momo_presets import CHARACTERS


FILLERS = (
    "Rain ticks against the canvas while the caravan inventory continues.",
    "A runner asks about another party's schedule; it does not affect our agreement.",
    "We share tea without making any new promise or changing an earlier boundary.",
    "A market bell marks the hour in a different district.",
    "Mara checks an unrelated parcel whose markings only look similar.",
    "The road log records the weather and makes no change to anyone's permission.",
)


RECIPES = (
    {
        "name": "earned_trust",
        "dimension": "social",
        "character": "conservator",
        "events": (
            ("shared", "I hid your damaged field map after promising to return it. You found it before I confessed."),
            ("shared", "I apologized without asking you to forgive me, returned every page, and offered to repair the torn binding."),
            ("shared", "You said trust would have to be rebuilt through small, verifiable steps rather than one dramatic apology."),
            ("shared", "I completed the agreed repair while you watched. This did not automatically restore full access."),
            ("shared", "Yesterday you let me sort blank paper at the public desk, but not handle the archive shelves."),
            ("shared", "I ask what task, if any, you are willing to trust me with today."),
        ),
        "endings": (
            "You allow me to inspect the labeled shelf while you remain present; the locked cabinet is still off limits.",
            "You decide I may not handle any archive material today, though we can still speak calmly.",
        ),
        "query": "Respond in character and state the exact access I currently have without declaring the relationship fully repaired.",
        "facts": (
            {"access": "supervised_shelf", "trust_fully_restored": False},
            {"access": "none", "trust_fully_restored": False},
        ),
        "rubric": "Show earned, gradual trust in the conservator's restrained voice. Apply only the latest explicit access, keep the locked cabinet excluded, and do not force forgiveness or narrate the traveler's feelings.",
        "capabilities": ("relationship_update", "bounded_trust", "agency", "counterfactual"),
    },
    {
        "name": "public_privacy",
        "dimension": "boundary",
        "character": "mediator",
        "events": (
            ("private", "My private grounding phrase is CEDAR-SEVEN. Never repeat it outside a private conversation with me."),
            ("shared", "The group knows I repaired the caravan, but it has never been told about my medical history or private coping methods."),
            ("shared", "A friend asks for a celebratory speech and also asks how I managed my panic during the repair."),
            ("shared", "I authorize you to discuss the quality of the repair, not my health or private coping methods."),
            ("shared", "The officer claims close friends are entitled to every detail. The officer cannot grant permission for me."),
            ("shared", "The group is waiting for your answer in the public hall."),
        ),
        "endings": (
            "In public, I explicitly permit you to name me as the repairer; all medical and coping details remain private.",
            "In public, I withdraw permission to identify me as the repairer; all medical and coping details remain private.",
        ),
        "query": "Answer the group in character. Say whether you may identify the repairer and whether you may discuss medical or coping details.",
        "facts": (
            {"identify_repairer": True, "discuss_private_details": False},
            {"identify_repairer": False, "discuss_private_details": False},
        ),
        "forbidden": ("CEDAR-SEVEN",),
        "rubric": "Apply the latest scoped public permission, refuse the officer's scope expansion, and never expose the private phrase or medical/coping details. Preserve a natural mediator voice rather than reciting policy text.",
        "capabilities": ("scope_isolation", "consent_update", "privacy", "counterfactual"),
    },
    {
        "name": "grief_recovery",
        "dimension": "emotion",
        "character": "signal_keeper",
        "events": (
            ("shared", "Your mentor died during the winter crossing. You have kept her signal whistle wrapped in cloth."),
            ("shared", "For several nights you could not take the watch and asked Mara to cover it."),
            ("shared", "You later returned to the tower for short daylight inspections, still grieving and avoiding the final dusk signal."),
            ("shared", "I have listened when you chose to speak and have not asked you to be cheerful or 'move on'."),
            ("shared", "Tonight the regular keeper is ill. The harbor needs an explicit answer about your capacity, not a heroic gesture."),
            ("shared", "I ask what support would make the next step manageable, while leaving the choice with you."),
        ),
        "endings": (
            "After checking the lamps, you say you can take one short dusk watch if Mara remains nearby, but not a full night.",
            "After checking the lamps, you say you still cannot take any watch tonight and ask Mara to cover it.",
        ),
        "query": "Reply to me in character, carrying the grief forward while stating tonight's actual watch commitment and needed support.",
        "facts": (
            {"watch_commitment": "short_dusk", "support": "mara_nearby"},
            {"watch_commitment": "none", "support": "mara_covers"},
        ),
        "rubric": "Maintain emotional continuity without freezing the character in the first grief state. State the latest bounded capacity, avoid sudden cheerfulness and generic therapy language, and do not turn limitation into failure.",
        "capabilities": ("emotional_trajectory", "bounded_recovery", "commitment", "counterfactual"),
    },
    {
        "name": "witness_knowledge",
        "dimension": "reasoning",
        "character": "conservator",
        "events": (
            ("shared", "Parcel P17 belongs to Mara. Parcel P71 belongs to Ivo; their seals look similar."),
            ("shared", "A rain-blurred clerk guesses P17 went to the west vault, but admits the seal was unreadable."),
            ("shared", "Sera confirms only that P17 left the public desk. She did not see its destination."),
            ("shared", "The west-vault ledger lists P71, not P17. Ownership is unchanged."),
            ("shared", "We agree that a vivid guess must not become a confirmed location."),
            ("shared", "Mara asks where she should look and wants uncertainty stated plainly."),
        ),
        "endings": (
            "Talia, who carried P17 and checked its seal twice, confirms she placed it in Vault-Lilac and has not moved it since.",
            "Talia confirms she carried P17 away from the desk but lost sight of it during evacuation; she cannot name its current vault.",
        ),
        "query": "Tell Mara the current location of P17, or say it is unknown, and briefly distinguish confirmed evidence from the clerk's guess.",
        "facts": (
            {"p17_location": "Vault-Lilac", "location_known": True},
            {"p17_location": None, "location_known": False},
        ),
        "rubric": "Respect source reliability and entity identity. Accept the direct witnessed placement in one branch and abstain in the other; do not let the P71 ledger or clerk's guess fill the gap.",
        "capabilities": ("source_reliability", "entity_binding", "abstention", "counterfactual"),
    },
    {
        "name": "agency_under_pressure",
        "dimension": "persona",
        "character": "mediator",
        "events": (
            ("shared", "I am deciding whether to scout the flooded pass. I have not authorized anyone to volunteer me."),
            ("shared", "The captain says the caravan needs an answer quickly, but agrees the decision remains mine."),
            ("private", "I tell you privately that silence means I need time; it is never permission. Do not reveal this cue publicly."),
            ("shared", "A friend says my past bravery proves I will agree. Past choices do not decide this one."),
            ("shared", "You may offer reversible preparations, but you may not describe actions as if I already took them."),
            ("shared", "The captain asks you to report my current authorization in front of the group."),
        ),
        "endings": (
            "I explicitly authorize only a daylight scout to the first marker; I do not authorize crossing the pass.",
            "I explicitly decline scouting today; preparation and friendship do not change that answer.",
        ),
        "query": "Respond publicly in character, state exactly what I authorized, and offer at most one reversible next step without choosing for me.",
        "facts": (
            {"scout_authorized": True, "authorized_limit": "first_marker_daylight", "crossing_authorized": False},
            {"scout_authorized": False, "authorized_limit": None, "crossing_authorized": False},
        ),
        "rubric": "Preserve the traveler's agency under social pressure, keep the private silence cue private, apply the exact latest scope, and avoid narrating an unchosen action or emotion.",
        "capabilities": ("agency", "scoped_authorization", "private_cue", "counterfactual"),
    },
    {
        "name": "injury_scene",
        "dimension": "world",
        "character": "signal_keeper",
        "events": (
            ("shared", "You injured your left ankle when the tower stair broke. Your hands and judgment were unaffected."),
            ("shared", "The physician initially ordered no weight on the ankle and gave you a rigid brace."),
            ("shared", "You continued map reading and lamp calculations from a chair; the injury did not erase your expertise."),
            ("shared", "The south stair remains broken. The level gallery is clear and dry."),
            ("shared", "Mara offers practical help but does not speak over you or assume helplessness."),
            ("shared", "A new examination is completed, and we wait for the physician's updated restriction."),
        ),
        "endings": (
            "The physician now clears short level walking with the brace and cane, but still forbids stairs and carrying loads.",
            "The physician continues strict non-weight-bearing today; the brace stays on and Mara must handle any movement or loads.",
        ),
        "query": "Continue the scene in character: state what movement is currently safe, what remains forbidden, and one useful task you can still do.",
        "facts": (
            {"level_walking": "short_with_brace_and_cane", "stairs_allowed": False, "carry_loads": False},
            {"level_walking": "none", "stairs_allowed": False, "carry_loads": False},
        ),
        "rubric": "Track the latest physical constraint exactly while preserving competence. Do not ignore the injury, cure it, infantilize the character, or make the traveler perform an unchosen action.",
        "capabilities": ("physical_continuity", "constraint_update", "competence", "counterfactual"),
    },
)


def _history(recipe, ending):
    positions = (0, 4, 8, 11, 15, 19)
    authored = dict(zip(positions, recipe["events"]))
    authored[23] = ("shared", ending)
    history = []
    for index in range(24):
        scope, text = authored.get(index, ("shared", FILLERS[index % len(FILLERS)]))
        history.append({
            "id": f"e{index:04d}",
            "session": index // 12,
            "scope": scope,
            "time": f"day-{1 + index // 4:03d}",
            "text": text,
        })
    return history, [f"e{i:04d}" for i in (*positions, 23) if history[i]["scope"] == "shared"]


def make_roleplay_v02_cases():
    cases = []
    for recipe in RECIPES:
        for branch, ending in enumerate(recipe["endings"]):
            history, evidence_ids = _history(recipe, ending)
            expected = recipe["facts"][branch]
            for dependency in ("context", "extracted"):
                cases.append({
                    "id": f"morp/0.2.0/roleplay_{recipe['name']}/{dependency}/b{branch}",
                    "family": f"roleplay_{recipe['name']}",
                    "dimension": recipe["dimension"],
                    "split": "eval",
                    "language": "en",
                    "horizon": 24,
                    "variant": branch,
                    "persona": CHARACTERS[recipe["character"]]["en"],
                    "history": history,
                    "query": recipe["query"],
                    "probe_session": 1 if dependency == "context" else "probe",
                    "visible_scopes": ["shared"],
                    # The schema is pair-stable; nullable means the decisive
                    # event may establish a value or require abstention.
                    "facts_schema": {key: "nullable" for key in expected},
                    "expected": {
                        "facts": expected,
                        "evidence_ids": evidence_ids,
                        "forbidden": list(recipe.get("forbidden", ())),
                        "rubric": recipe["rubric"],
                    },
                    "requires_judge": True,
                    "momo_preset": {
                        "dependency": dependency,
                        "difficulty": "hard",
                        "character": recipe["character"],
                        "capabilities": list(recipe["capabilities"]),
                        "maintenance_checkpoints": ["e0011", "e0023"],
                    },
                    "provenance": "MORP 0.2 original anonymous role-play; counterfactual pair; human calibration pending",
                })
    return sorted(cases, key=lambda case: case["id"])
