#!/usr/bin/env python3
"""Derive the public loop snapshot from bounded reconciliation events."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
EVENTS = ROOT / "perf/runs/contract-reconciliation-2026-07-30/events"
SNAPSHOT = ROOT / "perf/current-loop-state.json"
CONTRACT_FILES = {
    "active_goal_sha256": ROOT / "ACTIVE_GOAL.toml",
    "acceptance_matrix_sha256": ROOT / "perf/acceptance-matrix.toml",
    "loop_state_schema_sha256": ROOT / "perf/loop-state.schema.json",
    "experiment_report_schema_sha256": ROOT / "perf/experiment-report.schema.json",
    "synthetic_smoke_artifact_manifest_schema_sha256": (
        ROOT / "perf/synthetic-smoke-artifact-manifest.schema.json"
    ),
}
PRIVACY_FIELDS = {
    "contains_raw_resume_text", "contains_raw_query_text", "contains_candidate_results",
    "contains_local_paths", "contains_tokens", "contains_diagnostics_package",
}
TRANSITIONS = {
    554: ("reconcile_current_main_import_attribution_contract", "pr_opened", "evidence_review"),
    555: ("open_pr", "evidence_review", "pr_opened"),
}
EVENT_STRINGS = (
    "run_id", "observed_at", "lease_owner", "lease_expires_at", "heartbeat_at",
    "action_id", "idempotency_key", "last_confirmed_side_effect", "next_wake_at",
)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def git_output(*args: str) -> bytes:
    result = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, check=False)
    require(result.returncode == 0, result.stderr.decode(errors="replace").strip())
    return result.stdout


def replace_once(text: str, old: str, new: str, label: str) -> str:
    require(text.count(old) == 1, f"{label}: expected exactly one source anchor")
    return text.replace(old, new, 1)


def load_events() -> list[tuple[pathlib.Path, dict]]:
    paths = sorted(EVENTS.glob("*.json"), key=lambda path: int(path.stem))
    versions = [int(path.stem) for path in paths]
    require(versions in ([554], [554, 555]), f"unexpected event versions: {versions!r}")
    return [(path, json.loads(path.read_bytes())) for path in paths]


def validate_event(
    event: dict,
    path: pathlib.Path,
    previous_hash: str,
    expected_version: int,
) -> None:
    version = event.get("state_version")
    require(path.stem == str(version) and version == expected_version + 1, f"{path}: version mismatch")
    require(event.get("schema_version") == "resume-ir.loop-reconciliation-event.v1", f"{path}: schema mismatch")
    require(event.get("previous_event_hash") == previous_hash, f"{path}: event hash mismatch")
    require(event.get("expected_state_version") == expected_version, f"{path}: CAS version mismatch")
    transition = event.get("transition")
    observed_transition = (
        transition.get("name"),
        transition.get("from"),
        transition.get("to"),
    ) if isinstance(transition, dict) else None
    require(observed_transition == TRANSITIONS.get(version), f"{path}: transition mismatch")
    for key in EVENT_STRINGS:
        require(isinstance(event.get(key), str) and bool(event[key]), f"{path}: {key} must be non-empty")
    require(event.get("result") == "passed", f"{path}: event result must be passed")
    require(isinstance(event.get("evidence_refs"), list) and bool(event["evidence_refs"]), f"{path}: evidence refs missing")
    privacy = event.get("privacy")
    require(isinstance(privacy, dict) and set(privacy) == PRIVACY_FIELDS
            and all(value is False for value in privacy.values()), f"{path}: privacy mismatch")
    verification = event.get("verification")
    require(
        isinstance(verification, list)
        and all(
            isinstance(entry, dict)
            and set(entry) == {"command", "exit_code", "evidence_ref"}
            and entry.get("exit_code") == 0
            for entry in verification
        ),
        f"{path}: verification mismatch",
    )
    observation = event.get("observation")
    require(isinstance(observation, dict), f"{path}: observation must be an object")
    common = {
        "owner_issue": "#270",
        "primary_benchmark_lane": "full_import_ocr_backlog",
        "private_input_capability": "blocked_missing_configured_private_roots",
    }
    if version == 554:
        common |= {
            "merged_prior_prs": ["#249", "#267", "#269"],
            "current_schema": 35,
            "configured_private_roots": False,
            "active_prs": [],
            "transition_evidence_ref": "https://github.com/FrankQDWang/resume-ir/issues/270",
        }
    for key, value in common.items():
        require(observation.get(key) == value, f"{path}: observation.{key} mismatch")
    if version == 555:
        require(isinstance(observation.get("active_prs"), list)
                and len(observation["active_prs"]) == 1, f"{path}: PR mismatch")


def render_field(name: str, value: object) -> str:
    return f'  "{name}": ' + json.dumps(value, indent=2).replace("\n", "\n  ")


def replace_contract_pins(text: str, base_sha: str) -> str:
    pins = {key: digest(path.read_bytes()) for key, path in CONTRACT_FILES.items()}
    pins["git_head_sha"] = base_sha
    start = text.index('  "contract_pins": {')
    end = text.index('\n  "current_slice":', start)
    return text[:start] + render_field("contract_pins", pins) + "," + text[end:]


def replace_ledger(text: str, active_prs: list[str]) -> str:
    ledger = {"primary_issue": "#270", "active_prs": active_prs, "open_blockers": []}
    start = text.rfind('  "github_ledger": {')
    require(start >= 0 and text.endswith("  }\n}\n"), "github ledger anchor mismatch")
    return text[:start] + render_field("github_ledger", ledger) + "\n}\n"


def runtime_block(event: dict, previous_state_hash: str) -> str:
    fields = [
        ("platform_lane", "macos_m4_discovery"),
        ("run_id", event["run_id"]),
        ("state_version", event["state_version"]),
        ("previous_state_hash", previous_state_hash),
        ("expected_state_version", event["state_version"]),
        ("last_confirmed_side_effect", event["last_confirmed_side_effect"]),
        ("blocked_count", 0),
        ("same_blocker_key", "blocked_missing_configured_private_roots"),
    ]
    return "\n".join(f'  "{key}": {json.dumps(value)},' for key, value in fields)


def apply_event(text: str, event: dict, previous_state_hash: str) -> str:
    observation = event["observation"]
    transition = event["transition"]
    current_slice = json.loads(text)["current_slice"]
    text = replace_once(
        text,
        f'  "workflow_state": "{transition["from"]}",',
        f'  "workflow_state": "{transition["to"]}",',
        "workflow state",
    )
    if event["state_version"] == 554:
        text = replace_once(
            text, '  "evidence_lane": "gui_manual",', '  "evidence_lane": "w0_docs",', "evidence lane"
        )
    text = replace_once(
        text,
        f'  "current_slice": {json.dumps(current_slice)},',
        f'  "current_slice": {json.dumps(observation["current_slice"])},',
        "current slice",
    )
    history_marker = '\n  ],\n  "verification": {'
    history = {
        "from": transition["from"],
        "to": transition["to"],
        "evidence_ref": observation["transition_evidence_ref"],
    }
    text = replace_once(
        text,
        history_marker,
        ",\n    " + json.dumps(history, separators=(",", ":")) + history_marker,
        "transition history",
    )
    commands_marker = '\n    ],\n    "all_required_commands_ran":'
    commands = "".join(
        ",\n      " + json.dumps(entry, separators=(",", ":"))
        for entry in event["verification"]
    )
    text = replace_once(text, commands_marker, commands + commands_marker, "verification commands")
    text = text.replace('"all_required_commands_ran": false', '"all_required_commands_ran": true', 1)
    ledger_marker = '  "github_ledger": {'
    block = runtime_block(event, previous_state_hash)
    if '  "platform_lane": "macos_m4_discovery",' in text:
        start = text.index('  "platform_lane": "macos_m4_discovery",')
        end = text.index(ledger_marker, start)
        text = text[:start] + block + "\n" + text[end:]
    else:
        text = replace_once(text, ledger_marker, block + "\n" + ledger_marker, "runtime state")
    return replace_ledger(text, observation["active_prs"])


def reduce_snapshot() -> bytes:
    events = load_events()
    base_sha = events[0][1].get("observation", {}).get("live_main_sha")
    require(isinstance(base_sha, str) and len(base_sha) == 40, "live_main_sha must be a full commit")
    git_output("merge-base", "--is-ancestor", base_sha, "HEAD")
    base_bytes = git_output("show", f"{base_sha}:perf/current-loop-state.json")
    state = json.loads(base_bytes)
    expected_version = len(state.get("transition_history", []))
    require(expected_version == 553 and state.get("workflow_state") == "pr_opened",
            "legacy base snapshot bootstrap mismatch")
    text = base_bytes.decode()
    previous_event_hash = digest(base_bytes)
    for path, event in events:
        validate_event(event, path, previous_event_hash, expected_version)
        text = apply_event(text, event, digest(text.encode()))
        text = replace_contract_pins(text, base_sha)
        expected_version = event["state_version"]
        previous_event_hash = digest(path.read_bytes())
    state = json.loads(text)
    require(state.get("state_version") == expected_version
            and state.get("current_slice") == "#270 current-main installed-equivalent import attribution contract"
            and state.get("privacy") == {field: False for field in PRIVACY_FIELDS},
            "derived snapshot invariant mismatch")
    return text.encode()


def main() -> int:
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--write", action="store_true")
    args = parser.parse_args()
    reduced = reduce_snapshot()
    if args.write:
        SNAPSHOT.write_bytes(reduced)
    elif not SNAPSHOT.exists() or SNAPSHOT.read_bytes() != reduced:
        raise ValueError("perf/current-loop-state.json is not the reducer output")
    print("current loop-state reducer passed")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (KeyError, OSError, TypeError, ValueError, json.JSONDecodeError) as exc:
        print(f"current loop-state reducer failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
