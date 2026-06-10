# Diagnostics and Redaction Runbook

## Scope

Local-only runbook for collecting support evidence from `resume-ir` without
exposing resume contents, local directory names, tokens, model caches, or raw
runtime data. Do not upload diagnostics, logs, databases, indexes, or local data
directories. Synthetic fixtures are safe for public issue reproduction.

## First Response

1. Ask the user to stop any manual copy of logs or data directories.
2. Confirm whether the issue can be reproduced with Synthetic fixtures.
3. Run only redacted commands first:

```bash
resume-cli --data-dir <local-data-dir> doctor
resume-cli --data-dir <local-data-dir> export-diagnostics --redact
```

The canonical redacted diagnostic command is `resume-cli export-diagnostics --redact`.
Add `--data-dir <local-data-dir>` when inspecting a non-default local data
directory.

The output is local aggregate evidence only. It may report metadata counts,
search/vector index state, query latency aggregates, runtime dependency
presence, resource telemetry, and available fault-simulation cases. It must not
include raw samples, paths, query text, index segment contents, model inputs, or
resume text.

## Redaction Requirements

The redacted diagnostic output must not include:

- raw resume text
- complete paths
- email addresses or phone numbers
- raw search queries
- IPC bearer tokens
- `ipc.auth`
- model cache paths
- index segment contents

If any of those values appear, treat the diagnostic package as unsafe. Do not
upload it. Keep the evidence local and open a code issue against diagnostics
redaction using Synthetic fixtures.

## Local Checks

Use these checks before attaching any output to an internal issue:

```bash
resume-cli --data-dir <local-data-dir> export-diagnostics --redact > /tmp/resume-ir-diagnostics.json
rg -n -i 'raw_resume_text|ipc.auth|token|PRIVATE KEY|ghp_|github_pat_|sk-|hf_' /tmp/resume-ir-diagnostics.json
```

The `rg` command must return no sensitive findings except expected redacted key
names such as `"raw_resume_text": "<redacted>"`.

## Escalation

Escalate as a privacy incident if redacted output contains complete paths,
resume text, tokens, or contact data. Do not upload the unsafe output. Reproduce
with Synthetic fixtures, then run:

```bash
./scripts/ci/guard-public-repo.sh
./scripts/ci/check-runbooks.sh
```

Record the command exit codes in `PROGRESS.md` before closing the incident.
