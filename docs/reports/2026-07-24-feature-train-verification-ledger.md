# v0.1.3–v0.1.8 Feature-Train Verification Ledger

This is the auditable test authority for the feature train. A passing row may
be reused only while its input fingerprint and behavior boundary remain valid.
Unknown is not passed. A failure has no reuse value. Repairs rerun the failed
row and rows whose declared inputs changed; they do not reopen unrelated rows.

## Checkpoint

| Round | Scope | Commit | Result | Reuse |
| --- | --- | --- | --- | --- |
| P0-C01 | S810 daemon bootstrap/capability hard cut | `b2e1258dd694dcd5b54ae967ad89b3eb137acadf` | checkpoint committed; prior R07–R12 evidence remains in the S810 ledger | immutable base; do not rerun merely to start this train |

## Row schema

Each execution row must record:

- stable row and runner cell id;
- exact command and behavior boundary;
- git tree plus declared input fingerprint;
- start/end time, exit code and bounded receipt;
- `passed`, `failed`, `invalidated` or `not_run`;
- invalidating commit/files when applicable;
- DMG SHA-256, install receipt id and screenshot SHA-256 for native rows;
- privacy declaration and residual risk.

## P0 contract/version round

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| P0-01 | Product version has one manifest authority and Tauri uses its path form | `f36d7009b194981157cbe7c0f6a9de7fcda9330545dfd89e62772a864d84e69b` | passed | product version helper/config/build-plan changes |
| P0-02 | Install/reinstall/source-binding evidence derives the canonical version | `4404c062a4d1ecfbd25b072f8f83028af2309f7264bf124c6e5c9b9fcb84190e` | passed | lifecycle, source binding or deployment changes |
| P0-03 | Feature-train machine contract and mutation guards are exact | `bcb97b8b4d950ca6b1d054661e980d12e12cd30d6df3646d658b6b14029cd832` | passed | active goal, matrix, loop state, fixture pin or checker changes |
| P0-04 | Public boundary and changed-file whitespace are clean | `d2ca4f1c8ccc9ea236421aeeaf9818c0d0d1375c23e2c4e01846c1dfa504b29b` | passed | any later public-input change |

P0-01 commands passed on 2026-07-24: the exact product-version Node test,
affected DMG-plan/worktree-release/config Node tests, locked desktop Cargo
metadata and official Tauri `info` config resolution.

P0-02 commands passed on 2026-07-24: the exact source-binding,
release-deployment, install lifecycle, lifecycle journal and reinstall Node
tests. No Rust workspace, frontend suite or DMG build was replayed.

P0-03 commands passed on 2026-07-24: governance mutation tests, performance
contract checker after updating the two invalidated synthetic pins,
autonomous-goal checker, loop-state checker and parallel-runner self-test.

P0-04 commands passed on 2026-07-24: public repository guard and
`git diff --check`. The two user-owned research documents and generated
`node_modules/` remain outside the train.

## Version rounds

Rows for v0.1.3 through v0.1.8 are appended when each linked issue opens.
Every round begins with focused failing regressions, retains unaffected earlier
passes, and ends with an exact-commit installed native row before issue closure.

## Final round

The complete resumable parallel matrix, merged-main install and soak remain
`not_run` until v0.1.8 is merged. Their absence does not block an individual
feature issue from closing after its own installed acceptance, but it does
block #217 and release-ready claims.
