# Fault Injection Runbook

## Scope

Local-only runbook for safe, synthetic fault simulation. Do not upload generated
diagnostics, logs, probe files, databases, indexes, or local data directories.
Synthetic fixtures are the only supported public reproduction data.

These commands exercise controlled probes. They do not fill a real disk, kill a
user-installed service, disconnect drives, or run destructive platform tests.

## Safe Fault Probes

Use a scratch directory under a temporary local path:

```bash
scratch="$(mktemp -d)"
data_dir="$(mktemp -d)"
```

Canonical probe forms:

- `resume-cli fault-simulate --case disk-space-low`
- `resume-cli fault-simulate --case permission-denied`
- `resume-cli fault-simulate --case file-lock`
- `resume-cli fault-simulate --case index-snapshot-corrupt`
- `resume-cli fault-simulate --case migration-failure`
- `resume-cli fault-simulate --case model-checksum`
- `resume-cli fault-simulate --case daemon-kill`
- `resume-cli fault-simulate --case ocr-crash`
- `resume-cli fault-simulate --case battery-mode`
- `resume-cli fault-simulate --case external-drive-disconnect`

Add `--data-dir <local-data-dir>` and `--scratch-dir <scratch-dir>` for an
isolated local run.

Run disk-space budget simulation:

```bash
resume-cli --data-dir "$data_dir" fault-simulate \
  --case disk-space-low \
  --scratch-dir "$scratch" \
  --required-bytes 4096 \
  --available-bytes 1024
```

Run permission-denied simulation:

```bash
resume-cli --data-dir "$data_dir" fault-simulate \
  --case permission-denied \
  --scratch-dir "$scratch"
```

Run file-lock contention simulation:

```bash
resume-cli --data-dir "$data_dir" fault-simulate \
  --case file-lock \
  --scratch-dir "$scratch"
```

Run full-text index active-snapshot corruption simulation:

```bash
resume-cli --data-dir "$data_dir" fault-simulate \
  --case index-snapshot-corrupt \
  --scratch-dir "$scratch"
```

Run metadata migration failure simulation:

```bash
resume-cli --data-dir "$data_dir" fault-simulate \
  --case migration-failure \
  --scratch-dir "$scratch"
```

Run model artifact checksum simulation against a controlled local model file:

```bash
printf 'SYNTHETIC MODEL CHECKSUM PROBE\n' > "$scratch/model.bin"
resume-cli --data-dir "$data_dir" fault-simulate \
  --case model-checksum \
  --scratch-dir "$scratch" \
  --model-file "$scratch/model.bin" \
  --expected-sha256 0000000000000000000000000000000000000000000000000000000000000000
```

Run daemon kill/restart simulation against a controlled daemon binary:

```bash
resume-cli --data-dir "$data_dir" fault-simulate \
  --case daemon-kill \
  --scratch-dir "$scratch" \
  --daemon-binary ./target/debug/resume-daemon
```

Run OCR command crash simulation against a controlled OCR command:

```bash
resume-cli --data-dir "$data_dir" fault-simulate \
  --case ocr-crash \
  --scratch-dir "$scratch" \
  --ocr-command ./tests/fixtures/bin/crashing-ocr
```

Run battery-mode degradation simulation:

```bash
resume-cli --data-dir "$data_dir" fault-simulate \
  --case battery-mode \
  --scratch-dir "$scratch" \
  --battery-state battery
```

Run external-drive disconnect recovery simulation:

```bash
resume-cli --data-dir "$data_dir" fault-simulate \
  --case external-drive-disconnect \
  --scratch-dir "$scratch" \
  --drive-state disconnected
```

Expected safe output includes `paths: <redacted>` and does not include the
scratch path, data path, command path, model bytes, OCR stdout, OCR stderr,
synthetic index probe text, or probe bytes. Checksum output is limited to short
digest prefixes. Battery and external-drive probes are safe state simulations
only; their output must say `real hardware drill: blocked`.

Add `--json` to any safe fault probe to emit structured evidence using
`fault-simulation.v1`. JSON output must include `redacted: true`,
`paths: <redacted>`, `evidence_level: local_synthetic_fault_probe`, the
canonical fault name, reproduced/not-reproduced status, and only aggregate or
synthetic probe details. Do not upload JSON reports generated from local
operator runs.

## Unsafe Faults

The following remain not complete or BLOCKED for public release readiness:

- actual ENOSPC by filling a real filesystem
- service-manager kill of a user-installed daemon
- real battery-mode transition validation
- real external-drive disconnect validation
- licensed model selection, download, and distribution validation
- Windows and macOS service-manager fault evidence

Do not simulate those by damaging user data. Use a dedicated test machine or VM
with disposable data and document the exact platform, build SHA, and cleanup
steps.

## Cleanup

```bash
rm -rf "$scratch" "$data_dir"
```

Before pushing changes that add fault probes, run:

```bash
cargo test -p resume-cli --test s71_fault_injection --locked
cargo test -p resume-daemon --test s81_daemon_kill --locked
./scripts/ci/guard-public-repo.sh
```
