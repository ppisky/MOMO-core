#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
python_bin="${PYTHON:-python3}"
"$python_bin" -m unittest discover -s benchmarks/morp/tests -v
cargo build -p momo_core --example morp_contract_probe --locked
morp_output="$(mktemp -d "$PWD/target/morp-offline-XXXXXXXX")"
"$python_bin" -m benchmarks.morp build --out "$morp_output/dataset"
"$python_bin" -m benchmarks.morp plan "$morp_output/dataset" --config benchmarks/morp/configs/momo.example.json --out "$morp_output/momo-plan.json"
"$python_bin" -m benchmarks.morp offline --probe target/debug/examples/morp_contract_probe --out "$morp_output/contracts.json"
printf 'Offline artifacts: %s (AI calls: 0)\n' "$morp_output"
