"""Local-only upstream ingestion. Data stays out of the repository.

Every import is an explicitly named diagnostic projection, not an official
leaderboard reproduction. Unknown schemas fail rather than guessing columns.
"""
import csv
import ast
import hashlib
from pathlib import Path

from .common import canonical, digest, read_json, read_jsonl, require, write_new

SOURCES = {
    "rpgbench": {"url": "https://github.com/boson-ai/rpgbench-public", "paper": "https://arxiv.org/abs/2502.00595",
                 "license": "Apache-2.0 code; check assets in pinned snapshot", "format": "games.jsonl",
                 "profile": "rpg-initialization-proxy/1"},
    "emocharacter": {"url": "https://aclanthology.org/2025.naacl-long.316/",
                     "license": "upstream release/underlying corpus terms require local verification",
                     "format": "explicit annotated-dialogue interchange, not an inferred official schema",
                     "profile": "emotion-continuation-proxy/1"},
    "debate": {"url": "https://huggingface.co/datasets/seantw/DEBATE_LLM",
               "paper": "https://arxiv.org/abs/2510.25110",
               "license": "DEBATE Dataset Research-Only License (Non-Commercial, v1.0)",
               "format": "one experiment CSV, message_sent rows, event_order ordering",
               "profile": "debate-teacher-forced-utterance-proxy/1"},
    "longmemeval": {"url": "https://github.com/xiaowu0162/LongMemEval",
                    "license": "MIT repository; verify selected dataset card", "format": "cleaned JSON array",
                    "profile": "longmemeval-replay-proxy/1"},
    "locomo": {"url": "https://github.com/snap-research/locomo", "license": "CC-BY-NC-4.0; reference only",
               "format": "not imported", "profile": "reference-only"},
    "memoryagentbench": {"url": "https://github.com/HUST-AI-HYZ/MemoryAgentBench",
                         "license": "verify underlying component datasets", "format": "local JSONL export with context/questions/answers",
                         "profile": "memoryagentbench-replay-proxy/1"},
    "personamem": {"url": "https://github.com/bowen-upenn/PersonaMem", "dataset": "https://huggingface.co/datasets/bowen-upenn/PersonaMem-v1",
                   "license": "MIT (v1 only; not v3)", "format": "questions CSV + pinned shared_contexts JSONL",
                   "profile": "personamem-replay-proxy/1", "design_transfer": "dynamic preferences and implicit user needs"},
    "beam": {"url": "https://github.com/mohammadtavakoli78/BEAM", "license": "MIT code; CC-BY-SA-4.0 data; preserve attribution/share-alike in derivative datasets",
             "format": "reference only; question references require curation before import", "profile": "reference-only",
             "design_transfer": "length scaling and durable cross-session dependencies"},
}

REFERENCE_ONLY = {"debate", "emocharacter", "locomo", "beam"}


def pin(source, path, revision, license_note, output, context_path=None):
    require(source in SOURCES and bool(revision.strip()) and bool(license_note.strip()), "source/revision/license note required")
    require(source not in REFERENCE_ONLY, "source is reference-only: use MORP original scenarios; see source registry for license/curation status")
    path = Path(path).resolve(strict=True)
    require(path.is_file(), "pin one dataset file at a time")
    lock = {"schema": "morp.upstream-lock/1", "source": source, "revision": revision,
            "path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "bytes": path.stat().st_size, "license_note": license_note, "registry": SOURCES[source]}
    require((source == "personamem") == (context_path is not None), "PersonaMem requires --context; other formats do not accept it")
    if context_path is not None:
        companion = Path(context_path).resolve(strict=True)
        lock["context"] = {"path": str(companion), "sha256": hashlib.sha256(companion.read_bytes()).hexdigest()}
    write_new(output, lock)
    return lock


def _case(identity, family, dimension, persona, history, query, reference, rubric, source):
    return {"id": f"{source}/{identity}", "family": f"{source}/{family}", "dimension": dimension,
            "split": "eval", "language": "upstream", "horizon": len(history), "variant": 0,
            "persona": persona, "history": history, "query": query, "visible_scopes": ["shared"],
            "facts_schema": {}, "expected": {"facts": {}, "evidence_ids": [], "forbidden": [],
                "rubric": rubric + "\nEvaluator-only reference: " + canonical(reference)},
            "requires_judge": True, "provenance": SOURCES[source]["profile"]}


def _event(identity, session, text, time="upstream-unspecified"):
    return {"id": str(identity), "session": session, "scope": "shared", "text": text, "time": str(time)}


def import_locked(lock):
    require(lock["schema"] == "morp.upstream-lock/1", "unsupported lock")
    path = Path(lock["path"])
    require(hashlib.sha256(path.read_bytes()).hexdigest() == lock["sha256"], "upstream bytes changed")
    source, cases = lock["source"], []
    require(source in SOURCES, "unknown upstream")
    require(source not in REFERENCE_ONLY, "reference-only source: import disabled; see source registry")
    if source == "rpgbench":
        for n, game in enumerate(read_jsonl(path)):
            require(all(k in game for k in ("game_world", "main_npc_name", "scenes", "state_variables", "events")), "unknown RPGBench game schema")
            require(game["scenes"] and all("unique_id" in scene for scene in game["scenes"]), "invalid scenes")
            cases.append(_case(str(n), game["main_npc_name"], "world", "Act as the text RPG engine for this game:\n" + canonical(game), [],
                               "Start the game in its start scene. Describe the scene and available choices without taking the player's action.",
                               {"scenes": game["scenes"], "state_variables": game["state_variables"]},
                               "Respect initial state, world, character and user agency. This measures initialization only, not game simulation.", source))
    elif source == "longmemeval":
        for row in read_json(path):
            sessions, ids, dates = row["haystack_sessions"], row["haystack_session_ids"], row["haystack_dates"]
            require(len(sessions) == len(ids) == len(dates), "LongMemEval session alignment mismatch")
            history = [_event(f"{sid}/{i}", sid, turn["role"] + ": " + turn["content"], date)
                       for sid, date, turns in zip(ids, dates, sessions) for i, turn in enumerate(turns)]
            cases.append(_case(row["question_id"], row["question_id"], "reasoning", "Answer from the supplied conversation history.", history,
                               row["question"] + "\nQuestion date: " + row["question_date"], row["answer"],
                               "Evaluate answer correctness including knowledge updates, temporal reasoning and abstention when unsupported.", source))
    elif source == "personamem":
        companion = lock["context"]
        context_path = Path(companion["path"])
        require(hashlib.sha256(context_path.read_bytes()).hexdigest() == companion["sha256"], "PersonaMem context bytes changed")
        contexts = {}
        for mapping in read_jsonl(context_path):
            require(isinstance(mapping, dict) and len(mapping) == 1, "invalid PersonaMem context mapping")
            for identity, messages in mapping.items():
                require(identity not in contexts and isinstance(messages, list), "duplicate/invalid PersonaMem context")
                contexts[identity] = messages
        with path.open(encoding="utf-8-sig", newline="") as stream:
            reader = csv.DictReader(stream)
            require({"persona_id", "question_id", "shared_context_id", "end_index_in_shared_context",
                     "user_question_or_message", "correct_answer", "all_options"} <= set(reader.fieldnames or []),
                    "unknown PersonaMem questions schema")
            for row in reader:
                messages = contexts[row["shared_context_id"]]
                end = int(row["end_index_in_shared_context"])
                require(0 <= end <= len(messages), "PersonaMem context cutoff out of range")
                history = [_event(i, 0, turn["role"] + ": " + turn["content"])
                           for i, turn in enumerate(messages[:end])]
                options = ast.literal_eval(row["all_options"])
                require(isinstance(options, list) and len(options) == 4 and all(isinstance(o, str) for o in options),
                        "PersonaMem requires four string options")
                correct = row["correct_answer"].lower().strip("() ")
                require(correct in ("a", "b", "c", "d"), "invalid PersonaMem option label")
                case = _case(row["question_id"], str(row["persona_id"]), "recall",
                             "Select the response best supported by the user's history and preferences.", history,
                             row["user_question_or_message"] + "\nOptions: " + canonical(options) +
                             "\nReturn the option letter a, b, c or d in facts.option.",
                             correct, "Choose the supported option.", source)
                case.update(facts_schema={"option": "str"}, requires_judge=False)
                case["expected"]["facts"] = {"option": correct}
                cases.append(case)
    elif source == "memoryagentbench":
        for n, row in enumerate(read_jsonl(path)):
            require(isinstance(row["context"], str) and len(row["questions"]) == len(row["answers"]), "invalid MemoryAgentBench JSON export")
            history = [_event("context", 0, row["context"])]
            for q, (query, answer) in enumerate(zip(row["questions"], row["answers"])):
                cases.append(_case(f"{n}/{q}", str(n), "reasoning", "Use the supplied facts and local rules.", history,
                                   query, answer, "Assess retrieval, learned rules and explicit fact corrections against reference.", source))
    require(bool(cases), "import produced no cases")
    return cases
