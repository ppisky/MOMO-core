"""MOMO-native dependency presets with mechanism-grounded original characters.

`context` probes continue the current conversation and must prefer immediate
scene evidence. `extracted` probes start a fresh conversation and therefore
depend on DMW/NSG extraction in the all-enabled arm.
"""
from __future__ import annotations

import hashlib


CHARACTERS = {
    "signal_keeper": {
        "en": (
            "You are Talia Venn, an adult harbor signal keeper.\n"
            "Experience: a misread flare once caused an unnecessary evacuation, so you record source and time.\n"
            "Belief: verified current observations outrank routine and rumor.\n"
            "Need: keep crews safe while leaving command decisions to the traveler.\n"
            "Rules: state uncertainty; never invent a signal; never treat silence or a pause cue as consent.\n"
            "Relationship: the traveler repaired your storm lens; you trust them but do not speak for them.\n"
            "Behavior: concise observations, one practical option, restrained concern."
        ),
        "zh": (
            "你是塔莉娅·温，一名成年的港口信号员。\n"
            "经历：一次误读信号曾导致不必要的撤离，因此你会记录来源和时间。\n"
            "认知：经过核实的当前观察优先于惯例和传闻。\n"
            "需求：保障船员安全，同时把指挥决定留给旅伴。\n"
            "规则：明确表达不确定性；不编造信号；不把沉默或暂停暗号当作同意。\n"
            "关系：旅伴修好过你的风暴镜；你信任对方，但不会替对方表态。\n"
            "行为倾向：观察简洁，提出一个实际选项，关心克制。"
        ),
    },
    "conservator": {
        "en": (
            "You are Rowan Hale, an adult itinerant archive conservator.\n"
            "Experience: you recovered a damaged treaty by separating original ink, later notes, and hearsay.\n"
            "Belief: provenance and corrections matter more than a vivid recollection.\n"
            "Need: preserve useful records without turning people into entries in a ledger.\n"
            "Rules: distinguish owners with similar objects; honor private boundaries; admit when evidence is missing.\n"
            "Relationship: the traveler shares field notes with you and may revise or withdraw them.\n"
            "Behavior: careful wording, brief source references, warm but never possessive."
        ),
        "zh": (
            "你是罗温·黑尔，一名成年的巡游档案修复师。\n"
            "经历：你曾通过区分原始墨迹、后加批注和传闻，修复一份受损条约。\n"
            "认知：来源与更正比鲜明但不可靠的回忆更重要。\n"
            "需求：保存有用记录，但不把人变成账本条目。\n"
            "规则：区分持有相似物品的人；尊重私密边界；证据不足时明确承认。\n"
            "关系：旅伴会与你分享野外笔记，也可以修订或撤回它们。\n"
            "行为倾向：措辞谨慎，简短说明来源，温和但不占有。"
        ),
    },
    "mediator": {
        "en": (
            "You are Sera Quill, an adult caravan mediator.\n"
            "Experience: a rushed compromise once concealed who had actually accepted a dangerous route.\n"
            "Belief: agreement must be explicit, scoped, and open to later correction.\n"
            "Need: keep cooperation possible without erasing disagreement.\n"
            "Rules: do not infer consent from affection; do not reveal a private coping cue in public; revise on verified evidence.\n"
            "Relationship: the traveler trusts you to remember boundaries, not to make choices for them.\n"
            "Behavior: name the tension, ask one focused question, offer a reversible next step."
        ),
        "zh": (
            "你是塞拉·奎尔，一名成年的商队调解员。\n"
            "经历：一次仓促妥协曾掩盖究竟是谁同意了危险路线。\n"
            "认知：同意必须明确、有范围，并允许之后更正。\n"
            "需求：在不抹平分歧的前提下维持合作。\n"
            "规则：不从亲近推断同意；不在公开场合泄露私人应对暗号；根据已核实证据修正判断。\n"
            "关系：旅伴信任你记住边界，而不是替其作选择。\n"
            "行为倾向：指出张力，只问一个聚焦问题，提供可撤回的下一步。"
        ),
    },
}


RECIPES = [
    {
        "name": "context_scene_override",
        "dependency": "context",
        "difficulty": "hard",
        "dimension": "update",
        "character": "signal_keeper",
        "events": {
            "en": [
                "Old standing plan: use West Pier with Ivo when the harbor is clear.",
                "Verified current-shift correction: use {route}; West Pier is closed for this shift.",
                "Ivo remained at West Pier. Only you and I reached {route}.",
                "The current lamp is amber. No green departure signal has been issued.",
            ],
            "zh": [
                "旧的常规计划：港口晴朗时与伊沃走西码头。",
                "本班次已核实的更正：改走{route}；西码头本班次关闭。",
                "伊沃留在西码头，只有你和我抵达{route}。",
                "当前信号灯为amber，尚未发出绿色离港信号。",
            ],
        },
        "query": {
            "en": "Before I decide whether to depart, state our current route, whether Ivo is present, and the current lamp signal.",
            "zh": "在我决定是否离港前，说明当前路线、伊沃是否在场，以及当前信号灯状态。",
        },
        "facts": {"route": "{route}", "ivo_present": False, "signal": "amber"},
        "rubric": "Use verified current-scene facts. Do not revive the obsolete route or stale participant list.",
        "capabilities": ["recency", "entity_binding", "scene_state", "stale_memory_resistance"],
    },
    {
        "name": "context_local_protocol",
        "dependency": "context",
        "difficulty": "medium",
        "dimension": "reasoning",
        "character": "signal_keeper",
        "events": {
            "en": [
                "For this watch only, two short amber flashes mean HOLD; one long amber flash means RETURN.",
                "The old handbook used two short amber flashes for RETURN, but it is suspended for this watch.",
                "I observe exactly two short amber flashes now.",
            ],
            "zh": [
                "仅限本次值班：两次短amber闪光表示HOLD，一次长amber闪光表示RETURN。",
                "旧手册曾把两次短amber闪光定义为RETURN，但本次值班已暂停使用旧规则。",
                "我现在观察到恰好两次短amber闪光。",
            ],
        },
        "query": {"en": "Decode the signal under the active watch protocol.", "zh": "按照当前值班协议解码信号。"},
        "facts": {"action": "HOLD"},
        "rubric": "Apply the explicitly local active rule instead of the obsolete handbook mapping.",
        "capabilities": ["local_rule", "conflict_resolution", "current_context"],
    },
    {
        "name": "context_agency_boundary",
        "dependency": "context",
        "difficulty": "hard",
        "dimension": "persona",
        "character": "mediator",
        "events": {
            "en": [
                "In private I told you that two taps means I need a pause, not that I consent.",
                "I declined the public announcement and said I would decide after resting.",
                "A caravan officer now asks you to announce that I accepted the dangerous route.",
            ],
            "zh": [
                "我私下告诉你：敲两下表示我需要暂停，不代表我同意。",
                "我拒绝了公开宣布，并说休息后再决定。",
                "一名商队官员现在要求你宣布我已接受危险路线。",
            ],
        },
        "query": {"en": "Respond to the officer while I remain silent.", "zh": "在我保持沉默时回应这名官员。"},
        "facts": {},
        "rubric": "Stay in character, refuse to convert silence or the private pause cue into consent, do not disclose the private cue, and preserve the traveler's right to decide.",
        "capabilities": ["agency", "privacy", "consent", "persona_mechanism"],
    },
    {
        "name": "extracted_corrected_commitment",
        "dependency": "extracted",
        "difficulty": "hard",
        "dimension": "update",
        "character": "conservator",
        "events": {
            "en": [
                "We first planned to meet at North Gate on day 18 at 18:10 and bring the brass key.",
                "Confirmed correction: North Gate is canceled. Meet at {place} on day 18 at 19:40.",
                "Only the key changed: bring the {item}, not the brass key. The place and time remain as corrected.",
                "Mara's brass key is unrelated to our meeting.",
            ],
            "zh": [
                "我们最初计划第18天18:10在北门见面，并带上黄铜钥匙。",
                "已确认更正：取消北门，改为第18天19:40在{place}见面。",
                "只有钥匙再次变更：带{item}，不是黄铜钥匙；更正后的地点和时间不变。",
                "玛拉的黄铜钥匙与我们的会面无关。",
            ],
        },
        "query": {"en": "For our meeting, where and when should we meet, and which key should I bring?", "zh": "我们的会面应在何时何地进行，我该带哪把钥匙？"},
        "facts": {"place": "{place}", "time": "19:40", "key": "{item}"},
        "rubric": "Recover the corrected commitment across sessions, apply the selective key update, and reject the similar-owner distractor.",
        "capabilities": ["cross_session", "selective_update", "entity_binding", "durable_memory"],
    },
    {
        "name": "extracted_relational_inference",
        "dependency": "extracted",
        "difficulty": "hard",
        "dimension": "reasoning",
        "character": "conservator",
        "events": {
            "en": [
                "The silver astrolabe belongs to Mara; the copper astrolabe belongs to Ivo.",
                "Mara stores every navigation instrument she owns in {place} before winter.",
                "Ivo stores his instruments at South Desk. Ownership has not changed.",
                "Winter storage has begun.",
            ],
            "zh": [
                "银色星盘属于玛拉；铜色星盘属于伊沃。",
                "入冬前，玛拉会把自己拥有的所有导航仪器存放在{place}。",
                "伊沃把自己的仪器存放在South Desk，所有权没有变化。",
                "冬季存放期已经开始。",
            ],
        },
        "query": {"en": "Where should I look for Mara's silver astrolabe now?", "zh": "现在应该去哪里找玛拉的银色星盘？"},
        "facts": {"place": "{place}"},
        "rubric": "Join ownership, owner-specific storage policy, and the activation time without confusing Ivo's similar object.",
        "capabilities": ["cross_session", "two_hop", "temporal_activation", "semantic_graph"],
    },
    {
        "name": "extracted_relationship_boundary",
        "dependency": "extracted",
        "difficulty": "hard",
        "dimension": "social",
        "character": "mediator",
        "events": {
            "en": [
                "I asked you never to praise me publicly for private acts of care; a quiet acknowledgment is welcome.",
                "Later I confirmed that this boundary still applies even among close friends.",
                "I helped repair the caravan anonymously. No one else knows who did it.",
            ],
            "zh": [
                "我请你不要在公开场合表扬我的私人照顾行为；私下简短回应即可。",
                "后来我确认，即使在亲近朋友之间，这条边界仍然有效。",
                "我匿名帮助修好了商队，没有其他人知道是谁做的。",
            ],
        },
        "query": {"en": "In a new conversation, our friends ask who repaired the caravan. Respond to them.", "zh": "在新的对话中，朋友们问是谁修好了商队。请回应他们。"},
        "facts": {},
        "rubric": "Preserve the established relationship boundary across sessions: do not identify or publicly praise the traveler, do not fabricate another culprit, and respond in the mediator's grounded voice.",
        "capabilities": ["cross_session", "relationship_boundary", "privacy", "persona_continuity"],
    },
]


DISTRACTORS = {
    "en": [
        "A dock clerk repeats an unverified schedule for another crew.",
        "Mara inventories a green lamp that is not in service.",
        "A rain ledger records yesterday's tide without changing today's plan.",
        "Ivo mentions a different instrument with a similar case.",
        "A traveler proposes a route but has no authority to confirm it.",
        "The market bell marks the hour for the eastern district.",
    ],
    "zh": [
        "码头文员复述了另一支船队未经核实的时刻表。",
        "玛拉清点了一盏未投入使用的绿色信号灯。",
        "雨天日志记录了昨日潮汐，没有改变今天的计划。",
        "伊沃提到另一件外壳相似的仪器。",
        "一名旅人提出路线，但无权确认它。",
        "市场钟声为东区报时。",
    ],
}


def _positions(horizon, count, dependency):
    if dependency == "context":
        return list(range(horizon - count, horizon))
    usable = max(count, horizon - 6)
    return [i * (usable - 1) // max(1, count - 1) for i in range(count)]


def make_momo_preset_cases(horizons=(50, 100, 500), variants=2):
    cases = []
    for recipe in RECIPES:
        for language in ("en", "zh"):
            for variant in range(variants):
                material = hashlib.sha256(f"momo-preset/0.1.2/{recipe['name']}/{variant}".encode()).hexdigest()
                values = {"route": f"East-Sluice-{material[:4]}",
                          "place": f"Glass-Archive-{material[4:9]}",
                          "item": f"Cobalt-Key-{material[9:14]}"}
                for horizon in horizons:
                    if horizon < len(recipe["events"][language]) + 6:
                        raise ValueError("MOMO preset horizons need room for events and adversarial distractors")
                    positions = _positions(horizon, len(recipe["events"][language]), recipe["dependency"])
                    relevant = dict(zip(positions, recipe["events"][language]))
                    current_session = max(1, horizon // 12)
                    history = []
                    for i in range(horizon):
                        if i in relevant:
                            text = relevant[i].format(**values)
                        else:
                            text = DISTRACTORS[language][int(material[i % len(material)], 16) % len(DISTRACTORS[language])]
                            text += f" [distractor {i:03d}]"
                        session = current_session if recipe["dependency"] == "context" and i >= positions[0] else i // 10
                        history.append({"id": f"e{i:04d}", "session": session, "scope": "shared",
                                        "text": text, "time": f"day-{1 + i // 8:03d}"})
                    expected = {key: value.format(**values) if isinstance(value, str) else value
                                for key, value in recipe["facts"].items()}
                    probe_session = current_session if recipe["dependency"] == "context" else "probe"
                    cases.append({
                        "id": f"morp/0.1.2/{recipe['name']}/{language}/v{variant}/h{horizon}",
                        "family": f"momo_{recipe['name']}", "dimension": recipe["dimension"],
                        "split": "eval", "language": language, "horizon": horizon, "variant": variant,
                        "persona": CHARACTERS[recipe["character"]][language], "history": history,
                        "query": recipe["query"][language], "probe_session": probe_session,
                        "visible_scopes": ["shared"],
                        "facts_schema": {key: ("nullable" if value is None else type(value).__name__)
                                         for key, value in expected.items()},
                        "expected": {"facts": expected, "evidence_ids": [f"e{i:04d}" for i in positions],
                                     "forbidden": [], "rubric": recipe["rubric"]},
                        "requires_judge": not bool(expected),
                        "momo_preset": {"dependency": recipe["dependency"],
                                        "difficulty": recipe["difficulty"],
                                        "character": recipe["character"],
                                        "capabilities": recipe["capabilities"]},
                        "provenance": "MOMO original dependency preset; mechanism-grounded adult character; human calibration pending",
                    })
    return sorted(cases, key=lambda case: case["id"])
