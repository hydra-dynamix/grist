#!/usr/bin/env python3
"""Build, verify, canonicalize, and intake governed Grist fixtures."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import shutil
import sys
import zipfile
from pathlib import Path, PurePosixPath
from typing import Any

GENERATOR_VERSION = "grist-fixture-builder/1"
CANONICAL_JSON = "grist/canonical-json/v1"
REQUIRED_CLASSES = {
    "minimal_valid", "representative_real_world", "maximum_complexity", "empty",
    "truncated", "malformed", "adversarial", "encrypted", "oversized",
    "deeply_nested", "mixed_encoding", "invalid_text", "nested_attachments",
    "nested_containers", "unsupported_construct", "provider_recording",
    "malicious_active_content", "downstream_regression",
}


def canonical_bytes(value: Any) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False
    ).encode("utf-8")


def digest(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def safe_relative(value: str) -> PurePosixPath:
    path = PurePosixPath(value)
    if path.is_absolute() or not path.parts or any(part in ("", ".", "..") for part in path.parts):
        raise ValueError(f"unsafe fixture-relative path: {value}")
    if "\\" in value:
        raise ValueError(f"fixture paths must use forward slashes: {value}")
    return path


def valid_slug(value: str) -> bool:
    return bool(value) and not value.startswith("-") and not value.endswith("-") and all(
        char in "abcdefghijklmnopqrstuvwxyz0123456789-" for char in value
    )


def deterministic_zip(members: list[tuple[str, bytes]]) -> bytes:
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w", compression=zipfile.ZIP_STORED, strict_timestamps=True) as archive:
        for name, content in sorted(members):
            safe_relative(name)
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            info.flag_bits = 0
            archive.writestr(info, content)
    return stream.getvalue()


def nested_zip(depth: int, leaf_name: str, leaf: bytes) -> bytes:
    if depth < 1 or depth > 32:
        raise ValueError("nested-container depth must be between 1 and 32")
    content = deterministic_zip([(leaf_name, leaf)])
    for level in range(depth - 1, 0, -1):
        content = deterministic_zip([(f"level-{level + 1}.zip", content)])
    return content


def provider_recording(spec: dict[str, Any]) -> bytes:
    kind = spec["provider_kind"]
    input_bytes = spec["input_text"].encode("utf-8")
    request = {
        "kind": kind,
        "input_identity": {"byte_length": len(input_bytes), "sha256": digest(input_bytes)},
        "network_access": "denied",
        "configuration_digest": digest(canonical_bytes(spec["public_configuration"])),
        "parameters_digest": digest(canonical_bytes(spec["options"])),
    }
    result = {"kind": kind, "result": spec["result"]}
    catalog = {
        "schema_version": "grist/provider-recording-catalog/v1",
        "kind": kind,
        "provider": spec["provider"],
        "entries": [{
            "request": request,
            "request_digest": digest(canonical_bytes(request)),
            "output_sha256": digest(canonical_bytes(result)),
            "result": result,
        }],
    }
    return canonical_bytes(catalog) + b"\n"


def render_output(spec: dict[str, Any]) -> bytes:
    kind = spec["kind"]
    if kind == "text":
        return spec["content"].encode("utf-8")
    if kind == "hex":
        return bytes.fromhex(spec["hex"])
    if kind == "repeat":
        return spec["content"].encode("utf-8") * int(spec["count"])
    if kind == "canonical_json":
        return canonical_bytes(spec["value"]) + b"\n"
    if kind == "nested_zip":
        return nested_zip(int(spec["depth"]), spec["leaf_name"], spec["leaf_content"].encode("utf-8"))
    if kind == "provider_recording":
        return provider_recording(spec)
    raise ValueError(f"unknown fixture builder kind: {kind}")


def load_recipes(repository: Path) -> dict[str, Any]:
    path = repository / "fixtures" / "builders" / "recipes.v1.json"
    recipes = json.loads(path.read_text(encoding="utf-8"))
    if recipes.get("schema_version") != "grist/fixture-builder-recipes/v1":
        raise ValueError("unsupported fixture builder recipe schema")
    if recipes.get("generator_version") != GENERATOR_VERSION:
        raise ValueError("recipe generator_version does not match this builder")
    return recipes


def generated_outputs(repository: Path) -> dict[str, bytes]:
    recipes = load_recipes(repository)
    outputs: dict[str, bytes] = {}
    for spec in recipes["outputs"]:
        relative = str(safe_relative(spec["path"]))
        if relative in outputs:
            raise ValueError(f"duplicate generated output: {relative}")
        outputs[relative] = render_output(spec)
    return outputs


def update_manifest_identities(repository: Path, outputs: dict[str, bytes]) -> None:
    manifest_path = repository / "fixtures" / "corpus.v1.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    for registered in manifest["formats"].values():
        for case in registered["cases"]:
            input_record = case["input"]
            path = input_record.get("path")
            if path in outputs:
                input_record["byte_length"] = len(outputs[path])
                input_record["sha256"] = digest(outputs[path])
            for expected in case.get("expected", []):
                path = expected["path"]
                if path in outputs:
                    data = outputs[path]
                    expected["byte_length"] = len(data)
                    expected["sha256"] = digest(data)
                    value = json.loads(data)
                    expected["canonical_sha256"] = digest(canonical_bytes(value))
    manifest_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n"
    )


def build(repository: Path, check: bool, update_manifest: bool) -> int:
    outputs = generated_outputs(repository)
    failures: list[str] = []
    for relative, content in outputs.items():
        target = repository / "fixtures" / Path(*PurePosixPath(relative).parts)
        if check:
            if not target.is_file() or target.read_bytes() != content:
                failures.append(relative)
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
    if check:
        if failures:
            for relative in failures:
                print(f"generated fixture drift: fixtures/{relative}", file=sys.stderr)
            return 1
        print(f"verified {len(outputs)} deterministic generated fixture(s)")
        return 0
    if update_manifest:
        update_manifest_identities(repository, outputs)
    print(f"built {len(outputs)} deterministic fixture(s)")
    return 0


def validate_manifest(repository: Path) -> int:
    manifest = json.loads((repository / "fixtures" / "corpus.v1.json").read_text(encoding="utf-8"))
    actual_classes = manifest.get("policy", {}).get("required_classes", [])
    errors: list[str] = []
    if len(actual_classes) != len(set(actual_classes)) or set(actual_classes) != REQUIRED_CLASSES:
        errors.append("policy.required_classes does not exactly cover Section 16.1")
    seen_ids: set[str] = set()
    seen_paths: set[str] = set()
    for format_name, registered in manifest.get("formats", {}).items():
        if registered.get("format") != format_name:
            errors.append(f"format registration mismatch: {format_name}")
        for case in registered.get("cases", []):
            case_id = case.get("id", "")
            if case_id in seen_ids:
                errors.append(f"duplicate fixture id: {case_id}")
            seen_ids.add(case_id)
            records = [case.get("input", {})] + case.get("expected", [])
            for record in records:
                relative = record.get("path")
                if relative is None:
                    continue
                try:
                    safe_relative(relative)
                except ValueError as error:
                    errors.append(str(error))
                    continue
                if relative in seen_paths:
                    errors.append(f"duplicate fixture path: {relative}")
                seen_paths.add(relative)
                target = repository / "fixtures" / Path(*PurePosixPath(relative).parts)
                if not target.is_file():
                    errors.append(f"missing fixture: {relative}")
                    continue
                data = target.read_bytes()
                if record.get("byte_length") != len(data) or record.get("sha256") != digest(data):
                    errors.append(f"fixture identity mismatch: {relative}")
    recipe_paths = set(generated_outputs(repository))
    for relative in sorted(recipe_paths - seen_paths):
        errors.append(f"builder output is not registered: {relative}")
    for relative in sorted(
        path
        for path in seen_paths
        if path.startswith("generated/") and path not in recipe_paths
    ):
        errors.append(f"generated fixture has no deterministic recipe: {relative}")
    generated_root = repository / "fixtures" / "generated"
    actual_generated = {
        path.relative_to(repository / "fixtures").as_posix()
        for path in generated_root.rglob("*")
        if path.is_file()
    }
    for relative in sorted(actual_generated - recipe_paths):
        errors.append(f"unregistered generated file: {relative}")
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(f"validated {len(seen_ids)} fixture registration(s) and {len(seen_paths)} checked file(s)")
    return build(repository, check=True, update_manifest=False)


def intake(repository: Path, args: argparse.Namespace) -> int:
    source = args.source.resolve(strict=True)
    data = source.read_bytes()
    if not args.metadata_only and (
        args.data_classification != "public" or not args.redistribution_permitted
    ):
        raise ValueError(
            "checked-in regression bytes require public classification and --redistribution-permitted"
        )
    manifest_path = repository / "fixtures" / "corpus.v1.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    for registered in manifest["formats"].values():
        if any(case["id"] == args.id for case in registered["cases"]):
            raise ValueError(f"fixture ID already exists: {args.id}")
    if not valid_slug(args.id) or not valid_slug(args.format):
        raise ValueError("regression ID and format must be lowercase slugs")
    if not all((args.source_project.strip(), args.issue_uri.strip(), args.license_expression.strip())):
        raise ValueError("project, issue URI, and license expression must be non-empty")
    classes = list(dict.fromkeys(["downstream_regression", *args.fixture_class]))
    unknown = set(classes) - REQUIRED_CLASSES
    if unknown:
        raise ValueError(f"unknown fixture class(es): {', '.join(sorted(unknown))}")
    relative: str | None = None
    storage = "external_only"
    redistribution = "metadata_only"
    if not args.metadata_only:
        suffix = source.suffix.lower() or ".bin"
        relative = f"regressions/{args.format}/{args.id}/input{suffix}"
        safe_relative(relative)
        target = repository / "fixtures" / Path(*PurePosixPath(relative).parts)
        target.parent.mkdir(parents=True, exist_ok=False)
        shutil.copyfile(source, target)
        storage = "checked_in"
        redistribution = "permitted"
    case = {
        "id": args.id,
        "classes": classes,
        "input": {
            "byte_length": len(data),
            "sha256": digest(data),
            "media_type": args.media_type,
        },
        "provenance": {
            "origin": "downstream_regression",
            "source": args.source_project,
            "source_project": args.source_project,
            "issue_uri": args.issue_uri,
            "original_sha256": digest(data),
            "license": {
                "expression": args.license_expression,
                "redistribution": redistribution,
            },
        },
        "handling": {
            "storage": storage,
            "data_classification": args.data_classification,
            "inert_only": True,
            "execution_allowed": False,
            "network_allowed": False,
        },
        "expected": [],
    }
    if relative is not None:
        case["input"]["path"] = relative
    registered = manifest["formats"].setdefault(args.format, {"format": args.format, "cases": []})
    registered["cases"].append(case)
    registered["cases"].sort(key=lambda item: item["id"])
    manifest["formats"] = dict(sorted(manifest["formats"].items()))
    manifest_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n"
    )
    print(f"registered downstream regression {args.id} ({storage})")
    return 0


def canonicalize(input_path: Path, output_path: Path) -> int:
    value = json.loads(input_path.read_text(encoding="utf-8"))
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_bytes(canonical_bytes(value) + b"\n")
    return 0


def parser() -> argparse.ArgumentParser:
    command = argparse.ArgumentParser(description=__doc__)
    command.add_argument("--repository", type=Path, default=Path(__file__).resolve().parents[1])
    subcommands = command.add_subparsers(dest="command", required=True)
    build_parser = subcommands.add_parser("build", help="regenerate deterministic fixtures")
    build_parser.add_argument("--check", action="store_true")
    build_parser.add_argument("--update-manifest", action="store_true")
    subcommands.add_parser("validate", help="verify policy, identities, and generated bytes")
    canonical = subcommands.add_parser("canonicalize", help="write canonical JSON v1 plus LF")
    canonical.add_argument("input", type=Path)
    canonical.add_argument("output", type=Path)
    regression = subcommands.add_parser("intake", help="register a downstream regression")
    regression.add_argument("source", type=Path)
    regression.add_argument("--format", required=True)
    regression.add_argument("--id", required=True)
    regression.add_argument("--source-project", required=True)
    regression.add_argument("--issue-uri", required=True)
    regression.add_argument("--license-expression", required=True)
    regression.add_argument("--media-type", required=True)
    regression.add_argument("--fixture-class", action="append", default=["malformed"])
    regression.add_argument("--data-classification", default="public")
    regression.add_argument("--redistribution-permitted", action="store_true")
    regression.add_argument("--metadata-only", action="store_true")
    return command


def main() -> int:
    args = parser().parse_args()
    repository = args.repository.resolve(strict=True)
    try:
        if args.command == "build":
            if args.check and args.update_manifest:
                raise ValueError("--check and --update-manifest are mutually exclusive")
            return build(repository, args.check, args.update_manifest)
        if args.command == "validate":
            return validate_manifest(repository)
        if args.command == "canonicalize":
            return canonicalize(args.input, args.output)
        if args.command == "intake":
            return intake(repository, args)
        raise AssertionError("unreachable command")
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"fixture corpus error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
