# Current-Main Import Attribution Contract Reconciliation Plan

Linked spec:
`docs/superpowers/specs/2026-07-30-current-main-import-attribution-contract.md`

## Assumption and ordered work

Remote `main`, live GitHub, and code are execution truth. This is public W0
contract work; no private input or product behavior is needed.

1. Observe main, issues/PRs, protection, schema, pins, and configured-root
   capability without reading private values.
2. Create one bounded owner: #270. Keep #217 as umbrella, #37 historical, and
   #268 closed.
3. Bind the active goal and matrix to #270, one lane, four independent
   milestones, schema v35, consistent private permission, and the missing-root
   terminal.
4. Record #217 → #270 and event 554, then derive the snapshot via the reducer.
5. Run reducer/compiler checks, performance/autonomous/loop gates, public
   guard, staged integrity, and diff review.
6. Commit/push, open a draft PR, append event 555, re-derive, reverify, and push
   the final state. Do not merge.

The exact 15-path budget is in `ACTIVE_GOAL.toml`; it contains no production,
workload, installer, or private-evidence path and remains under five commits
and 800 net lines.

## Success and rollback

Success means reproducible state points to #270 and the live draft PR, binds
current main/schema v35, and records
`blocked_missing_configured_private_roots` without a private claim.

Rollback reverts both commits and closes #270 as `not planned`; main, merged
PRs, user data, runtime state, and installed software remain untouched.
