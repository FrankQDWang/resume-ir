# Current-Main Import Attribution Contract and Plan

Issue #270 is the sole owner of current-main installed-equivalent import
attribution; #217 remains an open umbrella. The primary lane is
`full_import_ocr_backlog`, with four independent milestones. The repair observed
remote main `0387506bc381aaa04ded9eb67cc213ffcd9c6797` and schema v35.
`git_head_sha` is an observed-base pin, not live-current authority.

Reconciliation allows 15 public paths and no benchmark/profile. After normal
merge and cleanup, #270 must freshly observe merged main and take one path:

```text
fresh main + configured roots + capability -> goal_authorized
missing configured roots -> blocked_permission
  blocker: blocked_missing_configured_private_roots
```

Attribution permits benchmark/profile execution and bounded redacted evidence,
but no production edit, guessed root, HOME scan, private publication, App/DMG,
installed acceptance or goal-complete claim. Legacy install stays with #217.

1. Align schema v35, permissions, lane and milestones.
2. Keep 554/555 immutable; reduce continuous 556+ events from the graph.
3. Use v2 only for #217 → #270 umbrella delegation; preserve generic v1
   terminal handoffs.
4. Generate state through the reducer; run negative, contract, privacy and exact
   budget gates; push the draft PR without merging.

Success is truthful policy/routing, reproducible state and a reachable
post-merge phase without private claims. Rollback reverts this contract slice
and closes #270 as `not planned`; it never touches main, private data or apps.
