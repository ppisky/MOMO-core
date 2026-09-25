#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

python_bin="${PYTHON:-python3}"
skip_audit="${MOMO_SKIP_AUDIT:-0}"

step() {
  printf '\n==> %s\n' "$1"
}

step "Verify frozen 1.0 contract fixtures"
"$python_bin" - <<'PY'
import hashlib
import pathlib

root = pathlib.Path("contracts/1.0")
for line in (root / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
    expected, relative = line.split(maxsplit=1)
    path = root / relative.strip()
    actual = hashlib.sha256(path.read_bytes()).hexdigest()
    if actual != expected.lower():
        raise SystemExit(f"contract hash mismatch: {path}")
    print(f"{relative.strip()}: OK")
PY

step "Check Rust formatting"
cargo fmt --all -- --check

step "Run strict Clippy"
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

step "Run all Rust tests"
cargo test --workspace --all-features --locked

step "Run MORP offline contracts"
PYTHON="$python_bin" bash scripts/test-morp.sh

step "Build release artifacts"
cargo build --release --workspace --all-features --locked

step "Build Rust documentation"
cargo doc --workspace --all-features --no-deps --locked

if [[ "$skip_audit" == "1" ]]; then
  step "RustSec audit skipped by MOMO_SKIP_AUDIT=1; release gate is incomplete"
else
  step "Run RustSec audit"
  if ! cargo audit --version >/dev/null 2>&1; then
    printf 'cargo-audit is required; install it or set MOMO_SKIP_AUDIT=1\n' >&2
    exit 1
  fi
  cargo audit
fi

server_bin="target/release/momo-server"
if [[ ! -x "$server_bin" && -f "${server_bin}.exe" ]]; then
  server_bin="${server_bin}.exe"
fi
if [[ ! -x "$server_bin" && ! -f "$server_bin" ]]; then
  printf 'release server was not produced at %s\n' "$server_bin" >&2
  exit 1
fi

step "Scan release binary for removed 1.0 identities"
removed_patterns=(
  "MOMO_SCOPE_ID"
  "01900000-0000-7000-8000-000000000101"
  "character_catalogue"
  "character-catalogue"
)
for pattern in "${removed_patterns[@]}"; do
  if grep -aFq -- "$pattern" "$server_bin"; then
    printf 'removed identity found in release binary: %s\n' "$pattern" >&2
    exit 1
  fi
done

step "Run release server loopback smoke test"
smoke_dir="$(mktemp -d "$repo_root/target/release-smoke-XXXXXXXX")"
smoke_port="$($python_bin - <<'PY'
import socket

with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)"
server_pid=""

cleanup() {
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill -INT "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

MOMO_DATA_DIR="$smoke_dir/data" \
MOMO_SERVER_BIND="127.0.0.1:$smoke_port" \
  "$server_bin" >"$smoke_dir/stdout.log" 2>"$smoke_dir/stderr.log" &
server_pid="$!"

health_path="$smoke_dir/health.json"
health_ok=0
for ((attempt = 0; attempt < 100; attempt++)); do
  if ! kill -0 "$server_pid" 2>/dev/null; then
    break
  fi
  if "$python_bin" - "$smoke_port" "$health_path" <<'PY'
import json
import pathlib
import sys
import urllib.request

port = int(sys.argv[1])
output = pathlib.Path(sys.argv[2])
try:
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=1) as response:
        payload = json.load(response)
except Exception:
    raise SystemExit(1)
if payload.get("ok") is not True or payload.get("service") != "momo-server":
    raise SystemExit(1)
with urllib.request.urlopen(f"http://127.0.0.1:{port}/v1/memory/recovery", timeout=3) as response:
    if json.load(response) != {}:
        raise SystemExit("fresh runtime reported a recovery conflict")
output.write_text(json.dumps(payload, ensure_ascii=False, sort_keys=True), encoding="utf-8")
PY
  then
    health_ok=1
    break
  fi
  sleep 0.1
done

if [[ "$health_ok" != "1" ]]; then
  printf 'release server health smoke failed\n' >&2
  if [[ -f "$smoke_dir/stderr.log" ]]; then
    sed -n '1,120p' "$smoke_dir/stderr.log" >&2
  fi
  exit 1
fi

cleanup
server_pid=""
trap - EXIT

step "Record release binary hash"
"$python_bin" - "$server_bin" <<'PY'
import hashlib
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
print(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path}")
PY
printf 'Release smoke artifacts: %s\n' "$smoke_dir"
printf 'MOMO Core release verification passed.\n'
