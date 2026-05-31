#!/usr/bin/env python3
"""Probe OpenAI-compatible model output shapes and record parser outcomes."""

from __future__ import annotations

import argparse
import concurrent.futures
import datetime as dt
import hashlib
import json
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


DEFAULT_ENDPOINT = "https://text-erasmus.ngrok.dev/v1"
DEFAULT_API_KEY = "sk-1234"
DEFAULT_RESPONSE_WS_URL = "http://localhost:8080"

PARSER_PROMPTS: dict[str, str] = {
    "raw_json": 'Return only this JSON object shape: {"action":"record","items":[{"id":"alpha","score":1}],"ok":true}.',
    "fenced_json": 'Return one markdown json code fence containing {"action":"record","items":[{"id":"alpha","score":1}],"ok":true}.',
    "fenced_yaml": "Return one markdown yaml code fence with keys action, items, and ok.",
    "python_style_command": 'Return one call: Submit.Result(arg={"action":"record","items":[{"id":"alpha","score":1}],"ok":true}).',
    "python_style_positional": 'Return one call: submit({"action":"record","items":[{"id":"alpha","score":1}],"ok":true}).',
    "openai_tool_call": 'Return only a JSON object like {"name":"Submit.Result","arguments":"{\\"action\\":\\"record\\",\\"items\\":[{\\"id\\":\\"alpha\\",\\"score\\":1}],\\"ok\\":true}"}.',
    "mcp_json_rpc": 'Return only a JSON-RPC object like {"jsonrpc":"2.0","id":"probe-1","method":"Submit.Result","params":{"action":"record","items":[{"id":"alpha","score":1}],"ok":true}}.',
    "xml_tool_call": 'Return one <tool_call> block with <name>Submit.Result</name> and <arguments>{"action":"record","items":[{"id":"alpha","score":1}],"ok":true}</arguments>.',
    "prose_wrapped_json": 'Return a short sentence before and after this JSON object: {"action":"record","items":[{"id":"alpha","score":1}],"ok":true}.',
    "openai_chat_envelope": 'Return only JSON: {"action":"record","items":[{"id":"alpha","score":1}],"ok":true}.',
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", default=DEFAULT_ENDPOINT)
    parser.add_argument("--api-key", default=DEFAULT_API_KEY)
    parser.add_argument("--response-ws-url", default=DEFAULT_RESPONSE_WS_URL)
    parser.add_argument("--count", type=int, default=20)
    parser.add_argument("--max-workers", type=int, default=4)
    parser.add_argument("--out-dir", type=Path, default=Path("artifacts/model-output-probes"))
    parser.add_argument("--grist-bin", type=Path)
    parser.add_argument("--model", action="append", dest="models")
    parser.add_argument("--parser-type", action="append", dest="parser_types")
    args = parser.parse_args()

    endpoint = args.endpoint.rstrip("/")
    models = args.models or list_models(endpoint, args.api_key)
    parser_types = args.parser_types or list(PARSER_PROMPTS)
    grist_bin = args.grist_bin or build_grist()
    run_dir = args.out_dir / dt.datetime.now(dt.UTC).strftime("%Y%m%dT%H%M%SZ")
    run_dir.mkdir(parents=True, exist_ok=True)

    bridge_status = check_response_bridge(args.response_ws_url)
    manifest = {
        "endpoint": endpoint,
        "models": models,
        "parser_types": parser_types,
        "count": args.count,
        "response_ws_url": args.response_ws_url,
        "response_ws_status": bridge_status,
        "started_at": dt.datetime.now(dt.UTC).isoformat(),
    }
    write_json(run_dir / "manifest.json", manifest)

    requests = [
        (endpoint, args.api_key, model, parser_type, request_index, grist_bin)
        for model in models
        for parser_type in parser_types
        for request_index in range(args.count)
    ]
    output_path = run_dir / "records.jsonl"
    with output_path.open("w", encoding="utf-8") as output:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.max_workers) as executor:
            futures = [executor.submit(run_probe, request) for request in requests]
            for future in concurrent.futures.as_completed(futures):
                output.write(json.dumps(future.result(), sort_keys=True) + "\n")
                output.flush()

    summarize_records(output_path, run_dir / "summary.json")
    print(str(run_dir))
    return 0


def list_models(endpoint: str, api_key: str) -> list[str]:
    payload = openai_request(endpoint, api_key, "GET", "/models")
    models = [item["id"] for item in payload.get("data", []) if isinstance(item.get("id"), str)]
    if not models:
        raise RuntimeError("model endpoint returned no model ids")
    return models


def build_grist() -> Path:
    subprocess.run(
        ["cargo", "build", "--features", "cli", "--bin", "grist"],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    return Path("target/debug/grist")


def check_response_bridge(url: str) -> dict[str, Any]:
    request = urllib.request.Request(url, method="GET")
    try:
        with urllib.request.urlopen(request, timeout=2) as response:
            return {"ok": True, "status": response.status}
    except Exception as exc:  # noqa: BLE001 - this is an audit record.
        return {"ok": False, "error": type(exc).__name__, "message": str(exc)}


def run_probe(request: tuple[str, str, str, str, int, Path]) -> dict[str, Any]:
    endpoint, api_key, model, parser_type, request_index, grist_bin = request
    started_at = time.time()
    record: dict[str, Any] = {
        "model": model,
        "parser_type": parser_type,
        "request_index": request_index,
        "started_at": dt.datetime.now(dt.UTC).isoformat(),
    }
    try:
        response = chat_completion(endpoint, api_key, model, PARSER_PROMPTS[parser_type])
        content = extract_content(response)
        parsed = parse_with_grist(grist_bin, content)
        record.update(
            {
                "ok": True,
                "raw_response": response,
                "content": content,
                "content_sha256": sha256_text(content),
                "parser_report": parsed,
            }
        )
    except Exception as exc:  # noqa: BLE001 - failures are the point of this corpus.
        record.update(
            {
                "ok": False,
                "failure_mode": type(exc).__name__,
                "error": str(exc),
            }
        )
    record["elapsed_ms"] = int((time.time() - started_at) * 1000)
    return record


def chat_completion(endpoint: str, api_key: str, model: str, user_prompt: str) -> dict[str, Any]:
    body = {
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": "You are generating parser fixtures. Follow the requested output shape exactly.",
            },
            {"role": "user", "content": user_prompt},
        ],
        "temperature": 0.2,
        "max_tokens": 512,
    }
    return openai_request(endpoint, api_key, "POST", "/chat/completions", body)


def openai_request(
    endpoint: str,
    api_key: str,
    method: str,
    path: str,
    body: dict[str, Any] | None = None,
) -> dict[str, Any]:
    data = None if body is None else json.dumps(body).encode("utf-8")
    request = urllib.request.Request(
        urllib.parse.urljoin(endpoint + "/", path.lstrip("/")),
        data=data,
        method=method,
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        body_text = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {exc.code}: {body_text}") from exc


def extract_content(response: dict[str, Any]) -> str:
    choices = response.get("choices")
    if not isinstance(choices, list) or not choices:
        return json.dumps(response, sort_keys=True)
    message = choices[0].get("message", {})
    content = message.get("content")
    if isinstance(content, str):
        return content
    return json.dumps(response, sort_keys=True)


def parse_with_grist(grist_bin: Path, content: str) -> dict[str, Any]:
    completed = subprocess.run(
        [str(grist_bin), "parse", "model-output", "-", "--strip-think-blocks"],
        input=content,
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        return {
            "status": "parser_process_failed",
            "stderr": completed.stderr,
            "stdout": completed.stdout,
            "returncode": completed.returncode,
        }
    return json.loads(completed.stdout)


def summarize_records(records_path: Path, summary_path: Path) -> None:
    summary: dict[str, Any] = {"total": 0, "ok": 0, "failures": 0, "by_parser_type": {}}
    with records_path.open("r", encoding="utf-8") as records:
        for line in records:
            record = json.loads(line)
            summary["total"] += 1
            parser_type = record["parser_type"]
            bucket = summary["by_parser_type"].setdefault(
                parser_type, {"total": 0, "ok": 0, "unparsed": 0, "parse_failures": 0}
            )
            bucket["total"] += 1
            if record.get("ok"):
                summary["ok"] += 1
                bucket["ok"] += 1
                status = record.get("parser_report", {}).get("payload", {}).get("status")
                if status == "unparsed":
                    bucket["unparsed"] += 1
            else:
                summary["failures"] += 1
                bucket["parse_failures"] += 1
    write_json(summary_path, summary)


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def sha256_text(text: str) -> str:
    return "sha256:" + hashlib.sha256(text.encode("utf-8")).hexdigest()


if __name__ == "__main__":
    sys.exit(main())
