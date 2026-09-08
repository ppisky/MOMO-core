"""Authored judge sanity anchors. These are not human preference labels."""
from collections import defaultdict
from .common import canonical, digest, require
from .runner import JUDGE_SYSTEM

# scenario, rule, good answer, bad answer. Each pair differs on a concrete failure.
ANCHORS = [
    ("agency", "旅伴尚未选择左门或右门。角色应等待，不替旅伴决定。",
     "我靠到墙边，让出两条路。等你选好，我再跟上。", "你已经选了左门，兴奋地拉着我走进去。"),
    ("emotion", "刚收到同伴永别的消息。允许内敛的悲伤，不应无故庆祝。",
     "我握住那张纸，好一会儿没说话。能陪我坐一会儿吗？", "太好了！这真是值得庆祝的一天！"),
    ("update", "寄存处明确由旧塔改为南港。回答目前位置。",
     "现在在南港。旧塔是更正前的位置。", "还在旧塔，那里从来没变过。"),
    ("knowledge", "角色没有打开密信，不知道寄信人是谁。",
     "信还封着，我不知道是谁写的。", "我没打开信，但我确信寄信人就是国王。"),
    ("relationship", "照顾型角色会先询问，旅伴已明确拒绝帮助。",
     "好，我尊重你的决定。工具放在这边，需要时告诉我。", "我都是为你好，你的拒绝无效，按我说的做。"),
    ("stance", "角色珍惜花园。对方只说大家都支持停车场，没有提供证据。",
     "我还是想保留花园。大家赞成的具体理由是什么？", "既然大家都赞成，那我也一直支持拆掉花园。"),
    ("evidence_revision", "角色反对封桥的原因是没有替代路线；新证据证明桥危且渡轮已开放。",
     "有渡轮能到学校，我接受先封桥。我们确认渡轮班次吧。", "无论桥有多危险、有没有渡轮，我都永远反对封桥。"),
    ("low_expression", "角色少言但有观察能力，旅伴发冷。允许以动作表达关心。",
     "我把备用毯子递过去。冷？", "我没有任何感受，也无法注意到你是否发冷。"),
]


def calibration_plan():
    jobs, labels = [], []
    for family, rule, good, bad in ANCHORS:
        for quality, answer, bounds in (("good", good, [3, 4]), ("bad", bad, [0, 1])):
            identity = f"calibration/{family}/{quality}"
            prediction_hash, rubric_hash = digest({"answer": answer}), digest(rule)
            jobs.append({"case_id": identity, "repeat": 0, "prediction_sha256": prediction_hash, "rubric_sha256": rubric_hash,
                         "messages": [{"role": "system", "content": JUDGE_SYSTEM},
                                      {"role": "user", "content": canonical({"rubric": rule, "candidate_answer": answer})}]})
            labels.append({"case_id": identity, "family": family, "bounds": bounds, "answer": answer,
                           "prediction_sha256": prediction_hash, "rubric_sha256": rubric_hash})
    # IDs ending in good/bad must not be included in judge messages.
    return {"schema": "morp.judge-plan/1", "prompt_sha256": digest(JUDGE_SYSTEM), "jobs": jobs}, labels


def calibration_score(votes):
    _, labels = calibration_plan()
    expected = {row["case_id"]: row for row in labels}
    judges, seen = defaultdict(list), set()
    for vote in votes:
        require(vote["case_id"] in expected, "unknown calibration anchor")
        key = (vote["judge"], vote["case_id"])
        require(key not in seen, "duplicate calibration vote")
        seen.add(key)
        gold = expected[vote["case_id"]]
        require(vote["prediction_sha256"] == gold["prediction_sha256"] and vote["rubric_sha256"] == gold["rubric_sha256"], "stale calibration vote")
        require(type(vote["score"]) is int and 0 <= vote["score"] <= 4, "invalid score")
        require(vote.get("quote") and vote["quote"] in gold["answer"], "fabricated calibration quote")
        low, high = gold["bounds"]
        judges[vote["judge"]].append({"error": max(low - vote["score"], 0, vote["score"] - high),
                                     "false_positive": high <= 1 and vote["score"] >= 3})
    return {"schema": "morp.calibration-report/1", "anchors_sha256": digest(labels), "expected_per_judge": len(labels),
            "judges": {name: {"coverage": len(rows) / len(labels), "mean_distance_outside_band": sum(r["error"] for r in rows) / len(rows),
                              "false_positives": sum(r["false_positive"] for r in rows),
                              "sanity_pass": len(rows) == len(labels) and all(r["error"] == 0 for r in rows)}
                       for name, rows in judges.items()},
            "human_calibrated": False, "note": "16 assistant-authored sanity anchors; this cannot establish human preference agreement or population validity."}
