# Current-Main Installed-Equivalent Import Attribution Contract

## Status and observed conflict

Approved for a W0 contract-reconciliation slice on 2026-07-30. Issue #270 is
the single execution owner. The observed remote `main` was
`0387506bc381aaa04ded9eb67cc213ffcd9c6797`, where code authority is metadata
schema v35. Future attribution must observe remote `main` again; this SHA is
event evidence, not permanent policy.

The prior checked-in surfaces conflicted:

- `GOAL.md` had moved to query work, but the active slice still named umbrella
  #217;
- authority docs and the matrix ended at v34 while code was v35;
- private benchmark work was allowed globally and forbidden by the slice;
- the snapshot retained F26, PR #249, and an old head after PRs #249, #267,
  and #269 merged; and
- #217 was an umbrella, #37 described older PR #35/D10K work, and #268 was
  closed, leaving no unique attribution owner.

## Authority decision

`GOAL.md` remains unchanged: query work is the product direction. A bounded
import-attribution prerequisite may first establish current-main behavior.
Issue #270 alone owns it, with `full_import_ocr_backlog` as the single primary
lane. It is evidence work, not a profile or optimization issue.

This W0 slice ends at a draft PR and executes no attribution workload.

## Effective permission and capability

Goal-level permissions are ceilings; the active slice is the effective owner
permission. Both now allow a later private benchmark only with explicitly
configured roots. No root is configured, so that later transition is:

```text
blocked_missing_configured_private_roots
```

The public W0 repair may complete. No runner may guess a root, scan `HOME`,
read private input, or turn synthetic evidence into a private claim.

## Milestones

The four non-substitutable milestones are:

1. `first_searchable`;
2. `keyword_ready`;
3. `embedding_complete`;
4. `ocr_backlog_full_import`.

Future evidence must bind freshly observed current main, runtime identity,
command shape, resource budget, and bounded redacted aggregates. Milestones
cannot close one another or claim query P95, W1, D10K, installed acceptance,
or goal completion.

## Derived state

The #217 → #270 routing lives in `perf/active-slice-transition.json`. Event 554
records the reconciliation; event 555 records the eventual draft PR. The
reducer starts from the main-reachable legacy snapshot, validates event order,
hashes, owner, transition, capability, and privacy facts, then produces
`perf/current-loop-state.json` byte-for-byte. Cosmetic direct edits fail check
mode. Live GitHub remains execution truth above the derived snapshot.

## Success and non-goals

Success requires one open owner (#270), schema v35 alignment, consistent
private permission, an explicit missing-root capability, reducer reproduction,
and green focused contract/privacy gates with a live draft-PR ledger.

This slice changes no import, OCR, embedding, index, classifier, query, or GUI
business code; runs no benchmark, profile, App/DMG, install, acceptance, or
soak; restores no Windows/Linux work; weakens no threshold/privacy gate; and
does not push main, bypass protection, merge, close the issue, or complete the
goal.

## Rollback

Revert the contract-only commits and close #270 as `not planned`. Do not alter
main, merged PRs, user data, runtime state, installed applications, or roots.
