#!/usr/bin/env python3
"""Reduce append-only loop events through the ACTIVE_GOAL transition graph."""

import argparse, copy, hashlib, json, pathlib, subprocess, sys, tomllib

ROOT = pathlib.Path(__file__).resolve().parents[2]
EVENTS = ROOT / "perf/runs/contract-reconciliation-2026-07-30/events"
SNAPSHOT = ROOT / "perf/current-loop-state.json"
PINS = {
    "active_goal_sha256": ROOT / "ACTIVE_GOAL.toml",
    "acceptance_matrix_sha256": ROOT / "perf/acceptance-matrix.toml",
    "loop_state_schema_sha256": ROOT / "perf/loop-state.schema.json",
    "experiment_report_schema_sha256": ROOT / "perf/experiment-report.schema.json",
    "synthetic_smoke_artifact_manifest_schema_sha256":
        ROOT / "perf/synthetic-smoke-artifact-manifest.schema.json",
}
PRIVACY = {"contains_raw_resume_text", "contains_raw_query_text",
           "contains_candidate_results", "contains_local_paths",
           "contains_tokens", "contains_diagnostics_package"}
ATTRIBUTION_OWNER_ISSUE = "#270"
ATTRIBUTION_PRIMARY_LANE = "full_import_ocr_backlog"
ATTRIBUTION_UNCONFIGURED_TERMINAL = "blocked_missing_configured_private_roots"


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def git(*args):
    run = subprocess.run(["git", *args], cwd=ROOT, capture_output=True)
    require(run.returncode == 0, run.stderr.decode(errors="replace").strip())
    return run.stdout


def load_goal():
    with (ROOT / "ACTIVE_GOAL.toml").open("rb") as handle:
        return tomllib.load(handle)


def load_events():
    paths = sorted(EVENTS.glob("*.json"), key=lambda path: int(path.stem))
    versions = [int(path.stem) for path in paths]
    require(
        versions and versions == list(range(versions[0], versions[-1] + 1)),
        f"non-contiguous event versions: {versions}",
    )
    return [(path, json.loads(raw := path.read_bytes()), raw) for path in paths]


def contracts(goal):
    items = goal["autonomous_delivery"]["transitions"]
    result = {item["name"]: item for item in items}
    require(len(result) == len(items), "duplicate transition")
    return result


def validate(
    record,
    previous_hash,
    expected_version,
    goal,
    graph,
    owner_issue,
    primary_benchmark_lane,
    unconfigured_private_terminal,
):
    path, event, _ = record
    version = event.get("state_version")
    require(
        path.stem == str(version) and version == expected_version + 1,
        f"{path}: version/CAS mismatch",
    )
    require(event.get("expected_state_version") == expected_version, f"{path}: CAS mismatch")
    require(event.get("previous_event_hash") == previous_hash, f"{path}: hash mismatch")
    require(
        event.get("schema_version") in {
            "resume-ir.loop-reconciliation-event.v1",
            "resume-ir.loop-reconciliation-event.v2",
        },
        f"{path}: schema mismatch",
    )
    edge = event.get("transition", {})
    contract = graph.get(edge.get("name"))
    require(
        contract and edge.get("from") in contract["from"]
        and edge.get("to") == contract["to"],
        f"{path}: illegal transition",
    )
    for key in (
        "run_id", "observed_at", "lease_owner", "lease_expires_at",
        "heartbeat_at", "action_id", "idempotency_key",
        "last_confirmed_side_effect", "next_wake_at",
    ):
        require(isinstance(event.get(key), str) and event[key], f"{path}: missing {key}")
    require(event.get("result") == "passed" and event.get("evidence_refs"), f"{path}: result/evidence")
    require(
        set(event.get("privacy", {})) == PRIVACY
        and not any(event["privacy"].values()),
        f"{path}: privacy mismatch",
    )
    require(
        all(
            set(item) == {"command", "exit_code", "evidence_ref"}
            and item["exit_code"] == 0
            for item in event.get("verification", [])
        ),
        f"{path}: verification mismatch",
    )
    observation = event.get("observation", {})
    require(
        observation.get("owner_issue") == owner_issue
        and observation.get("primary_benchmark_lane") == primary_benchmark_lane
        and observation.get("private_input_capability")
        in {unconfigured_private_terminal, "configured_private_roots"}
        and observation.get("current_slice", "").startswith(owner_issue + " ")
        and isinstance(observation.get("active_prs"), list)
        and observation.get("transition_evidence_ref"),
        f"{path}: observation mismatch",
    )
    if "owner_issues" in contract:
        require(owner_issue in contract["owner_issues"], f"{path}: owner not authorized")
    permissions = goal["autonomous_delivery"]["permissions"]
    require(
        all(permissions.get(name) is True for name in contract["required_permissions"]),
        f"{path}: permission denied",
    )
    if event["schema_version"].endswith(".v2"):
        missing = set(contract["required_evidence"]) - set(event.get("evidence", {}))
        require(not missing, f"{path}: missing evidence {sorted(missing)}")
    return contract


def validate_archived_events(records, goal):
    graph = contracts(goal)
    base = records[0][1]["observation"]["live_main_sha"]
    require(len(base) == 40, "base sha mismatch")
    git("merge-base", "--is-ancestor", base, "HEAD")
    base_bytes = git("show", f"{base}:perf/current-loop-state.json")
    base_state = json.loads(base_bytes)
    version, previous_hash = len(base_state["transition_history"]), sha(base_bytes)
    require(
        base_state["workflow_state"] == records[0][1]["transition"]["from"],
        "base state mismatch",
    )
    for record in records:
        validate(
            record,
            previous_hash,
            version,
            goal,
            graph,
            ATTRIBUTION_OWNER_ISSUE,
            ATTRIBUTION_PRIMARY_LANE,
            ATTRIBUTION_UNCONFIGURED_TERMINAL,
        )
        version = record[1]["state_version"]
        previous_hash = sha(record[2])


def validate_successor_snapshot(records, goal):
    active_issue = goal["scope"]["active_slice"]["issue"]
    require(active_issue != ATTRIBUTION_OWNER_ISSUE, "successor issue required")
    validate_archived_events(records, goal)
    snapshot = json.loads(SNAPSHOT.read_bytes())
    require(
        snapshot.get("current_slice", "").startswith(active_issue + " "),
        "successor snapshot current_slice mismatch",
    )
    require(
        snapshot.get("github_ledger", {}).get("primary_issue") == active_issue,
        "successor snapshot primary issue mismatch",
    )
    expected_goal_hash = sha((ROOT / "ACTIVE_GOAL.toml").read_bytes())
    require(
        snapshot.get("contract_pins", {}).get("active_goal_sha256")
        == expected_goal_hash,
        "successor snapshot active-goal pin mismatch",
    )


def replace(text, old, new, label):
    require(text.count(old) == 1, f"{label}: anchor mismatch")
    return text.replace(old, new, 1)


def scalar(text, key, old, new):
    return replace(
        text, f'  "{key}": {json.dumps(old)},',
        f'  "{key}": {json.dumps(new)},', key,
    )


def render(key, value):
    return f'  "{key}": ' + json.dumps(value, indent=2).replace("\n", "\n  ")


def apply(text, event, contract, goal, state_hash):
    observation, edge = event["observation"], event["transition"]; current = json.loads(text); text = scalar(text, "workflow_state", edge["from"], edge["to"])
    actions = contract["allowed_actions"]
    phase = "contract_reconciliation" if "edit_contracts" in actions else "attribution_execution" if "activate_attribution_phase" in actions else None
    if phase:
        lane = goal["scope"]["active_slice"]["attribution"][phase]["evidence_lane"]
        if lane != current["evidence_lane"]:
            text = scalar(text, "evidence_lane", current["evidence_lane"], lane)
    text = scalar(text, "current_slice", current["current_slice"], observation["current_slice"])
    marker = '\n  ],\n  "verification": {'
    history = {"from": edge["from"], "to": edge["to"],
               "evidence_ref": observation["transition_evidence_ref"]}
    text = replace(text, marker, ",\n    " + json.dumps(history, separators=(",", ":")) + marker, "history")
    marker = '\n    ],\n    "all_required_commands_ran":'
    commands = "".join(
        ",\n      " + json.dumps(item, separators=(",", ":"))
        for item in event["verification"]
    )
    text = replace(text, marker, commands + marker, "commands")
    text = text.replace('"all_required_commands_ran": false', '"all_required_commands_ran": true', 1)
    runtime = [
        ("platform_lane", "macos_m4_discovery"), ("run_id", event["run_id"]),
        ("state_version", event["state_version"]), ("previous_state_hash", state_hash),
        ("expected_state_version", event["state_version"]),
        ("last_confirmed_side_effect", event["last_confirmed_side_effect"]),
        ("blocked_count", 0),
        ("same_blocker_key", observation["private_input_capability"] if observation["private_input_capability"] != "configured_private_roots" else ""),
    ]
    block = "\n".join(f'  "{key}": {json.dumps(value)},' for key, value in runtime)
    ledger_at = text.rfind('  "github_ledger": {')
    start = text.find('  "platform_lane": "macos_m4_discovery",')
    if start >= 0:
        text = text[:start] + block + "\n" + text[ledger_at:]
    else:
        text = text[:ledger_at] + block + "\n" + text[ledger_at:]
    ledger = {"primary_issue": observation["owner_issue"],
              "active_prs": observation["active_prs"], "open_blockers": []}
    ledger_at = text.rfind('  "github_ledger": {')
    require(text.endswith("  }\n}\n"), "ledger anchor mismatch")
    return text[:ledger_at] + render("github_ledger", ledger) + "\n}\n"


def reduce(records=None):
    goal, graph = load_goal(), contracts(load_goal())
    records = records or load_events()
    active = goal["scope"]["active_slice"]
    attr = active["attribution"]
    base = records[0][1]["observation"]["live_main_sha"]
    require(len(base) == 40, "base sha mismatch")
    git("merge-base", "--is-ancestor", base, "HEAD")
    base_bytes = git("show", f"{base}:perf/current-loop-state.json")
    text, state = base_bytes.decode(), json.loads(base_bytes)
    version, previous_hash = len(state["transition_history"]), sha(base_bytes)
    require(state["workflow_state"] == records[0][1]["transition"]["from"], "base state mismatch")
    for record in records:
        contract = validate(
            record,
            previous_hash,
            version,
            goal,
            graph,
            active["issue"],
            attr["primary_benchmark_lane"],
            active["unconfigured_private_run_terminal"],
        )
        text = apply(text, record[1], contract, goal, sha(text.encode()))
        pins = {key: sha(path.read_bytes()) for key, path in PINS.items()}
        pins["git_head_sha"] = base
        start, end = text.index('  "contract_pins": {'), text.index('\n  "current_slice":')
        text = text[:start] + render("contract_pins", pins) + "," + text[end:]
        version, previous_hash = record[1]["state_version"], sha(record[2])
    require(json.loads(text)["state_version"] == version, "derived version mismatch")
    return text.encode()


def follow_up(records):
    prior = records[-1]
    version = prior[1]["state_version"] + 1
    observation = copy.deepcopy(prior[1]["observation"])
    event = {
        "schema_version": "resume-ir.loop-reconciliation-event.v2",
        "run_id": prior[1]["run_id"], "state_version": version,
        "previous_event_hash": sha(prior[2]), "expected_state_version": version - 1,
        "observed_at": "test", "lease_owner": "test", "lease_expires_at": "test",
        "heartbeat_at": "test", "action_id": "test", "idempotency_key": "test",
        "last_confirmed_side_effect": "test", "next_wake_at": "test",
        "transition": {"name": "sync_base", "from": "pr_opened", "to": "base_synced"},
        "result": "passed", "evidence_refs": ["test"],
        "evidence": {"base_sha": 1, "head_sha": 1, "reconciliation_status": 1},
        "observation": observation, "verification": [],
        "privacy": {key: False for key in PRIVACY},
    }
    raw = json.dumps(event, sort_keys=True).encode()
    return EVENTS / f"{version}.json", event, raw


def self_test():
    records, failures, goal = load_events(), [], load_goal()
    validate_archived_events(records, goal)
    if goal["scope"]["active_slice"]["issue"] == ATTRIBUTION_OWNER_ISSUE:
        next_event = follow_up(records)
        state = json.loads(reduce([*records, next_event]))
        require(
            state["state_version"] == 556
            and state["workflow_state"] == "base_synced",
            "556 did not advance",
        )
        for label, mutate in (
            ("transition", lambda item: item["transition"].update(from_="bad")),
            ("hash", lambda item: item.update(previous_event_hash="0" * 64)),
            ("version", lambda item: item.update(state_version=557)),
        ):
            event = copy.deepcopy(next_event[1])
            if label == "transition":
                event["transition"]["from"] = "bad"
            else:
                mutate(event)
            try:
                reduce([*records, (next_event[0], event, json.dumps(event).encode())])
            except ValueError:
                continue
            failures.append(label)
    for label, key, value in (
        ("archive owner", "owner_issue", "#999"),
        ("archive hash", "previous_event_hash", "0" * 64),
        ("archive version", "state_version", 999),
    ):
        altered = copy.deepcopy(records)
        path, event, _ = altered[-1]
        if key in event["observation"]:
            event["observation"][key] = value
        else:
            event[key] = value
        altered[-1] = (path, event, json.dumps(event).encode())
        try:
            validate_archived_events(altered, goal)
        except ValueError:
            continue
        failures.append(label)
    require(not failures, f"negative tests passed unexpectedly: {failures}")


def main():
    parser = argparse.ArgumentParser()
    modes = parser.add_mutually_exclusive_group(required=True)
    for mode in ("check", "write", "self-test"):
        modes.add_argument(f"--{mode}", action="store_true")
    args = parser.parse_args()
    goal = load_goal()
    if args.self_test:
        self_test()
    elif goal["scope"]["active_slice"]["issue"] != ATTRIBUTION_OWNER_ISSUE:
        require(not args.write, "successor snapshots cannot be written from archived events")
        validate_successor_snapshot(load_events(), goal)
    else:
        output = reduce()
        if args.write:
            SNAPSHOT.write_bytes(output)
        else:
            require(SNAPSHOT.read_bytes() == output, "snapshot is not reducer output")
    print("current loop-state reducer passed")


if __name__ == "__main__":
    try:
        main()
    except (KeyError, OSError, TypeError, ValueError, json.JSONDecodeError) as exc:
        print(f"current loop-state reducer failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
