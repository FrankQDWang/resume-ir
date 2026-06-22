# Performance Contract Files

This directory contains machine-readable contracts for the active performance,
GUI, and manual/Codex closed-loop goal.

These files are public and must stay free of raw resume text, raw query text,
local paths, candidate results, tokens, diagnostics packages, and model caches.

Files:

- `acceptance-matrix.toml`: performance, evidence-lane, and privacy redlines.
- `loop-state.schema.json`: schema for long-running goal state reports.
- `experiment-report.schema.json`: schema for redacted experiment reports.
- `current-loop-state.json`: current public loop-state snapshot for this docs-hardening PR.
- `fixtures/valid/*.json`: synthetic positive fixtures for the CI contract guard.
- `fixtures/invalid/*.json`: synthetic negative fixtures that must fail the CI contract guard.

These contracts do not run private benchmarks. They define what later local-only
benchmark and GUI/manual evidence must prove before the goal can be marked
complete.

Run the public guard with:

```bash
python3 scripts/ci/check-performance-contracts.py
```

The guard is intentionally standard-library only and is wired into PR CI. It
checks schema versions, required scale gates, privacy booleans, W0 command
evidence, D10K/D100K/D1M completion semantics, GUI row-count limits, and
positive/negative fixtures.
