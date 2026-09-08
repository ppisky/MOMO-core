"""Strict, deterministic artifacts shared by every benchmark stage."""
import hashlib
import json
import math
from pathlib import Path


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)


def digest(value):
    return hashlib.sha256(canonical(value).encode("utf-8")).hexdigest()


def implementation_digest():
    root = Path(__file__).parent
    return digest({p.relative_to(root).as_posix(): p.read_text(encoding="utf-8").replace("\r\n", "\n")
                   for p in sorted(root.glob("*.py"))})


def _pairs(items):
    result = {}
    for key, value in items:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def parse(text):
    def real(value):
        number = float(value)
        require(math.isfinite(number), "non-finite JSON number")
        return number
    return json.loads(text, object_pairs_hook=_pairs,
                      parse_float=real,
                      parse_constant=lambda value: (_ for _ in ()).throw(ValueError(f"non-finite number: {value}")))


def read_json(path):
    return parse(Path(path).read_text(encoding="utf-8"))


def read_jsonl(path):
    return [parse(line) for line in Path(path).read_text(encoding="utf-8").splitlines() if line.strip()]


def write_new(path, value, lines=False):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    content = "".join(canonical(row) + "\n" for row in value) if lines else canonical(value) + "\n"
    # Refuse accidental overwrite of an experiment or a pinned dataset.
    with path.open("x", encoding="utf-8", newline="\n") as stream:
        stream.write(content)


def unique(rows, key):
    result = {}
    for row in rows:
        identity = row[key]
        if identity in result:
            raise ValueError(f"duplicate {key}: {identity}")
        result[identity] = row
    return result


def require(condition, message):
    if not condition:
        raise ValueError(message)


def finite(value):
    return type(value) in (int, float) and math.isfinite(value)


def load_dataset(directory):
    root = Path(directory)
    manifest = read_json(root / "manifest.json")
    cases = read_jsonl(root / "cases.jsonl")
    require(manifest["schema"] == "morp.dataset/1", "unsupported dataset schema")
    require(digest(cases) == manifest["cases_sha256"], "dataset content hash mismatch")
    require(len(cases) == manifest["count"] and bool(cases), "empty or miscounted dataset")
    unique(cases, "id")
    family_splits = {}
    for case in cases:
        from .corpus import DIMENSIONS
        require(case["dimension"] in DIMENSIONS, "unknown scoring dimension")
        require(isinstance(case["persona"], str) and bool(case["persona"].strip()), "persona required")
        require(isinstance(case["query"], str) and bool(case["query"].strip()), "query required")
        require(type(case["requires_judge"]) is bool, "requires_judge must be boolean")
        require(set(case["facts_schema"]) == set(case["expected"]["facts"]), "facts/labels schema mismatch")
        require(case["requires_judge"] or bool(case["expected"]["facts"]), "objective case has no targets")
        require(isinstance(case["expected"]["rubric"], str) and bool(case["expected"]["rubric"].strip()), "rubric required")
        require(len(case["history"]) == case["horizon"], "horizon/history mismatch")
        require(case["split"] in ("dev", "eval"), "invalid split")
        require(family_splits.setdefault(case["family"], case["split"]) == case["split"], "family crosses splits")
        event_ids = unique(case["history"], "id")
        require(all(isinstance(e["text"], str) and bool(e["text"].strip()) for e in case["history"]), "invalid history text")
        require(set(case["expected"]["evidence_ids"]) <= set(event_ids), "dangling evidence")
        require(all(event_ids[e]["scope"] in case["visible_scopes"] for e in case["expected"]["evidence_ids"]), "private evidence in gold")
        preset = case.get("momo_preset")
        if preset is not None:
            require(isinstance(preset, dict), "momo_preset must be an object")
            require(preset.get("dependency") in ("context", "extracted"), "unknown MOMO preset dependency")
            require(preset.get("difficulty") in ("medium", "hard"), "unknown MOMO preset difficulty")
            capabilities = preset.get("capabilities")
            require(isinstance(capabilities, list) and bool(capabilities)
                    and len(capabilities) == len(set(capabilities))
                    and all(isinstance(value, str) and bool(value) for value in capabilities),
                    "MOMO preset capabilities must be unique nonempty strings")
            shared_sessions = {event["session"] for event in case["history"]
                               if event["scope"] in case["visible_scopes"]}
            if preset["dependency"] == "context":
                require(case.get("probe_session") in shared_sessions,
                        "context preset probe must continue a visible session")
            else:
                require(case.get("probe_session") not in shared_sessions,
                        "extracted preset probe must start a fresh session")
    return manifest, cases
