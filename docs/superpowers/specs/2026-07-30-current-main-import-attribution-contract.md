# Current-Main Import Attribution Contract

Issue #270 is the sole owner of current-main installed-equivalent import
attribution. #217 remains an open umbrella, #37 is historical and #268 is
closed. `full_import_ocr_backlog` is the primary lane; first-searchable,
keyword-ready, embedding-complete and OCR-backlog/full-import are independent
milestones. This is evidence work, not optimization, App/DMG, installed
acceptance, W1, D10K or goal completion.

The contract repair observed remote `main`
`0387506bc381aaa04ded9eb67cc213ffcd9c6797` and schema v35. That SHA is an
immutable observed base stored in the legacy-named `git_head_sha` pin, not
live-current authority. Future attribution must freshly observe merged remote
`main`; a committed snapshot or fixture cannot satisfy that precondition.

`contract_reconciliation` permits only the 15 public contract paths and
forbids benchmark/profile execution. After normal merge and branch cleanup,
the same issue takes exactly one path:

```text
configured roots + capability + fresh clean main
  -> authorize_current_main_import_attribution -> goal_authorized
missing roots
  -> block_current_main_import_attribution_missing_roots -> blocked_permission
     same_blocker_key=blocked_missing_configured_private_roots
```

The attribution phase permits benchmark/profile execution and bounded redacted
evidence only. It permits no production edit, guessed root, `HOME` scan, private
publication, App/DMG or installed acceptance. The old install transition is
owner-limited to #217. Existing `capture_baseline` can then run under #270 and
fresh authority; it may not create an empty profile issue.

`perf/active-slice-transition.json` v2 records
umbrella-to-bounded-owner delegation and explicitly denies a terminal claim for
#217. Events 554/555 are immutable. The reducer loads owner and transitions
from `ACTIVE_GOAL.toml`, accepts arbitrary continuous later versions, and
validates CAS/hash chain, owner applicability, permissions, privacy and v2
required-evidence keys. Its executable self-test proves 556 advances and
illegal transition, broken hash and version gap fail.

Success is one owner, schema v35, truthful routing, reproducible current
snapshot, a reachable post-merge attribution path, precise missing-root
terminal and green contract/privacy gates within 15 files and 800 net lines.
No attribution has run. Rollback reverts the contract commits and closes #270
as `not planned`; main, user data, runtime, roots and installed apps remain
untouched.
