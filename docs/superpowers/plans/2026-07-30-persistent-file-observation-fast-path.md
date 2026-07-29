# Persistent file-observation fast path plan

Issue: [#268](https://github.com/FrankQDWang/resume-ir/issues/268)

Plan-review verdict: approved after an explicit scope challenge. The
cross-crate scope is irreducible because one hypothesis requires crawler
observation, durable migration, import gating, aggregate metrics, and
publication regression coverage. Keep it one Draft PR and commit-sized slices.

## Build sequence

1. Make the PR #267 no-op publication test RED on second-scan content bytes,
   while preserving the middle-only mutation regression.
2. Add a metadata-only crawler mode and typed macOS high-resolution
   observation, with deterministic proof that discovery does not open content.
3. Add v35 source-occurrence observation storage and continuous v34-to-v35
   migration. Prove old stores invent no observations and new rows survive
   reopen.
4. Attempt the fast path before full read in sequential and parse-worker
   preparation. Reuse `exact_rerun_decision`; fail closed on every mismatch,
   audit deadline, processing-contract rejection, I/O error, or TOCTOU.
5. Persist only strongly verified successful occurrences. Revalidate again
   before occurrence/publication commit.
6. Add aggregate counters and focused tests for cross-restart no-op,
   restored-mtime middle mutation, audit rehash, replacement/TOCTOU, migration,
   overlapping occurrence ownership, and PR #267 publication stability.
7. Run a local ignored witness over 2,000 synthetic files. Gate correctness on
   I/O counters, not wall-clock thresholds.
8. Run focused crates, workspace checks, schema/contract checks, macOS local
   verification that does not invoke excluded platforms, rust-analyzer when
   available, and the public-repository privacy guard.

## Stop conditions

Do not publish a PR if a same-size restored-mtime mutation can hit the fast
path, a restart loses valid observations, a v34 store gains fabricated
observations, processing-contract drift is bypassed, a TOCTOU accepts stale
content, or public evidence contains private/path-level data.
