# Performance Contract Files

This directory contains machine-readable contracts for the active performance,
GUI, and manual/Codex closed-loop goal.

These files are public and must stay free of raw resume text, raw query text,
local paths, candidate results, tokens, diagnostics packages, and model caches.

Files:

- `acceptance-matrix.toml`: performance, evidence-lane, and privacy redlines.
- `loop-state.schema.json`: schema for long-running goal state reports.
- `experiment-report.schema.json`: schema for redacted experiment reports.

These contracts do not run private benchmarks. They define what later local-only
benchmark and GUI/manual evidence must prove before the goal can be marked
complete.
