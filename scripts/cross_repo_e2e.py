#!/usr/bin/env python3
"""Launch real Core + mobot processes and verify one idempotent 0.5 response turn."""

from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


class UpstreamHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    chat_calls = 0
    embedding_calls = 0

    def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        length = int(self.headers.get("content-length", "0"))
        request = json.loads(self.rfile.read(length) or b"{}")
        if self.path == "/v1/chat/completions":
            type(self).chat_calls += 1
            response = {
                "id": "chatcmpl_cross_repo",
                "object": "chat.completion",
                "created": int(time.time()),
                "model": request.get("model", "chat-model"),
                "choices": [
                    {
                        "index": 0,
                        "message": {"role": "assistant", "content": "cross-repo-ok"},
                        "finish_reason": "stop",
                    }
                ],
                "usage": {
                    "prompt_tokens": 5,
                    "completion_tokens": 2,
                    "total_tokens": 7,
                },
            }
        elif self.path == "/v1/embeddings":
            type(self).embedding_calls += 1
            response = {
                "object": "list",
                "model": request.get("model", "embedding-model"),
                "data": [
                    {
                        "object": "embedding",
                        "index": 0,
                        "embedding": [1.0, 0.0, 0.0],
                    }
                ],
                "usage": {"prompt_tokens": 1, "total_tokens": 1},
            }
        else:
            self.send_error(404)
            return
        encoded = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(encoded)))
        self.send_header("x-request-id", "cross-repo-upstream")
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, _format: str, *_args: object) -> None:
        return


class QuietThreadingHTTPServer(ThreadingHTTPServer):
    def handle_error(self, _request: object, _client_address: object) -> None:
        return


def json_request(method: str, url: str, payload: object | None = None) -> object:
    encoded = None if payload is None else json.dumps(payload).encode()
    request = urllib.request.Request(url, data=encoded, method=method)
    if encoded is not None:
        request.add_header("content-type", "application/json")
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.load(response)


def wait_for_health(origin: str, process: subprocess.Popen[str], name: str) -> None:
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if process.poll() is not None:
            output = process.stdout.read() if process.stdout else ""
            raise RuntimeError(f"{name} exited early: {output}")
        try:
            json_request("GET", f"{origin}/health")
            return
        except (OSError, urllib.error.HTTPError):
            time.sleep(0.05)
    raise TimeoutError(f"{name} did not become healthy")


def terminate(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def mobot_config(gateway_port: int, upstream_port: int, temp: Path) -> str:
    return f"""
schema_version = 3

[[providers]]
provider_id = "cross-repo-upstream"
name = "Cross repo upstream"
base_url = "http://127.0.0.1:{upstream_port}/v1"
protocol = "openai_chat_completions"
api_key_env = "MOMO_CROSS_REPO_UPSTREAM_KEY"

[[models]]
profile_id = "chat-main"
name = "Chat"
provider_id = "cross-repo-upstream"
model_id = "chat-model"
category = "chat"
context_window = 8192

[models.request_parameters]
max_tokens = 256

[models.reliability]
timeout_seconds = 5
max_concurrency = 4
transient_retries = 1
circuit_breaker_failures = 3
circuit_breaker_cooldown_seconds = 1

[[models]]
profile_id = "embedding-main"
name = "Embedding"
provider_id = "cross-repo-upstream"
model_id = "embedding-model"
category = "embedding"

[models.embedding_profile]
dimension = 3
normalization = "none"

[models.request_parameters]
encoding_format = "float"

[model_use]
conversation = "chat-main"
memory_distillation = "chat-main"
semantic_graph_governance = "chat-main"
embedding = "embedding-main"

[gateway]
bind = "127.0.0.1:{gateway_port}"
allow_remote = false
api_key_env = "MOMO_CROSS_REPO_GATEWAY_KEY"

[core]
origin = "http://127.0.0.1:9"
data_dir = "{temp.as_posix()}/unused-data"
run_dir = "{temp.as_posix()}/unused-run"
autostart = false
""".strip()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--core-binary", required=True, type=Path)
    parser.add_argument("--mobot-binary", required=True, type=Path)
    args = parser.parse_args()
    core_binary = args.core_binary.resolve()
    mobot_binary = args.mobot_binary.resolve()
    for binary in (core_binary, mobot_binary):
        if not binary.exists():
            raise FileNotFoundError(binary)

    upstream_port = free_port()
    gateway_port = free_port()
    core_port = free_port()
    upstream = QuietThreadingHTTPServer(("127.0.0.1", upstream_port), UpstreamHandler)
    upstream_thread = threading.Thread(target=upstream.serve_forever, daemon=True)
    upstream_thread.start()

    with tempfile.TemporaryDirectory(prefix="momo-cross-repo-") as directory:
        temp = Path(directory)
        config = temp / "mobot.toml"
        config.write_text(mobot_config(gateway_port, upstream_port, temp), encoding="utf-8")
        clean_environment = os.environ.copy()
        for name in ("MOMO_CROSS_REPO_UPSTREAM_KEY", "MOMO_CROSS_REPO_GATEWAY_KEY"):
            clean_environment.pop(name, None)
        mobot = subprocess.Popen(
            [str(mobot_binary), "--config", str(config), "gateway", "serve"],
            cwd=directory,
            env=clean_environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        core_environment = clean_environment.copy()
        core_environment.update(
            {
                "MOMO_SERVER_BIND": f"127.0.0.1:{core_port}",
                "MOMO_DATA_DIR": str(temp / "core-data"),
                "MOMO_MODEL_GATEWAY_ORIGIN": f"http://127.0.0.1:{gateway_port}/v1",
                "MOMO_MEMORY_DISTILL_EVERY": "200",
                "MOMO_NSG_GOVERN_EVERY": "200",
            }
        )
        core = subprocess.Popen(
            [str(core_binary)],
            cwd=directory,
            env=core_environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        gateway_origin = f"http://127.0.0.1:{gateway_port}"
        core_origin = f"http://127.0.0.1:{core_port}"
        direct_core: subprocess.Popen[str] | None = None
        try:
            wait_for_health(gateway_origin, mobot, "mobot gateway")
            wait_for_health(core_origin, core, "MOMO Core")
            character = json_request(
                "POST",
                f"{core_origin}/v1/characters",
                {
                    "name": "Cross Repo",
                    "description": "E2E character",
                    "character_markdown": "You are a cross-repository test character.",
                    "user_markdown": "",
                },
            )
            character_id = str(character["id"])
            payload = {
                "model": "conversation",
                "input": "Run the 0.5 cross-repository flow.",
                "momo": {
                    "schema": "momo.responses/0.5",
                    "request_id": "cross-repo-e2e-1",
                    "character_id": character_id,
                    "memory": True,
                    "semantic_graph": True,
                    "mo_state": True,
                },
            }
            first = json_request("POST", f"{core_origin}/v1/responses", payload)
            replay = json_request("POST", f"{core_origin}/v1/responses", payload)
            assert first == replay
            assert first["output_text"] == "cross-repo-ok"
            assert first["finish_reason"] == "stop"
            assert first["momo"]["upstream_request_id"] == "cross-repo-upstream"
            assert first["momo"]["warnings"] == []
            conversation_id = first["momo"]["conversation_id"]
            messages = json_request(
                "GET", f"{core_origin}/v1/conversations/{conversation_id}/messages"
            )
            assert len(messages) == 2
            assert UpstreamHandler.chat_calls == 1
            assert UpstreamHandler.embedding_calls == 1

            direct_port = free_port()
            direct_environment = clean_environment.copy()
            direct_environment.update(
                {
                    "MOMO_SERVER_BIND": f"127.0.0.1:{direct_port}",
                    "MOMO_DATA_DIR": str(temp / "direct-core-data"),
                    "MOMO_MODEL_GATEWAY_ORIGIN": f"http://127.0.0.1:{upstream_port}/v1",
                    "MOMO_MEMORY_DISTILL_EVERY": "200",
                    "MOMO_NSG_GOVERN_EVERY": "200",
                }
            )
            direct_core = subprocess.Popen(
                [str(core_binary)],
                cwd=directory,
                env=direct_environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
            )
            direct_origin = f"http://127.0.0.1:{direct_port}"
            wait_for_health(direct_origin, direct_core, "direct MOMO Core")
            direct_character = json_request(
                "POST",
                f"{direct_origin}/v1/characters",
                {
                    "name": "Direct",
                    "description": "Direct upstream character",
                    "character_markdown": "You are the direct deployment test character.",
                    "user_markdown": "",
                },
            )
            direct = json_request(
                "POST",
                f"{direct_origin}/v1/responses",
                {
                    "model": "chat-model",
                    "input": "Run the direct deployment flow.",
                    "momo": {
                        "schema": "momo.responses/0.5",
                        "request_id": "direct-e2e-1",
                        "character_id": str(direct_character["id"]),
                        "memory": False,
                        "semantic_graph": False,
                        "mo_state": False,
                    },
                },
            )
            assert direct["output_text"] == "cross-repo-ok"
            assert direct["finish_reason"] == "stop"
            assert UpstreamHandler.chat_calls == 2
        finally:
            if direct_core is not None:
                terminate(direct_core)
            terminate(core)
            terminate(mobot)
            upstream.shutdown()
            upstream.server_close()

    print(
        "Direct Core and Core -> mobot 0.5 response, memory/NSG, persistence and replay passed"
    )


if __name__ == "__main__":
    main()
