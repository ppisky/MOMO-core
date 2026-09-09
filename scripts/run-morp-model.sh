#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python_bin="${PYTHON:-python3}"
config=""
suite="roleplay"
split="eval"
repeats="1"
variants="1"
output_root=""
allow_ai=0
horizons=()
dimensions=()
families=()
characters=()
arms=()
case_ids=()
dependencies=()

usage() {
  cat <<'EOF'
Usage: scripts/run-morp-model.sh --config PATH [options]

Options:
  --suite roleplay|legacy-all|memory|acgn|momo|stress   Dataset suite (default: roleplay)
  --split dev|eval|all     Dataset split (default: eval)
  --repeats N              Planned repeats (default: 1)
  --horizon N              Event horizon; repeat for more than one (default: 50)
  --variants N             Template variants (default: 1)
  --dimension NAME         Include a dimension; repeat to include more than one
  --family NAME            Include a scenario family; repeat to include more than one
  --character ID_OR_NAME   Include a role-play persona or legacy ACGN character; repeatable
  --arm NAME               Include label_free, labeled, or labels_only; repeatable
  --case-id ID             Include one exact case ID; repeatable
  --dependency NAME        Include context or extracted MOMO presets; repeatable
  --output-root PATH       New output directory (default: unique directory under target)
  --python PATH            Python executable (default: $PYTHON or python3)
  --allow-ai               Execute the plan; omission guarantees zero AI calls
EOF
}

while (($#)); do
  case "$1" in
    --config) config="${2:?missing value for --config}"; shift 2 ;;
    --suite) suite="${2:?missing value for --suite}"; shift 2 ;;
    --split) split="${2:?missing value for --split}"; shift 2 ;;
    --repeats) repeats="${2:?missing value for --repeats}"; shift 2 ;;
    --horizon) horizons+=("${2:?missing value for --horizon}"); shift 2 ;;
    --variants) variants="${2:?missing value for --variants}"; shift 2 ;;
    --dimension) dimensions+=("${2:?missing value for --dimension}"); shift 2 ;;
    --family) families+=("${2:?missing value for --family}"); shift 2 ;;
    --character) characters+=("${2:?missing value for --character}"); shift 2 ;;
    --arm) arms+=("${2:?missing value for --arm}"); shift 2 ;;
    --case-id) case_ids+=("${2:?missing value for --case-id}"); shift 2 ;;
    --dependency) dependencies+=("${2:?missing value for --dependency}"); shift 2 ;;
    --output-root) output_root="${2:?missing value for --output-root}"; shift 2 ;;
    --python) python_bin="${2:?missing value for --python}"; shift 2 ;;
    --allow-ai) allow_ai=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) printf 'Unknown argument: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$config" ]] || { printf '%s\n' '--config is required' >&2; usage >&2; exit 2; }
[[ -f "$config" ]] || { printf 'Config does not exist: %s\n' "$config" >&2; exit 2; }
config="$(cd "$(dirname "$config")" && pwd)/$(basename "$config")"
if ((${#horizons[@]} == 0)); then horizons=(50); fi

cd "$repo_root"
if [[ -z "$output_root" ]]; then
  mkdir -p "$repo_root/target"
  output_root="$(mktemp -d "$repo_root/target/morp-model-XXXXXXXX")"
else
  [[ ! -e "$output_root" ]] || { printf 'Output root already exists: %s\n' "$output_root" >&2; exit 2; }
  mkdir -p "$output_root"
fi

dataset="$output_root/dataset"
plan="$output_root/plan.json"
run="$output_root/run"
report="$output_root/prejudge-report.json"

"$python_bin" -m benchmarks.morp build --out "$dataset" --suite "$suite" \
  --horizons "${horizons[@]}" --variants "$variants"
"$python_bin" -m benchmarks.morp validate "$dataset"
plan_args=(-m benchmarks.morp plan "$dataset" --config "$config" --split "$split" --repeats "$repeats" --out "$plan")
for value in "${dimensions[@]}"; do plan_args+=(--dimension "$value"); done
for value in "${families[@]}"; do plan_args+=(--family "$value"); done
for value in "${characters[@]}"; do plan_args+=(--character "$value"); done
for value in "${arms[@]}"; do plan_args+=(--arm "$value"); done
for value in "${case_ids[@]}"; do plan_args+=(--case-id "$value"); done
for value in "${dependencies[@]}"; do plan_args+=(--dependency "$value"); done
"$python_bin" "${plan_args[@]}"

"$python_bin" - "$plan" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as stream:
    plan = json.load(stream)
print(f"MORP plan: protocol={plan['protocol']}; candidate_calls={plan['candidate_calls']}; "
      f"maintenance_calls={plan['maintenance_calls']}")
PY
printf 'Plan: %s\n' "$plan"

if ((allow_ai == 0)); then
  printf '%s\n' 'Plan-only mode complete (AI calls: 0). Pass --allow-ai to execute this exact plan.'
  exit 0
fi

candidate_calls="$("$python_bin" -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["candidate_calls"])' "$plan")"
printf 'WARNING: AI execution enabled: up to %s candidate calls, plus provider-dependent maintenance calls.\n' "$candidate_calls" >&2
"$python_bin" -m benchmarks.morp run "$dataset" --plan "$plan" --out "$run" --allow-ai
"$python_bin" -m benchmarks.morp score "$dataset" --plan "$plan" \
  --predictions "$run/predictions.jsonl" --out "$report"
printf 'Candidate predictions: %s\n' "$run/predictions.jsonl"
printf 'Pre-judge report: %s\n' "$report"
"$python_bin" - "$report" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as stream:
    summary = json.load(stream)["score_summary"]
objective = "pending" if summary["objective_score"] is None else summary["objective_score"]
selected = "pending" if summary["selected_score"] is None else summary["selected_score"]
print(f"Coverage: {summary['coverage']}/100")
print(f"Objective score: {objective}/100")
print(f"Selected score: {selected}/100 ({summary['status']})")
roleplay = "pending" if summary.get("roleplay_score") is None else summary["roleplay_score"]
print(f"Role-play score: {roleplay}/100")
PY
printf '%s\n' 'Role-play score remains pending until one identity-bound reviewer file is supplied (two model judges remain optional).'
