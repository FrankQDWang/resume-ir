# Progress

This file tracks long-running production Goal execution against `GOAL.md`, the
system design docs, the execution docs, and this running evidence log. Obsolete
preliminary checklists are historical execution context only, not the
production-ready scope source.

## Execution Boundaries

- Repository: `/Users/frankqdwang/MLE/resume-ir`
- Data policy: S0-S96, S98, S101, S102, S103, S104, S107, S108, S111, S112, S114, S115, S116, S117, S118, S119, S120, S121, S124, S125, S126, S128, S129, S130, S131, S132, S133, S134, S135, S137, S138, S139, S140, S141, S142, S143, S144, S145, S146, S147, S148, S149, S150, S151, S152, S153, S154, S155, S156, S157, S158, S159, S160, S161, S162, S163, S164, S165, S166, S167, S168, S169, S170, S172, S173, S174, S175, S176, S177, S178, S179, S180, S181, S182, S183, S184, S185, S186, S187, S188, S189, S190, S191, S192, S193, S194, S195, S196, S197, S198, S199, S200, S201, S202, S203, S204, S205, S206, S207, S208, S209, S210, S211, S212, S213, S214, S215, S216, S217, S218, S219, S220, S221, S222, S223, S224, S225, S226, S227, and S228 used synthetic fixtures only.
  S97, S99, S100, S105, S106, S109, S110, S113, S122, S123, and S127 also used private local-only witnesses against anonymized temporary copies from a
  user-authorized local resume sample directory; no real resume data, filenames,
  paths, counts, raw text, or diagnostics were committed or uploaded.
  S136 also used a private local-only witness against anonymized temporary
  copies from the user-authorized local resume sample directory; no real resume
  data, filenames, paths, counts, raw text, or diagnostics were committed or
  uploaded.
  S171 also used private local-only PDF/Word witnesses against the
  user-authorized local resume sample directory through temporary witness
  copies; no real resume data, filenames, paths, counts, raw text, or
  diagnostics were committed or uploaded.
- Remote side effects: the public GitHub repository `FrankQDWang/resume-ir` was created during S67 after public-repo guard passed, and local `main` was pushed at `cc009da12c7c5753bbf3e66642fccee7db2ebeae`, then updated to `135f927` after S67 and `d0798fa` after S68. Main branch protection has been configured, draft PR #8 exists for the branch-protection progress record, and draft PR #9 exists for the current feature branch. No release, upload of runtime data, signing, or notarization has been performed.
- Slice rule: acceptance command passes before a slice is marked complete.

## Production Gap Audit

S42 included a read-only P0-P6 product gap audit using `GOAL.md`, the system
design docs, the execution docs, and this evidence log as scope sources. Deleted
obsolete preliminary files and checklists are not product scope.

- P0 architecture: Rust workspace, CLI/daemon entrypoints, SQLite metadata,
  task/status tables, loopback status IPC, an authenticated loopback import
  command IPC endpoint, CLI import-over-IPC submission, an authenticated
  loopback import cancellation command IPC endpoint, CLI cancel-over-IPC
  submission, authenticated loopback full-text search command IPC, CLI
  search-over-IPC submission, local and authenticated loopback redacted detail
  retrieval, doctor, diagnostics, a one-shot daemon import worker, a
  long-running daemon import scheduler, a daemon OCR worker loop for queued OCR
  jobs, and a daemon embedding worker loop for local vector snapshot generation
  exist. Import tasks have retry
  backoff, running-task heartbeat, stale-running task recovery, queued/
  retryable/running cancellation markers, cancelled-task status reporting, and
  cooperative cancellation checks during import scanning plus per-file import
  processing, status-pollable live import progress persisted in scan scope
  counters without path disclosure, and a polling import-rescan mode that
  requeues completed roots for background incremental import without printing
  root paths. The daemon can also stream authenticated redacted import progress
  snapshots over loopback IPC while import, OCR, or embedding worker loops run.
  The daemon now writes a local endpoint discovery
  manifest, and the CLI can use `--ipc auto` for status, import progress,
  import, cancel-import, search, and detail commands. A daemon full-text index
  maintenance worker can now force a local snapshot rebuild or run in a loop to
  repair non-ready snapshot roots. Public-repository governance now includes
  MIT licensing, CODEOWNERS, contribution/security policy, PR templates,
  GitHub Actions workflow definitions, dependency update configuration, local
  license checks, public push guardrails, and a PR-triggered hosted macOS plus
  Windows workspace build/test workflow. Missing or BLOCKED production
  control-plane work includes full service lifecycle proof, platform installer
  proof, and platform service validation.
- P1 import/search: directory scanning, DOCX/legacy `.doc` via local converter,
  text-layer PDF/UTF-8 and BOM-marked UTF-16 TXT parsing, cleaning, sectioning,
  polling background rescan for completed import roots, OS filesystem watcher
  integration that requeues completed roots through the existing durable import
  task path on local file changes, encrypted full-text published snapshot
  publish/recover with extended Windows transient read-open, snapshot-publish,
  and directory-cleanup filesystem retries, incremental full-text snapshot
  updates for import, OCR text indexing, and soft-delete removals, delete rebuild,
  redacted snippets, and an
  isolated local PDF/Word witness command
  that can use either an explicit root or the local-discovery root preset,
  anonymizes selected inputs, runs the real import path in a temporary data
  directory, can optionally run bounded OCR jobs through the existing local OCR
  worker path, reports redacted success/failure counters without stopping a
  budgeted witness on the first per-document OCR failure, prints only aggregate
  redacted output, can run a redacted internal full-text search probe without
  printing the private query or matched files, can run a redacted field-extraction
  aggregate probe without printing field values, filenames, or paths, and
  removes private witness data. S127, S136, and S171 reran the explicit-root private
  PDF/Word witness against the user-authorized local sample directory for import/
  search/field probes plus bounded OCR witnesses using local `tesseract` and
  `pdftoppm`; these runs removed private temporary data and no private evidence
  was committed or uploaded. Import scan errors are persisted and now surfaced
  as redacted kind/operation aggregate breakdowns through local status, doctor,
  and redacted diagnostics without path or path-digest disclosure. Missing production work includes
  production-grade PDF coverage, full
  legacy Word converter distribution and cross-platform proof, large-corpus
  proof, cross-platform watcher behavior proof, and large-corpus incremental
  update performance proof.
- P2 fields/dedupe/privacy: high-confidence rules for name, contacts/date/
  education/major/school-tier/company/title/skills/certs/years, persisted entity mentions,
  broadened high-signal major aliases,
  broadened high-signal location aliases,
  metadata-indexed field prefiltering before the full-text TopDocs cutoff,
  contact HMAC assignment, hash-only exact email/phone search filtering through
  local CLI and daemon IPC without raw contact or contact-hash output, candidate
  folding, and explicit best-effort local
  purge of tombstoned documents across metadata, obsolete full-text snapshots,
  full-text staging directories, vector records, current ingest jobs, and
  current OCR page-cache records exist. Contact hash key backup/restore now
  exists through local privacy CLI/API with passphrase-protected local backup
  files, redacted outputs, owner-only key file creation where supported, and
  restore refusal when a target key already exists. Metadata SQLCipher key
  backup/restore now also exists through local CLI/API with passphrase-protected
  local backup files, wrong-passphrase restore rejection without key creation,
  restored-key reopen proof for copied encrypted metadata, redacted outputs, and
  restore refusal when a target key already exists. Metadata SQLCipher key
  rotation now exists through local CLI/API, rekeys the encrypted SQLite
  database, replaces the owner-only local key file, proves the old key can no
  longer open the database, and keeps outputs free of key material and local
  paths; forensic erase proof remains absent. A labeled
  field-quality evaluator and gate now score precision/recall/F1 from JSONL
  samples without emitting raw text, sample IDs, paths, or field values.
  Private business labeled field-quality release evidence is now accepted only
  as strict redacted local aggregate JSON with dataset/annotation manifest
  digests, explicit false raw-data/path/value/sample-ID booleans, a fixed field
  taxonomy, and production field metrics for name, email, phone, school,
  school_tier, degree, major, company, title, location, skill, certificate, date
  ranges, and years experience.
  Soft-dedupe scoring now compares
  same-name profiles with bounded non-contact evidence overlap and surfaces
  redacted suspected-duplicate hints in local CLI and daemon search results
  without low-confidence candidate folding. A local candidate-review CLI can
  now list redacted same-name soft-dedupe suggestions, explicitly merge two or
  more unassigned searchable versions into a manual candidate for default search
  folding, split that candidate back into independent searchable versions, and
  list redacted multi-contact conflicts when email and phone hashes point at
  different candidates without printing contact values, contact hashes, names,
  schools, companies, local paths, or resume text. A
  labeled dedupe-quality evaluator
  and gate now score precision/recall/F1 from JSONL profile pairs without
  emitting names, schools, companies, skills, sample IDs, document IDs, paths,
  or raw resume text. Private business labeled dedupe-quality release evidence
  is now accepted only as strict redacted local aggregate JSON with
  dataset/annotation manifest digests, explicit false raw-data/path/profile-
  value/sample-ID/document-ID booleans, a fixed dedupe taxonomy, and aggregate
  pair counts plus quality metrics.
  The local PDF/Word witness can now
  run a redacted field-extraction probe that
  verifies persisted field mentions by aggregate field type only, without
  selecting, printing, or committing raw/normalized field values, filenames,
  paths, private queries, raw text, or diagnostics. A SQLCipher-backed
  `MetaStore::open_encrypted` path exists and is tested with a synthetic
  encrypted file that cannot be read without the correct key. The default
  CLI/daemon data-dir metadata path now creates an owner-only local metadata
  SQLCipher key under `metadata-secrets/`, opens `metadata.sqlite3` through
  SQLCipher, migrates an existing plaintext `metadata.sqlite3` in the default
  data-dir path to SQLCipher while preserving synthetic rows and removing the
  plaintext file from the default path, reports metadata and OCR cache
  `sqlcipher` in doctor/redacted diagnostics, and keeps contact-key permission
  failures isolated from metadata-key availability. Published full-text
  snapshots are now written as
  local encrypted XChaCha20-Poly1305 envelopes with an owner-only local snapshot
  key; the `snapshots/<name>` artifact no longer contains plaintext Tantivy
  files or stored resume text, while active open, fallback recovery, CLI search,
  daemon full-text search, and diagnostics continue to work through a private
  temporary decrypt-and-open path. OCR page-cache records stay inside the
  default SQLCipher metadata store; doctor/export report that boundary, and a
  synthetic OCR worker test proves raw default `metadata.sqlite3` lacks the
  SQLite header, OCR text token, and engine-profile marker after a cache write.
  Deleted-document purge now reports and proves removal of persisted OCR word
  boxes from purged OCR page-cache rows, covering the current bbox/PII evidence
  surface for OCR cache cleanup.
  Deleted-document purge also reports and proves removal of persisted embedding
  job specs linked to purged ingest jobs for deleted documents, keeping vector
  worker queue metadata auditable during cleanup.
  Deleted-document purge now also removes import tasks whose roots contain
  deleted documents and no visible documents, cascading scan scopes, scan
  errors, and cancellations so emptied import-root paths are cleared from
  current metadata while roots with visible documents are retained.
  Import-root purge matching now normalizes local file URI prefixes, Windows
  verbatim canonical roots, drive-letter case, and path separators before
  comparing roots with document paths, preserving the same redacted purge output
  on hosted Windows.
  Certificate extraction now treats certificate section headers as bounded
  context, suppresses header values, handles labeled certificate lines with
  ASCII or fullwidth colons, and extracts high-signal certificate aliases such
  as PMP, CKA, CISSP, CFA Level I, AWS/Azure/Kubernetes certifications, and CPA
  with canonical normalized values and span evidence that import persists.
  Certificate extraction now also recognizes AWS Security Specialty, Google
  Professional Data Engineer, and CCNA aliases while preventing those known
  certificate lines from being misclassified as titles.
  Certificate extraction now also recognizes CKS, Certified Kubernetes Security
  Specialist, HashiCorp Certified Terraform Associate, Terraform Associate,
  Google/GCP Associate Cloud Engineer, AZ-204/Azure Developer, and RHCSA
  aliases with canonical normalized values that import persists and search
  filters parse.
  Skill extraction now also treats skill section headers as bounded context,
  suppresses header values, handles labeled skill lines with ASCII or fullwidth
  colons, and extracts high-signal aliases such as TypeScript, PostgreSQL,
  K8s/Kubernetes, Go/Golang, Redis, React, and Node.js with canonical normalized
  values and span evidence that import persists.
  Skill extraction now also recognizes expanded high-signal data, ML, and
  frontend aliases such as Spark, Hadoop, Airflow, TensorFlow, PyTorch,
  scikit-learn, Vue.js, Angular, and GraphQL, while avoiding `.js` suffixes
  being separately extracted as JavaScript.
  Skill extraction and filtering now also recognize high-signal cloud, data,
  and DevOps aliases such as AWS/Amazon Web Services, Azure, GCP/Google Cloud
  Platform, Terraform, Ansible, Jenkins, GitLab CI, Kafka, Flink,
  Elasticsearch, MongoDB, and Snowflake, and rank-fusion normalizes common
  user-input aliases such as K8s, Golang, Postgres, NodeJS, React.js, TS,
  sklearn, and GitLab CI/CD to the same canonical skill keys used by persisted
  profiles.
  Candidate-name field filtering now matches persisted `name` entity mentions
  through direct CLI search and CLI/daemon IPC, with metadata prefiltering
  before the full-text top-k cutoff and normalized name profile matching.
  Date-range extraction now also handles Chinese year/month ranges such as
  `2020年1月 - 2024年3月`, normalizes them to the existing `YYYY-MM/YYYY-MM`
  schema, preserves exact span evidence, and keeps years-experience derivation
  plus import persistence working.
  Date-range extraction now also handles open-ended present/current ranges such
  as `2020年1月 - 至今`, `Jan 2021 - Present`, and
  `2022.03 - Current`, normalizes them to `YYYY-MM/PRESENT`, preserves exact
  span evidence, and derives years-experience against the current local month.
  Phone extraction now also handles China mainland mobile numbers with or
  without `+86`/`0086` and common separators, normalizing them to E.164
  `+86...` before privacy-preserving import persists redacted phone mentions.
  Company/title extraction now strips common English and Chinese labels such as
  `Company:`, `Title:`, `公司：`, and `职位：` from the field evidence span,
  normalizes company suffixes such as `Co., Ltd.`, `Pte Ltd`, `GmbH`,
  `有限公司`, and `有限合伙`, avoids treating unrelated labeled lines as company
  evidence, and keeps import persistence aligned with the stripped values.
  Title extraction now also maps broader high-signal English and Chinese role
  aliases for frontend, full-stack, machine-learning, data-science, DevOps, QA,
  engineering-manager, and solutions-architect families while avoiding
  certificate-line title false positives.
  Title extraction now also maps platform, security, mobile, and business
  analyst role families, and suppresses known certificate aliases such as Google
  Professional Data Engineer from title extraction.
  Location extraction now handles explicitly labeled English and Chinese
  location lines such as `Location:`, `Base:`, and `所在地：`, canonicalizes
  common and high-signal city aliases including Bay Area/SF, New York/NYC,
  Hong Kong, Singapore, Chongqing, Tianjin, Xi'an, Changsha, Hefei, Qingdao,
  Taipei, major North American technology hubs, and several international
  hubs, extracts city-level evidence from explicitly labeled address lines
  without persisting full street-address spans for the current high-signal
  alias set, handles case-insensitive English address city substrings and
  Chinese district-style address city substrings such as `北京海淀区` and
  `深圳南山区`, persists span-backed `location` entity mentions, and supports
  location filtering through direct CLI search plus CLI/daemon IPC with
  metadata prefiltering before full-text top-k truncation.
  School/degree extraction now strips common English and Chinese education
  labels such as `School:`, `Degree:`, `学校：`, and `学历：` from field evidence,
  normalizes school whitespace/case, maps degree aliases such as MSc, BSc,
  MEng, M.Tech, MPhil, BSc, B.Tech, B.E., PhD, `博士研究生`, and
  `硕士研究生` to canonical degree values, and avoids duplicate generic degree
  matches inside labeled degree evidence.
  Degree extraction now limits unlabeled degree aliases to education context
  while still accepting explicitly labeled degree lines anywhere, preventing
  skill/product phrases such as `MS SQL` from being persisted as a master's
  degree.
  School tier extraction now recognizes explicit `985`, `211`, `C9 League`,
  `Project 985/211`, `双一流`, Double First-Class, Ivy League, Russell Group,
  overseas, and regular-school evidence inside bounded education/school
  context, persists canonical `school_tier` mentions, and supports
  `--school-tier` search filtering through direct CLI and daemon IPC paths.
  Missing production work
  includes broader dictionaries and normalization beyond the current
  high-signal certificate/skill/title aliases, remaining labeled school/degree
  forms and degree aliases, school-tier inference beyond explicit aliases, remaining address forms beyond
  current high-signal labeled address city substring extraction,
  Chinese explicit/open-ended date ranges, China mobile phone formats, and
  labeled company/title forms, real
  business labeled field and dedupe quality datasets/results, remaining future
  non-cache PII surface purge coverage, and forensic erase proof.
- P3 semantic/hybrid: local embedding command protocol, persisted vector
  snapshot, in-memory linear KNN, persistent HNSW ANN query backend, RRF
  helpers, embedding worker, model/dimension-scoped durable per-version
  embedding jobs, model-scoped vector query isolation, section-level vector
  inputs, CLI semantic/hybrid query execution, and local model-pack manifest
  validation with checksum plus license-reviewed gates now exist. The daemon
  can now execute a configured local embedding command in one-shot or
  long-running worker mode, persist a vector snapshot while serving status IPC,
  skip already completed version jobs across daemon restarts, re-embed
  completed versions when the configured model id or dimension changes, and
  write document plus section vectors inside one version job. Persistent vector
  search now rebuilds a process-local HNSW ANN graph from the durable vector
  snapshot, preserves model-scoped graph isolation, and reports the ANN backend
  through redacted CLI status, doctor, and diagnostics output without emitting
  vectors or local paths. Persistent vector mutations now use a stable
  sidecar file lock, reload the latest snapshot while holding that lock, merge
  the current mutation, and refresh local HNSW state before returning, preventing
  stale CLI/daemon writers from overwriting each other's vector updates or
  tombstones. Persistent vector snapshots are now written as local encrypted
  XChaCha20-Poly1305 envelopes with an owner-only local snapshot key, so the
  `vector.snapshot` artifact no longer stores vector IDs, document IDs, model
  IDs, or float values as plaintext while reopen, inspection, semantic search,
  daemon embedding workers, and diagnostics continue to work.
  A labeled vector-quality evaluator and gate now score recall@k, MRR, NDCG@k,
  and zero-recall queries from JSONL samples using the local embedding command
  protocol without emitting raw queries, candidate text, sample IDs, candidate
  IDs, vectors, command paths, or resume paths.
  Private business labeled vector-quality release evidence is now accepted only
  as strict redacted local aggregate JSON with dataset, annotation, and model
  manifest digests, explicit false raw-query/candidate-text/resume-path/sample-
  ID/candidate-ID/vector booleans, a fixed vector taxonomy, and aggregate
  recall/MRR/NDCG metrics. Release-readiness now keeps stable release blocked
  until representative private business labeled vector-quality evidence exists.
  Missing or BLOCKED work includes licensed model selection/download/
  distribution, real business semantic quality datasets/results, real ANN
  recall/latency proof at large corpus scale, and real performance proof.
- P4 OCR: OCR_REQUIRED routing, durable OCR jobs, pause/resume control, page
  cache schema, local OCR command client, local PDF page-render command
  protocol, local Poppler `pdftoppm` PDF renderer adapter, local Tesseract OCR
  adapter with TSV confidence and word-box parsing, timeout/cancel/temp cleanup,
  page-count detection for scanned PDFs, multi-page OCR fan-out, per-page cache
  entries with persisted OCR word boxes, aggregate OCR text indexing, a
  per-document OCR page-count backpressure guard, redacted page-budget
  remediation diagnostics, and redacted local OCR runtime availability
  diagnostics exist. The CLI and daemon can now claim queued OCR jobs, reject
  scanned PDFs above the configured local page budget before renderer/OCR
  invocation, persist a safe `ocr_page_budget_exceeded` job failure kind,
  surface aggregate page-budget blocks through local status, doctor, redacted
  diagnostics, and daemon status IPC, report local `pdftoppm`, Tesseract, and
  requested Tesseract OCR language-pack availability without binary paths or
  dumping the full local language list, render valid PDF pages through local `pdftoppm` or a configured renderer,
  execute local OCR commands or local Tesseract on the rendered image, persist
  cache entries for each page, index combined OCR text with page count, honor
  persistent pause state, keep serving status IPC while OCR runs, and exercise
  OCR from the local PDF/Word witness command with redacted completed, blocked,
  and per-document failure aggregate output. The benchmark runner can now
  exercise synthetic OCR page throughput through the existing local command or
  Tesseract OCR clients and gate redacted page-latency/pages-per-second reports
  with explicit synthetic opt-in. Private real-corpus OCR throughput release
  evidence is now accepted only as strict redacted local aggregate JSON with
  dataset, OCR runtime, renderer, and language-pack manifest digests, explicit
  false raw OCR text/page image/resume-path/document-ID/page-ID/command-path
  booleans, and aggregate latency/throughput metrics; release-readiness now
  keeps stable release blocked until representative private real-corpus OCR
  throughput evidence exists. OCR runtime diagnostics now support combined
  Tesseract language requests such as `eng+chi_sim` by checking each requested
  language pack without dumping the full local language list; this machine also
  verified a local Apache-2.0 `tesseract-lang` install and `eng+chi_sim`
  availability through redacted doctor output. The CLI and daemon OCR workers
  now also preflight requested Tesseract language packs on cache misses before
  invoking the renderer or OCR engine, persist redacted retryable
  `LanguageUnavailable` page-cache failures when a requested pack is absent,
  and avoid printing command paths, requested language names, local language
  lists, or OCR engine stderr in that blocked path. Local status, doctor,
  export-diagnostics, daemon status IPC, and CLI status-over-IPC now surface a
  redacted aggregate `ocr_language_unavailable` blocker count and remediation
  message for these failed OCR jobs without exposing requested language names or
  local runtime paths.
  OCR runtime manifest validation now exists for reviewed local OCR runtime
  packs: `resume-cli ocr validate-manifest` verifies the local Tesseract/
  renderer/language-pack artifact checksums and reviewed license metadata
  without printing local paths, runtime bytes, language-pack bytes, or complete
  digests. This is governance evidence only and does not approve distribution.
  Deleted-document purge now removes
  current OCR jobs and current OCR page-cache entries that are no longer shared
  by visible documents. Missing or BLOCKED work includes final OCR/renderer
  distribution approval, full non-English OCR quality and language-pack distribution policy, full-library scanned
  resume OCR proof beyond bounded local witness budgets, actual representative
  private real-corpus OCR throughput runs, and Windows/macOS
  validation.
- P5 packaging/platform: not production-ready. A local CLI service lifecycle
  now writes, reports, removes, starts, stops, and reports runtime state for a
  macOS user LaunchAgent without CLI path disclosure. A local-only macOS
  LaunchAgent witness has installed a temporary daemon, observed `not_loaded`
  before start, observed `running` after start, read daemon status through
  authenticated IPC auto-discovery, stopped the daemon, observed `not_loaded`
  after stop, uninstalled the LaunchAgent, and removed temporary local data.
  Hosted macOS and Windows workspace build/test checks now run for pull
  requests through Platform CI. A release dry-run workflow can now generate and
  upload a redacted `release-artifacts.json` checksum manifest for locally built
  release binaries and a redacted SPDX 2.3 `release-sbom.json` from locked Cargo
  metadata without recording local paths or runtime data. The tracked GitHub
  Actions workflows now use the current Node 24-compatible major versions for
  checkout and artifact upload actions, with the workflow policy guard rejecting
  the deprecated Node 20 action majors. The updated hosted release dry-run has
  executed successfully on the feature branch and produced the redacted
  `release-dry-run` artifact. A local macOS-only unsigned pkg/dmg dry-run script
  and guard now generate and validate synthetic pkg/dmg artifacts plus a redacted
  package manifest. The Release workflow is now wired to build release binaries
  on hosted macOS, run the unsigned macOS package dry-run, verify the dmg with
  `hdiutil`, enforce the public artifact boundary, and upload a
  `macos-package-dry-run` artifact. The updated hosted Release workflow has
  executed successfully on the feature branch and produced non-expired
  `release-dry-run` and `macos-package-dry-run` artifacts. These dry-runs do
  not sign, notarize, create a GitHub Release, install, upgrade, uninstall, or
  validate Gatekeeper behavior. A Windows PowerShell MSI dry-run script,
  local non-Windows guard, release runbook wiring, and Release workflow hosted
  Windows package job are now present. The hosted Windows MSI dry-run exposed
  that WiX `7.0.0` requires OSMF EULA acceptance on GitHub-hosted runners, so
  the dry-run tool pin was moved to WiX `6.0.2`; a later hosted Release run
  proved the Windows MSI dry-run and artifact upload path. The same later run
  exposed a transient hosted macOS `hdiutil verify` resource-unavailable race
  immediately after DMG creation, so the macOS DMG verification path now uses a
  bounded retry helper. The full hosted Release workflow has now executed
  successfully with release manifest/SBOM, macOS package dry-run, and Windows
  MSI dry-run artifacts uploaded as non-expired workflow artifacts. The CLI
  service lifecycle surface now has an explicit Windows Service dry-run mode
  for install, status, start, stop, and uninstall command-plan evidence without
  touching LaunchAgent files or exposing local paths, and release-readiness now
  tracks Windows service lifecycle as a separate blocked release criterion.
  Signing, notarization, Windows service validation, real upgrade/uninstall runs,
  GitHub Release upload, and platform installer/service
  validation remain absent, not complete, or externally blocked by platform
  credentials/runners.
- P6 performance/stability: synthetic benchmark runner, status/doctor/export
  diagnostics, redacted resource telemetry for the data-disk volume, current
  process memory, CPU cores, OCR page-budget remediation, and OCR runtime
  availability, snapshot fallback, explicit obsolete
  full-text snapshot and staging cleanup for deleted-document purge, safe fault
  simulation for disk-space budget, permission-denied probes, file-lock
  contention probes, metadata migration failure probes against synthetic broken
  scratch databases, daemon-kill/restart probes against configured daemon
  binaries, OCR command crash probes, model-checksum probes against controlled
  local model artifacts, local model-pack manifest validation, targeted fault
  tests, persistent vector snapshot writer-lock protection against stale
  concurrent writers, hosted-Windows full-text snapshot read-open/publish/
  commit/directory-cleanup retry hardening, metadata-rebuild fallback when import
  encounters an unreadable active full-text snapshot,
  local-only macOS LaunchAgent start/stop witness evidence, local-only
  production runbooks, a runbook CI policy guard, a workflow policy guard, and
  release artifact manifest plus SBOM policy guards, GitHub Actions runtime
  compatibility guards, hosted release dry-run execution evidence, and a
  synthetic OCR throughput benchmark/gate exist. Local runtime query telemetry
  now records bounded redacted aggregate samples for successful CLI and daemon
  searches, and reports query latency P50/P95/P99 plus last result count through
  local status, daemon status IPC, doctor, and redacted diagnostics without
  storing or printing raw query text. Safe local fault simulations now also
  cover battery-mode degradation and external-drive disconnect recovery
  messaging, and doctor/redacted diagnostics advertise those fault hooks while
  the release-readiness gate keeps real hardware drills blocked.
  Active import daemon kill/restart proof now exists for a running import task:
  the foreground import scheduler can be killed after claiming a task, then a
  restarted worker can explicitly lower stale-running recovery and retry-backoff
  thresholds for local drills, recover the interrupted task, complete import,
  and produce searchable full-text results without printing local paths.
  Hosted Windows full-text snapshot tests now also route staging-orphan fixture
  writes through the existing transient Windows lock retry helper, covering the
  observed `os error 33` setup race in synthetic snapshot tests.
  Hosted Windows full-text snapshot publishing now also routes snapshot archive
  file reads, encrypted snapshot envelope reads, and encrypted-header probes
  through the same bounded transient Windows lock retry policy, covering the
  observed `os error 33` read race during incremental snapshot tests.
  Hosted Windows full-text commits now also route Tantivy writer commits through
  the bounded transient Windows access-denied retry policy, covering the
  observed `os error 5` commit race during synthetic full-text snippet tests.
  Hosted Windows daemon scheduler tests now avoid rerunning metadata migrations
  from the test process after foreground daemon readiness has already proved the
  store is migrated, reducing live worker-loop SQLCipher/SQLite DDL contention
  in the startup-queue test harness.
  The benchmark runner now has explicit synthetic query, synthetic OCR
  throughput, labeled vector-quality, private real-corpus query release-
  evidence, private business vector-quality release-evidence, private business
  field-quality release-evidence, and private business dedupe-quality release-
  evidence gates, plus private real-corpus OCR throughput release-evidence
  gates; query, OCR, and vector smoke gates are wired into PR and nightly
  workflows. Synthetic query benchmark document generation is now streamed into
  the full-text index instead of pre-collecting the full synthetic document set
  in memory, and redacted synthetic benchmark reports include
  `generation_mode: "streaming"` for runbook audit. Synthetic runs must opt in
  with `--allow-synthetic` and cannot prove 100k/1M production performance or
  representative OCR throughput.
  Private real-corpus query
  reports are accepted only as strict redacted local aggregate JSON with local
  corpus/query-set digests, explicit hot-index hybrid query evidence
  (`query_mode: hybrid`, `fulltext+field+vector+rrf`, no hot-path OCR, parsing,
  or heavy model inference), and no raw text, paths, queries, filenames, or
  sample identifiers. Private business field-quality reports are accepted only
  as strict redacted aggregate JSON with dataset/annotation manifest digests and
  production field metrics, and cannot include raw text, paths, field values, or
  sample identifiers. Private business dedupe-quality reports are accepted only
  as strict redacted aggregate JSON with dataset/annotation manifest digests,
  aggregate pair counts, and quality metrics, and cannot include raw text,
  paths, profile values, sample identifiers, or document identifiers.
  Private business vector-quality reports are accepted only as strict redacted
  aggregate JSON with dataset/annotation/model manifest digests, aggregate
  recall/MRR/NDCG metrics, and explicit proof that raw queries, candidate text,
  resume paths, sample identifiers, candidate identifiers, and vectors are not
  included. Private real-corpus OCR throughput reports are accepted only as
  strict redacted aggregate JSON with dataset/OCR-runtime/renderer/language-pack
  manifest digests, aggregate latency and throughput metrics, and explicit
  proof that raw OCR text, page images, resume paths, document identifiers, page
  identifiers, and command paths are not included. The release-readiness gate
  and release blockers runbook now include vector quality and OCR throughput as
  separate blocked release criteria instead of relying only on the model and OCR
  license/distribution blockers.
  Missing or BLOCKED work includes actual 100k/1M real-corpus benchmark runs,
  real-corpus nightly/release performance evidence, real business labeled field,
  dedupe, and vector datasets/results, actual representative private
  real-corpus OCR throughput runs, licensed model selection/distribution, real
  semantic/vector quality datasets/results, destructive
  service-level kill/actual ENOSPC fault injection, actual battery/external-
  drive hardware fault drills, Windows/macOS validation, and cross-platform
  performance evidence.

## Slice Status

| Slice | Status | Evidence | Blockers |
|---|---|---|---|
| S0 | Complete | Git initialized; initial design baseline committed as `43e3d1c`; acceptance showed only S0 files pending before commit. | None |
| S1 | Complete | `cargo metadata --no-deps`, `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace` passed. | None |
| S2 | Complete | `cargo fmt --check`, `cargo test -p core-domain`, `cargo test -p config`, and `cargo clippy -p core-domain -p config --all-targets -- -D warnings` passed after review-fix changes. | None |
| S3 | Complete | `cargo fmt --check`, `cargo test -p meta-store`, and `cargo clippy -p meta-store --all-targets -- -D warnings` passed. | None |
| S4 | Complete | `cargo fmt --check`, `cargo test -p meta-store`, `cargo test -p resume-cli`, `cargo test -p resume-daemon`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S4 CLI/daemon smoke commands passed. | None for the S4 slice; product search, indexing, OCR, embeddings, IPC, diagnostics, and cross-platform verification remain not complete. |
| S5 | Slice complete | `cargo fmt --check`, `cargo test -p fs-crawler`, and `cargo clippy -p fs-crawler --all-targets -- -D warnings` passed. | None for the S5 slice; product import execution, document parsing, indexing, OCR, and query closure remain not complete. |
| S6 | Slice complete | `cargo fmt --check`, `cargo test -p parser-common`, `cargo test -p parser-docx`, `cargo test -p parser-pdf`, and `cargo clippy -p parser-common -p parser-docx -p parser-pdf --all-targets -- -D warnings` passed. | None for the S6 slice; OCR execution, text cleaning, indexing, search, and S7+ remain not complete. |
| S7 | Slice complete | `cargo fmt --check`, `cargo test -p text-normalizer`, `cargo test -p sectionizer`, `cargo test -p extractor-rules`, and `cargo clippy -p text-normalizer -p sectionizer -p extractor-rules --all-targets -- -D warnings` passed. | None for the S7 slice; import execution, indexing, search, OCR execution, embeddings, and S8+ remain not complete. |
| S8 | Slice complete | `cargo fmt --check`, `cargo test -p index-fulltext`, `cargo test -p search-planner`, `cargo run -p resume-cli -- search "Java 支付"`, and `cargo clippy -p index-fulltext -p search-planner -p resume-cli --all-targets -- -D warnings` passed. | None for the S8 slice; import execution, OCR execution, embeddings, vector search, and S9+ remain not complete. |
| S9 | Slice complete | `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S9 import/status/search smoke commands passed. | None for the S9 slice; OCR execution, embeddings, field filtering, packaging, and production-scale performance remain not complete. |
| S10 | Slice complete | `cargo fmt --check`, `cargo test -p extractor-rules`, `cargo test -p rank-fusion`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S10 filtered search smoke command passed. | None for the S10 slice; filters are recall-then-filter over the top full-text candidates, and OCR/embeddings/production-scale performance remain not complete. |
| S11 | Slice complete | `cargo test -p embedder`, `cargo test -p index-vector`, `cargo test -p rank-fusion`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace` passed. | None for the S11 skeleton; deterministic embedder and in-memory vector index are test-only scaffolding, not product semantic search or performance claims. |
| S12 | Slice complete | `cargo test -p ocr-client`, `cargo test -p ingest-scheduler`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace` passed. | None for the S12 skeleton; OCR remains disabled by default and no real OCR worker, DB page queue, or query-path OCR was added. |
| S13 | Slice complete | `cargo test --workspace`, `cargo run -p resume-cli -- doctor`, and `cargo run -p resume-cli -- export-diagnostics --redact` passed. | None for the S13 skeleton; query smoke is a small current-run measurement only, and fault handling is simulated/diagnostic rather than a destructive daemon kill or disk-fill exercise. |
| S14 | Product slice complete | `cargo fmt --check`, `cargo test -p meta-store`, `cargo test -p import-pipeline`, `cargo test -p resume-cli --test s8_search_cli`, `cargo test -p resume-cli --test s14_delete_search`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S14 import/search/delete/search CLI smoke passed. | None for this soft-delete/default-search slice; physical deletion, vector-index deletion, queue cancellation, atomic snapshot rollback, and complete audit retention remain not complete. |
| S15 | Product slice complete | `cargo fmt --check`, `cargo test -p meta-store`, `cargo test -p import-pipeline`, `cargo test -p resume-cli --test s15_ocr_handoff`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S15 import/status/doctor/search/export-diagnostics CLI smoke passed. | None for this durable OCR handoff slice; real OCR execution, page rendering/cache, pause/resume worker recovery, searchable OCR text indexing, bbox/confidence persistence, and deleted-document queue cancellation remain not complete. |
| S16 | Product slice complete | `cargo fmt --check`, `cargo test -p extractor-rules`, `cargo test -p meta-store`, `cargo test -p import-pipeline`, `cargo test -p resume-cli --test s16_persisted_fields`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S16 import/status/filtered-search/export-diagnostics CLI smoke passed. | None for this persisted-field-mention slice; Tantivy field fast fields, DB/index pre-filtering before recall, candidate soft dedupe/folding, contact hash indexes, field F1 benchmark, and production-scale field performance remain not complete. |
| S17 | Product slice complete | `cargo fmt --check`, `cargo test -p benchmark-runner`, `cargo clippy -p benchmark-runner --all-targets -- -D warnings`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S17 `resume-benchmark synthetic-query` CLI smoke passed. | None for this synthetic benchmark-runner slice; real 10万/100万 corpus runs, real business query sets, OCR/vector benchmarks, RSS/CPU/disk telemetry, cross-platform benchmark evidence, and P95 target pass/fail gates remain not complete. |
| S18 | Product slice complete | `cargo fmt --check`, `cargo test -p resume-cli --test s18_candidate_folding`, `cargo test -p resume-cli --test s8_search_cli`, `cargo test -p resume-cli --test s10_search_filters`, `cargo test -p resume-cli --test s14_delete_search`, `cargo test -p resume-cli --test s16_persisted_fields`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace` passed. | None for this assigned-candidate search folding slice; automatic candidate assignment, contact-hash dedupe, merge confidence, candidate table/indexes, low-confidence suspected-same-person hints, and version expansion UI remain not complete. |
| S19 | Product slice complete | `cargo fmt --check`, `cargo test -p core-domain contact_hash_only_hydrates_external_keyed_digests`, `cargo test -p meta-store`, `cargo test -p import-pipeline`, `cargo test -p resume-cli --test s16_persisted_fields`, `cargo test -p resume-cli --test s18_candidate_folding`, `cargo clippy -p core-domain -p meta-store --all-targets -- -D warnings`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace` passed. | None for this candidate persistence and hashed-contact assignment slice; import-time keyed hashing, key management/rotation, automatic candidate assignment from extracted fields, candidate merge review, foreign-key migration enforcement, low-confidence duplicate hints, and version expansion UI remain not complete. |
| S20 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this loopback status IPC slice; final production IPC remains not complete: no gRPC/UDS/Named Pipe transport, authenticated command API, import/search IPC endpoints, service lifecycle integration, Windows IPC validation, or remote access support. |
| S21 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p privacy`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext snippets_redact_contact_values_near_query_matches`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p privacy -p import-pipeline -p index-fulltext -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this import-time keyed-contact assignment slice; key rotation, encrypted metadata, candidate merge review UI, low-confidence duplicate hints, multi-contact conflict workflow, key backup/recovery, and full dedupe quality metrics remain not complete. |
| S22 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this `entity_mention` contact redaction slice; SQLite encryption, `resume_version.raw_text`/`clean_text`, full-text index contact storage, physical free-page/WAL purge, SQLCipher, key rotation/backup, diagnostic key health, and full PII audit remain not complete. |
| S23 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this future full-text index contact-redaction slice; existing Tantivy segments, SQLite `resume_version.raw_text`/`clean_text`, SQLCipher, physical deletion/free-page/WAL purge, hash-based contact search, and full PII audit remain not complete. |
| S24 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p privacy`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p privacy -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this contact-hash key diagnostics slice; key rotation, backup/recovery, SQLCipher, full diagnostic package audit, and complete PII audit remain not complete. |
| S25 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p import-pipeline -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the S25 synthetic import/status/search/doctor/export-diagnostics CLI smoke passed. | None for this active full-text snapshot publish and diagnostics slice; last-good fallback after active pointer corruption, old snapshot GC, physical segment purge, vector snapshotting, SQLCipher, full disk-full/kill-daemon fault injection, and cross-platform atomic rename validation remain not complete. |
| S26 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this read-path full-text snapshot last-good fallback slice; snapshot GC/retention, active-pointer repair, staging cleanup, physical purge, vector fallback, real disk-full/kill-daemon fault injection, and cross-platform filesystem validation remain not complete. |
| S27 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p fs-crawler -p import-pipeline -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this local discovery-profile slice; default whole-machine root presets, multi-root CLI/UI, progress/cancel/budget limits, persisted scan-profile schema, symlink cycle protection if follow-symlink is later enabled, real local resume witness runs, and cross-platform root/exclusion validation remain not complete. |
| S28 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p import-pipeline -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this multi-root CLI import slice; automatic default root presets, persisted scan scope metadata, import progress/cancel, per-root partial-failure UX, true atomic multi-root transaction semantics, real local resume witness runs, and cross-platform root path validation remain not complete. |
| S29 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client`, `/Users/frankqdwang/.cargo/bin/cargo test -p ingest-scheduler`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p ingest-scheduler --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this local OCR command execution client slice; concrete OCR engine selection/license/install, PDF page rendering, OCR cache persistence, worker queue integration, searchable OCR text indexing, bbox persistence, full pause/resume worker recovery, real scanned-resume witness run, and Windows command execution validation remain not complete or BLOCKED. |
| S30 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this SQLite OCR page cache slice; PDF page rendering, OCR worker queue integration, cache lookup/write from actual OCR execution, bbox storage, full-text indexing of OCR output, cache GC/retention, real scanned-resume witness run, and SQLCipher/physical purge remain not complete. |
| S31 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this local OCR worker command/cache-write slice; PDF page rendering, per-page multi-page OCR, daemon-loop OCR execution, searchable OCR text indexing, bbox persistence, full pause/resume loop, concrete OCR engine install/license, real scanned-resume witness run, and Windows process-tree validation remain not complete. |
| S32 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this local-discovery root preset slice; real whole-machine witness runs, explicit user confirmation UX, persisted scan-scope records, progress/cancel/budget limits, per-root partial-failure UX, cross-platform root enumeration validation, and proof that all local resumes are discoverable remain not complete. |
| S33 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this persisted OCR pause/resume control slice; daemon OCR loop integration, interrupting an already-running OCR child, process-tree pause semantics, PDF page rendering, concrete engine install/license, searchable OCR indexing, bbox persistence, real scanned-resume witness, and Windows process-control validation remain not complete. |
| S34 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p embedder`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p embedder -p index-vector --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this local embedding command client slice; concrete embedding model selection/license/install, model distribution, embedding daemon/queue integration, persistent vector index, CLI semantic/hybrid search using the vector channel, quality/performance benchmarks, real data validation, and cross-platform process-tree validation remain not complete or BLOCKED. |
| S35 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p embedder`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this persisted import scan-scope metadata slice; live progress streaming, cancel/resume controls for import scans, budget limits, per-file scan error UI, real whole-machine witness runs, encrypted path metadata, and cross-platform root validation remain not complete. |
| S36 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p fs-crawler -p meta-store -p import-pipeline -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this scan file-budget enforcement slice; live progress streaming, user-triggered import cancellation, time/byte/CPU budgets, persisted per-file errors, real whole-machine witness runs, encrypted path metadata, and Windows/macOS full-disk validation remain not complete. |
| S37 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-cli -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this redacted persisted scan-error slice; live progress streaming, user-triggered import cancellation, time/byte/CPU budgets, file-level UI/UX, real whole-machine witness runs, encrypted path metadata, keyed path-error correlation, and Windows/macOS full-disk validation remain not complete. |
| S38 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p index-vector -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this persisted vector snapshot slice; real licensed embedding model selection/distribution, import-time embedding queue integration, CLI semantic/hybrid query execution, vector snapshot GC/repair, quality benchmarks, real data validation, and cross-platform validation remain not complete or BLOCKED. |
| S39 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli embed_worker_debug_output_redacts_candidate_text_and_command_path`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p embedder`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p embedder -p index-vector --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this CLI local embedding worker slice; real licensed embedding model selection/distribution, OS-enforced no-network sandboxing for user-provided commands, daemon-loop embedding execution, import-time embedding job state, CLI semantic/hybrid query execution, vector snapshot GC/repair, quality benchmarks, real data validation, and cross-platform command validation remain not complete or BLOCKED. |
| S40 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this local-discovery default budget and multi-root budget-summary slice; live progress streaming, user cancellation, time/byte/CPU budgets, user-facing partial-result UX, real whole-machine witness runs, and Windows/macOS full-disk validation remain not complete. |
| S41 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p import-pipeline -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this OCR worker searchable-index slice; multi-page PDF rendering, daemon-loop OCR execution, concrete OCR engine install/license, bbox persistence, real scanned-resume witness runs, encrypted OCR text storage/physical purge, and Windows process-tree validation remain not complete. |
| S42 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext`, `/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p rank-fusion -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this CLI semantic/hybrid query slice; licensed embedding model selection/distribution, ONNX/HNSW/FAISS or equivalent ANN, daemon-loop embedding queue, section vectors, real semantic quality/performance benchmarks, OS-enforced no-network command sandboxing, and cross-platform validation remain not complete or BLOCKED. |
| S43 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-cli -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this one-shot daemon import worker slice; long-running scheduling loop, authenticated import command IPC endpoint, import cancellation/progress streaming, background OCR/vector workers, multi-process stress testing, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S44 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this long-running daemon import scheduler slice; authenticated import command IPC endpoint, import cancellation/progress streaming, configurable retry policy, singleton service lifecycle enforcement, background OCR/vector workers, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S45 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_serves_status_while_import_worker_processes_late_queued_task -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_does_not_start_import_worker_when_ipc_bind_fails -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_rejects_worker_tick_limit_in_combined_ipc_worker_mode -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this combined status IPC plus import worker event-loop slice; authenticated command IPC endpoint, import cancellation/progress streaming, configurable retry policy, singleton service lifecycle enforcement, background OCR/vector workers, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S46 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_authenticates_and_queues_import_command_over_ipc -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_requires_bearer_token_for_import_command_ipc -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_task_and_scan_scope_insert_atomically_for_daemon_command_ipc -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this authenticated loopback import command IPC slice; search/detail IPC endpoints, CLI import-over-IPC UX, command token rotation/revocation, import cancellation/progress streaming, singleton service lifecycle enforcement, daemon OCR/vector workers, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S47 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_import_command_preserves_local_discovery_preset_scope -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this CLI import-over-IPC UX slice; search/detail IPC endpoints, daemon endpoint discovery UX, token rotation/revocation, import cancellation/progress streaming, singleton service lifecycle enforcement, daemon OCR/vector workers, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S48 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p rank-fusion -p resume-cli -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this authenticated full-text search-over-IPC slice; daemon endpoint discovery UX, semantic/hybrid daemon search IPC, token rotation/revocation, import/search progress streaming, singleton service lifecycle enforcement, daemon OCR/vector workers, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S49 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s49_detail_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this redacted detail retrieval slice; daemon endpoint discovery UX, semantic/hybrid daemon search IPC, token rotation/revocation, import/search/detail progress streaming, singleton service lifecycle enforcement, daemon OCR/vector workers, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S50 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this daemon OCR worker slice; real PDF page rendering, multi-page OCR, bbox persistence, concrete OCR engine install/license, OCR backpressure, encrypted OCR text purge, real scanned-resume witness runs, daemon embedding worker, and Windows/macOS service/process validation remain not complete or BLOCKED. |
| S51 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this daemon local embedding worker slice; licensed model selection/distribution, ONNX/HNSW/FAISS or equivalent ANN, durable per-version embedding job state, section vectors, semantic quality metrics, real performance proof, OS-enforced no-network sandboxing for configured commands, and Windows/macOS validation remain not complete or BLOCKED. |
| S52 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this durable embedding job-state slice; licensed model selection/distribution, ONNX/HNSW/FAISS or equivalent ANN, model/version invalidation, section vectors, semantic quality metrics, real performance proof, OS-enforced no-network sandboxing for configured commands, and Windows/macOS validation remain not complete or BLOCKED. |
| S53 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this model/dimension-scoped durable embedding job slice; licensed model selection/distribution, ONNX/HNSW/FAISS or equivalent ANN, model-scoped vector query isolation, section vectors, semantic quality metrics, real performance proof, OS-enforced no-network sandboxing for configured commands, and Windows/macOS validation remain not complete or BLOCKED. |
| S54 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this model-scoped vector query isolation slice; licensed model selection/distribution, ONNX/HNSW/FAISS or equivalent ANN, section vectors, semantic quality metrics, real performance proof, OS-enforced no-network sandboxing for configured commands, and Windows/macOS validation remain not complete or BLOCKED. |
| S55 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this section-level vector input slice; licensed model selection/distribution, ONNX/HNSW/FAISS or equivalent ANN, semantic quality metrics, real performance proof, OS-enforced no-network sandboxing for configured commands, and Windows/macOS validation remain not complete or BLOCKED. |
| S56 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this queued/retryable import cancellation slice; live progress streaming, cooperative cancellation of already-running import scans, daemon endpoint discovery UX, token rotation/revocation, singleton service lifecycle enforcement, real whole-machine witness runs, and Windows/macOS validation remain not complete. |
| S57 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p parser-text`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this TXT parser/import/search slice; legacy `.doc`, broader TXT encoding heuristics beyond UTF-8/BOM-marked UTF-16, watcher/background incremental import, production-grade PDF coverage, large-corpus proof, and incremental index updates remain not complete. |
| S58 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this high-confidence name mention slice; broad name dictionaries, multilingual name normalization, name-based soft-dedupe scoring, labeled field F1 metrics, encrypted local storage, and physical purge remain not complete. |
| S59 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this local IPC endpoint auto-discovery slice; live progress streaming, cooperative cancellation of already-running import scans, token rotation/revocation, singleton service lifecycle enforcement, real whole-machine witness runs, and Windows/macOS validation remain not complete. |
| S60 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this running-import cooperative cancellation slice; live progress streaming, cancel-over-IPC UX, token rotation/revocation, singleton service lifecycle enforcement, real whole-machine witness runs, and Windows/macOS validation remain not complete. |
| S61 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this status-pollable import progress slice; dedicated push progress streaming, cancel-over-IPC UX, token rotation/revocation, singleton service lifecycle enforcement, real whole-machine witness runs, and Windows/macOS validation remain not complete. |
| S62 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this authenticated import cancel-over-IPC slice; dedicated progress stream, token rotation/revocation, singleton service lifecycle enforcement, real whole-machine witness runs, Windows/macOS validation, and packaging/signing remain not complete or BLOCKED. |
| S63 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this authenticated import progress stream slice; daemon index-maintenance workers, service lifecycle, CI, CODEOWNERS, real whole-machine witness runs, Windows/macOS validation, token rotation/revocation, and packaging/signing remain not complete or BLOCKED. |
| S64 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this daemon full-text index maintenance worker slice; queued incremental index jobs, snapshot GC/retention, vector or ANN index maintenance, service lifecycle, CI, CODEOWNERS, real whole-machine witness runs, Windows/macOS validation, token rotation/revocation, and packaging/signing remain not complete or BLOCKED. |
| S65 | Local slice complete; remote unblocked later | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo metadata --no-deps --locked --format-version 1`, `/Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- synthetic-query --documents 24 --queries 6 --top-k 5 --json`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace --locked`, `./scripts/ci/check-licenses.sh`, `./scripts/ci/guard-public-repo.sh`, `sh -n scripts/ci/guard-public-repo.sh scripts/ci/check-licenses.sh scripts/ci/verify-local.sh scripts/ci/configure-github-repo.sh`, and the obsolete-reference marker scan passed with no matches. | Remote GitHub repository work was blocked in S65 by invalid CLI auth but was unblocked and started during S67. Real whole-machine witness runs, Windows/macOS validation, token rotation/revocation, and packaging/signing remain not complete or BLOCKED. |
| S66 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked`, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace --locked`, `./scripts/ci/guard-public-repo.sh`, and the obsolete-reference marker scan passed with no matches. | None for this macOS LaunchAgent CLI lifecycle slice; live `launchctl` start/stop was implemented but not exercised against the user's real login session, and Windows service/MSI, signed pkg/dmg, notarization, real upgrade/uninstall, hosted runner validation, and complete release packaging remain not complete or BLOCKED. |
| S67 | Product governance slice complete | `gh repo view FrankQDWang/resume-ir` showed the repository was initially absent, `gh repo create FrankQDWang/resume-ir --public --source=. --remote=origin --description "Local-first resume search engine" --disable-wiki` created it, `git remote -v` showed HTTPS origin, `./scripts/ci/guard-public-repo.sh` passed, `git push -u origin main` pushed `cc009da12c7c5753bbf3e66642fccee7db2ebeae`, and `sh -n scripts/ci/configure-github-repo.sh` plus `git diff --check` passed after the HTTPS fallback script fix. | Branch protection is intentionally deferred until this S67 progress/script-fix commit is pushed. PR creation, hosted Actions results, releases, signing, notarization, Windows/macOS package validation, and real whole-machine witness runs remain not complete or BLOCKED. |
| S68 | Product governance slice complete | `./scripts/ci/configure-github-repo.sh FrankQDWang resume-ir` failed at `gh repo edit` with `HTTP 422` because `--allow-forking` is only applicable to org-owned private repositories, `sh -n scripts/ci/configure-github-repo.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, and the obsolete-reference marker scan passed after removing that invalid option. | Branch protection still has to be rerun after S68 is pushed. Hosted Actions results, releases, signing, notarization, Windows/macOS package validation, and real whole-machine witness runs remain not complete or BLOCKED. |
| S71 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked`, and `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings` passed after the RED test first failed because `fault-simulate` did not exist. | None for this safe fault-simulation CLI slice; actual disk-fill/ENOSPC, real file-lock semantics, kill-daemon fault injection, OCR worker crash injection, migration-failure injection, model checksum fault, battery mode, external-drive disconnect, and cross-platform validation remain not complete or BLOCKED. |
| S72 | Stability slice complete | `./scripts/ci/verify-local.sh` first exposed a concurrent local-command embedder temp-directory collision as `EngineFailed`; after the fix, `/Users/frankqdwang/.cargo/bin/cargo test -p embedder --test s11_embedder --locked` passed with 6 tests and `./scripts/ci/verify-local.sh` passed end to end. | None for this CI stability slice; licensed model packaging, ANN, real semantic quality metrics, OS-enforced no-network sandboxing for configured commands, and Windows/macOS validation remain not complete or BLOCKED. |
| S73 | CI portability slice complete | GitHub Actions PR #9 `rust workspace` failed on Linux because the embedder permission test used macOS `stat -f` before GNU `stat -c`; the fixture command now uses GNU `stat -c` first and falls back to macOS `stat -f`. | None for this Linux CI test portability slice; broader Linux package validation, Windows validation, signed installers, notarization, and full cross-platform release evidence remain not complete or BLOCKED. |
| S74 | CI fix attempt; superseded by S75 | GitHub Actions PR #9 `rust workspace` then failed in `ocr-client` because timeout cleanup could return after the direct child exited while descendants still held output pipes. The first local fix sent `KILL` to the process group even after the direct child exited, and `./scripts/ci/verify-local.sh` passed locally, but GitHub Actions still failed on the same descendant-pipe timing test. | S74 alone did not clear Linux CI; S75 follows with the actual timeout-path reader fix. |
| S75 | CI fix attempt; superseded by S76 | GitHub Actions PR #9 `rust workspace` still failed in `local_command_worker_terminates_descendants_that_keep_output_pipes_open` after S74. The timeout/cancel/error path returned the terminal OCR error without joining stdout/stderr reader threads, preventing inherited pipes from delaying timeout return, and `./scripts/ci/verify-local.sh` passed locally. | GitHub Actions later failed with exit 143 while running `tests/s50_ocr_worker.rs`, so S75 was not sufficient; S76 follows with child-process cleanup plus output-reader joining. |
| S76 | CI fix attempt; superseded by S77 | GitHub Actions PR #9 `rust workspace` failed after S75 with exit 143 while running daemon OCR worker tests. The S76 fix restored timeout/cancel error-path output-reader joining, while terminating direct child processes before the parent exited and then terminating the process group so inherited pipes would not hang cleanup. Focused local checks passed for `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked` and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked`; `./scripts/ci/verify-local.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, and the obsolete-reference marker scan also passed. | GitHub Actions later failed in the original inherited-pipe descendant timeout test, so S76 was not sufficient on Linux; S77 follows with portable process-group signal syntax. |
| S77 | CI fix attempt; superseded by S78 | GitHub Actions PR #9 `rust workspace` failed after S76 in `local_command_worker_terminates_descendants_that_keep_output_pipes_open`; the timeout returned only after the descendant closed inherited pipes. The S77 fix used `/bin/kill <signal> -- -PGID` for OCR Unix process-group signaling and removed the unreliable direct-child `pkill -P` helper. Focused local checks passed for `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked` and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked`; `./scripts/ci/verify-local.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, and the obsolete-reference marker scan also passed. | GitHub Actions later passed OCR but failed with exit 143 while running daemon embedding worker tests, exposing the same process-group signaling gap in the embedder; S78 follows. |
| S78 | CI portability slice complete | GitHub Actions PR #9 `rust workspace` failed after S77 with exit 143 while running `tests/s51_embedding_worker.rs`, after OCR tests had passed. The S78 fix applies the same `/bin/kill <signal> -- -PGID` Unix process-group syntax to the local command embedder and adds an embedder inherited-pipe descendant timeout regression test. Focused local checks passed for `/Users/frankqdwang/.cargo/bin/cargo test -p embedder --test s11_embedder --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker --locked`, and `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked`; `./scripts/ci/verify-local.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, the obsolete-reference marker scan, and PR #9 hosted checks also passed after formatting. | None for this process-cleanup portability slice. Real embedding model packaging, ANN, Linux/macOS/Windows service validation, signed installers, notarization, and full release evidence remain not complete or BLOCKED. |
| S79 | Product diagnostics slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_redacted_resource_telemetry --locked` first failed because doctor/export did not report resource telemetry; after implementation, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings`, `./scripts/ci/verify-local.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, the obsolete-reference marker scan, and PR #9 hosted checks passed. | None for this redacted resource telemetry slice; 100k/1M real-corpus benchmarks, nightly gates, destructive kill/actual ENOSPC fault injection, file-lock semantics, runbooks, and cross-platform performance evidence remain not complete or BLOCKED. |
| S80 | Product fault-injection slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_file_lock_reproduces_contention_without_path_leak --locked` and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths --locked` first failed because `file-lock` was not supported and diagnostics did not advertise `file_lock`; after implementation, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings`, `./scripts/ci/verify-local.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, and the obsolete-reference marker scan passed. | None for this safe file-lock contention slice; 100k/1M real-corpus benchmarks, nightly gates, destructive kill/actual ENOSPC fault injection, kill-daemon/OCR-crash fault injection, model checksum fault, battery mode, external-drive disconnect, runbooks, and cross-platform performance evidence remain not complete or BLOCKED. |
| S81 | Product fault-injection slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_daemon_kill_restarts_configured_daemon_without_path_leak -- --exact` first failed because `daemon-kill` was not supported, and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths -- --exact` first failed because diagnostics did not advertise `daemon_kill`; after implementation, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s81_daemon_kill --locked`, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings`, `git diff --check`, and `./scripts/ci/verify-local.sh` passed. | None for this safe daemon-kill/restart probe slice; destructive service-manager kill, actual ENOSPC, OCR-crash fault injection, model checksum fault, battery mode, external-drive disconnect, runbooks, Windows/macOS service validation, and cross-platform performance evidence remain not complete or BLOCKED. |
| S82 | Product fault-injection slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_ocr_crash_reproduces_engine_failure_without_payload_or_path_leak -- --exact` first failed because `ocr-crash` was not supported, and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths -- --exact` first failed because diagnostics did not advertise `ocr_crash`; after implementation, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked`, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings`, `git diff --check`, and `./scripts/ci/verify-local.sh` passed. | None for this safe OCR command-crash probe and retryable worker-failure slice; destructive service-manager kill, actual ENOSPC, model checksum fault, battery mode, external-drive disconnect, runbooks, Windows/macOS service validation, and cross-platform performance evidence remain not complete or BLOCKED. |
| S83 | Product runbook/CI guard slice complete | `sh scripts/ci/check-runbooks.sh` first failed with `missing required runbook: docs/runbooks/diagnostics-redaction.md`; after adding local-only runbooks and wiring the guard into local/hosted CI, `./scripts/ci/check-runbooks.sh`, `sh -n scripts/ci/check-runbooks.sh scripts/ci/verify-local.sh scripts/ci/guard-public-repo.sh scripts/ci/check-licenses.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, and `./scripts/ci/verify-local.sh` passed. | None for this production runbook and policy-guard slice; 100k/1M real-corpus benchmarks, nightly performance gates, destructive service-level kill/actual ENOSPC fault injection, model checksum fault, battery mode, external-drive disconnect, Windows/macOS service validation, and cross-platform performance evidence remain not complete or BLOCKED. |
| S84 | Product benchmark-gate slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked` first failed because `evaluate_benchmark_gate_json` and `BenchmarkGateConfig` did not exist; after implementation, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings`, the `resume-benchmark synthetic-query` plus `resume-benchmark gate` smoke, `./scripts/ci/check-runbooks.sh`, `git diff --check`, and `./scripts/ci/verify-local.sh` passed. | None for this synthetic benchmark gate and workflow wiring slice; 100k/1M real-corpus benchmark datasets, real-corpus nightly/release performance gates, semantic/vector quality gates, OCR throughput gates, Windows/macOS benchmark runners, and cross-platform performance evidence remain not complete or BLOCKED. |
| S85 | Product fault-injection slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_model_checksum --locked` first failed because `model-checksum` was unsupported; after implementation, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings`, `./scripts/ci/check-runbooks.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, the obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this controlled local model artifact checksum probe slice; real licensed model selection/download/distribution, model package manifest governance, semantic/vector quality gates, battery mode, external-drive disconnect, destructive actual ENOSPC/service-manager drills, Windows/macOS validation, and cross-platform performance evidence remain not complete or BLOCKED. |
| S86 | Product model-governance slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker model_manifest_validate --locked` first failed because `model validate-manifest` was unsupported, then failed after schema tightening because the implementation only accepted a single-model manifest instead of `model_pack_id` plus `models[]`; `./scripts/ci/verify-local.sh` also exposed a daemon scheduler test race where a post-startup queued task could be claimed before its scan scope was written, fixed by using the existing atomic `insert_import_task_with_scan_scope` API in the test helper. After implementation and the stability fix, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings`, `./scripts/ci/check-runbooks.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, the obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this local model-pack manifest validation slice and daemon scheduler test stability repair; real licensed OCR/embedding model selection/download/distribution, model quality evaluation, ANN production indexing, semantic/vector quality gates, production model performance proof, and cross-platform release evidence remain not complete or BLOCKED. |
| S87 | Product search slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_fields_before_fulltext_top_k_cutoff --locked -- --exact` first failed because field filters were applied only after the full-text TopDocs cutoff, causing a synthetic Rust candidate outside the top five unfiltered keyword hits to be missed with `--top-k 1`; after implementation, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p index-fulltext -p meta-store --all-targets --locked -- -D warnings`, `git diff --check`, `./scripts/ci/check-runbooks.sh`, `./scripts/ci/guard-public-repo.sh`, the obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this metadata-indexed field prefilter slice; broader dictionaries, stronger normalization, labeled field F1, ANN/vector quality gates, SQLCipher/encrypted metadata, physical purge, large-corpus proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S88 | Product privacy/delete slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact` first failed because `resume-cli purge` was unsupported; after implementation, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p index-fulltext -p index-vector -p meta-store --all-targets --locked -- -D warnings`, `git diff --check`, `./scripts/ci/check-runbooks.sh`, `./scripts/ci/guard-public-repo.sh`, the obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this explicit best-effort local deleted-document purge slice; SQLCipher/encrypted metadata, forensic erase, full OCR/cache/job-retention purge coverage, real-resume witness runs, large-corpus proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S89 | Product OCR slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_processes_all_scanned_pdf_pages_before_indexing --locked -- --exact` first failed because OCR worker behavior was single-page/cache-write `1`; after implementation, focused CLI, daemon, OCR client, import-pipeline, parser-pdf, fmt, clippy, `git diff --check`, runbook guard, public-repo guard, obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this local PDF render command protocol and multi-page OCR fan-out slice; concrete PDF renderer/OCR engine install and license evidence, real Poppler/PDFium/Tesseract witness runs, bbox persistence, backpressure, full OCR cache/job purge coverage, real scanned-resume witness runs, large-corpus proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S90 | Product privacy/delete slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact` first failed after the test was tightened because `purge --deleted` did not report or remove OCR cache/job retention surfaces; after implementation, the focused RED/GREEN test, full `s14_delete_search`, `meta-store`, focused clippy, fmt, `git diff --check`, runbook guard, public-repo guard, obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this current OCR cache/job purge slice; SQLCipher/encrypted metadata, forensic erase, future OCR bbox purge surfaces, real-resume witness runs, large-corpus proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S91 | Product OCR slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client pdftoppm_renderer_renders_valid_pdf_page_to_ppm_without_payload_debug_leaks --locked -- --exact` first failed because `PdftoppmPdfRenderer` and `PdftoppmRenderSpec` did not exist; after implementation, OCR client, CLI handoff, daemon worker, fmt, focused clippy, `git diff --check`, runbook guard, public-repo guard, obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this local Poppler `pdftoppm` renderer adapter and CLI/daemon worker wiring slice; Tesseract or equivalent real OCR recognition engine, renderer/OCR distribution policy, bbox persistence, backpressure, real scanned-resume witness runs, large-corpus proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S92 | Product OCR slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client tesseract_worker_recognizes_synthetic_image_without_payload_debug_leaks -- --exact` first failed because `TesseractOcrClient` and `TesseractOcrSpec` did not exist; after implementation, local Tesseract 5.5.2 was installed, the focused Tesseract OCR client, CLI worker, daemon worker, fmt, focused clippy, `git diff --check`, runbook guard, public-repo guard, obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this local Tesseract adapter and CLI/daemon wiring slice; final OCR/renderer distribution policy, non-English language packs, OCR bbox persistence, backpressure, real scanned-resume witness runs, large-corpus OCR throughput proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S93 | Product OCR slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client tesseract_worker_recognizes_synthetic_image_without_payload_debug_leaks --locked -- --exact` and `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite ocr_page_cache_persists_word_boxes_without_debug_payload_leak --locked -- --exact` first failed because OCR word-box APIs and cache persistence did not exist; after implementation, OCR client, meta-store, CLI handoff, daemon worker, fmt, focused clippy, `git diff --check`, schema expectation guard, and `./scripts/ci/verify-local.sh` passed. | None for this OCR word-box persistence slice; final OCR/renderer distribution policy, non-English language packs, backpressure, real scanned-resume witness runs, large-corpus OCR throughput proof, Windows/macOS validation, and future OCR bbox purge surface audits remain not complete or BLOCKED. |
| S94 | Product OCR backpressure slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact` and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact` first failed because OCR max-page budget parameters and guards did not exist; after implementation, CLI OCR handoff, daemon OCR worker, service lifecycle, fmt, focused clippy, `git diff --check`, runbook guard, public-repo guard, obsolete-reference marker guard, and `./scripts/ci/verify-local.sh` passed. | None for this OCR page-count backpressure slice; final OCR/renderer distribution policy, non-English language packs, real scanned-resume witness runs, large-corpus OCR throughput proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S95 | Product OCR remediation slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact` first failed because `status` did not report `ocr page budget blocked`; after implementation, meta-store, CLI OCR handoff, daemon OCR worker, CLI status IPC, fmt, focused clippy, and related full suites passed. | None for this redacted OCR page-budget remediation slice; final OCR/renderer distribution policy, non-English language packs, real scanned-resume witness runs, large-corpus OCR throughput proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S96 | Product OCR diagnostics slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_ocr_runtime_without_paths_or_language_dump --locked -- --exact` first failed because doctor did not report `ocr renderer pdftoppm`; after implementation, OCR runtime diagnostics, non-executable tool handling, full diagnostics, fmt, focused clippy, guards, and local verification passed. | None for this redacted local OCR runtime diagnostics slice; final OCR/renderer distribution policy, non-English language pack install/selection policy, real scanned-resume witness runs, large-corpus OCR throughput proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S97 | Product import slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p parser-doc --test s6_doc extracts_legacy_doc_text_with_local_converter_without_output_leakage --locked -- --exact` first failed because `DocParser::with_converter` did not exist; after implementation, parser-doc, parser-common, import-pipeline, fmt, focused clippy, and a private local-only PDF/Word witness passed with no path leaks. | None for this legacy Word local-converter slice; converter distribution policy, Windows/Linux converter proof, remaining malformed/encrypted DOC behavior, full OCR completion for scanned PDFs, large-corpus proof, and full real-resume library validation remain not complete or BLOCKED. |
| S98 | Product import scheduler slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_rescans_completed_root_without_path_leak --locked -- --exact` first failed because daemon did not accept `--rescan-completed-imports`; after implementation, daemon import scheduler, meta-store, fmt, focused clippy, focused tests, `git diff --check`, runbook guard, public-repo guard, private-witness marker scan, obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this polling background rescan slice; true OS filesystem watcher integration, large-corpus long-running rescan proof, cross-platform watcher behavior, and incremental index-update-only writes remain not complete or BLOCKED. |
| S99 | Product local witness slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_imports_only_pdf_and_word_samples_without_persisting_private_data --locked -- --exact` first failed because `resume-cli witness` was unsupported; after implementation, focused witness, full `s9_import_search`, fmt, focused clippy, guard checks, `./scripts/ci/verify-local.sh`, and a private local-only PDF/Word witness with redacted output passed. | None for this isolated local PDF/Word witness command slice; it is not a production benchmark, does not package converters/OCR/model runtimes, does not prove Windows/Linux behavior, and does not complete full real-library quality/performance validation. |
| S100 | Product local OCR witness slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_executes_local_command_without_output_or_path_leak --locked -- --exact` and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_without_command_reports_blocked_without_persisting_private_data --locked -- --exact` first failed because `resume-cli witness` did not accept OCR options; after implementation, focused witness OCR tests, full import-search and OCR suites, fmt, focused clippy, guard checks, `./scripts/ci/verify-local.sh`, and bounded private local-only OCR witnesses passed. | None for this isolated local OCR witness option slice; it is not a full-library OCR proof, does not package OCR runtimes, does not prove non-English OCR quality, does not prove Windows/Linux behavior, and does not complete large-corpus OCR throughput validation. |
| S101 | Product import watcher slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_watcher_requeues_completed_root_after_file_change_without_path_leak --locked -- --exact` first failed because daemon did not accept `--watch-import-roots`; after implementation, focused watcher exact, full daemon import scheduler suite, daemon clippy, license guard, fmt, guard checks, and `./scripts/ci/verify-local.sh` passed. | None for this local OS watcher requeue slice; it does not prove Windows watcher behavior, long-running watcher soak stability, large-corpus event storms, or incremental index-update-only writes. |
| S102 | Product field-quality gate slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality --locked` first failed because the field-quality APIs did not exist, and `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_quality_outputs_redacted_report_and_gate --locked -- --exact` first failed because `resume-benchmark` did not accept `field-quality`; after implementation, focused field-quality tests, full benchmark-runner tests, focused clippy, license guard, fmt, guard checks, and `./scripts/ci/verify-local.sh` passed. | None for this labeled field-quality evaluator/gate slice; it does not supply real business labeled datasets, prove production field F1, improve dictionaries, or complete soft-dedupe scoring. |
| S103 | Product soft-dedupe hint slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion soft_dedupe --locked` first failed because soft-dedupe APIs did not exist; `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding search_marks_soft_duplicate_hints_without_low_confidence_folding --locked -- --exact` first failed because local search did not print soft-dedupe hints; and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_includes_redacted_soft_dedupe_hints --locked -- --exact` first failed because daemon search JSON omitted `soft_dedupe`. After implementation, focused rank-fusion, CLI, daemon IPC tests, related suites, focused clippy, fmt, diff, runbook, public guard, and `./scripts/ci/verify-local.sh` passed. | None for this bounded redacted soft-dedupe hint slice; it does not prove real dedupe precision/recall, does not implement manual merge review, does not add large-name-bucket indexing beyond existing mention indexes and bounded candidate scans, and does not prove million-corpus latency impact. |
| S104 | Product metadata migration fault-injection slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_metadata_migration_failure_reproduces_without_path_or_schema_leak --locked -- --exact` first failed because `migration-failure` was unsupported; `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths --locked -- --exact` first failed because diagnostics did not list `metadata_migration`. After implementation, focused fault/diagnostics tests, related suites, focused clippy, fmt, diff, runbook, public guard, marker scans, and `./scripts/ci/verify-local.sh` passed. | None for this safe synthetic migration-failure probe; it does not perform destructive migration rollback drills against real user metadata, backup/restore workflow proof, cross-platform filesystem fault proof, or upgrade rehearsal. |
| S105 | Product local OCR witness-budget slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_can_budget_documents_after_full_private_scan_without_path_leak --locked -- --exact` first failed because `witness` rejected `--ocr-max-documents`; after implementation, focused witness exact, full import-search witness suite, OCR handoff suite, focused clippy, fmt, diff, runbook, public guard, marker scans, `./scripts/ci/verify-local.sh`, and a private local-only full-directory witness with a bounded OCR document budget passed. | None for this redacted local OCR witness-budget control; it does not prove full-library OCR completion, OCR throughput, OCR quality, non-English OCR behavior, packaged OCR runtime distribution, Windows/Linux behavior, or large-corpus performance. |
| S106 | Product local-discovery witness slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_local_discovery_preset_uses_discovery_profile_without_path_leak --locked -- --exact` first failed because `witness` rejected `--root-preset local-discovery`; after implementation, focused local-discovery witness exact, full import-search witness suite, fs-crawler suite, focused clippy, fmt, diff, runbook, public guard, marker scans, `./scripts/ci/verify-local.sh`, and a private local-only local-discovery witness using the user-authorized sample directory override passed. | None for this redacted local-discovery witness path; it does not prove default whole-machine scans from `/`, Windows drive scanning, full-library OCR completion, large-corpus performance, or cross-platform watcher behavior. |
| S107 | Product synthetic OCR throughput gate slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner synthetic_ocr_throughput_reports_page_latency_without_payload_or_path_leakage --locked -- --exact` first failed because the OCR throughput API did not exist, and `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_ocr_throughput_outputs_redacted_report_and_gate --locked -- --exact` first failed because `resume-benchmark` rejected `ocr-throughput`; after implementation, focused OCR throughput tests, full benchmark-runner tests, focused clippy, fmt, diff, runbook, public guard, marker scans, and `./scripts/ci/verify-local.sh` passed. | None for this synthetic OCR throughput benchmark/gate; it does not prove real scanned-resume OCR quality, full-library OCR completion, non-English OCR behavior, packaged OCR runtime distribution, 100k/1M corpus performance, or Windows/Linux behavior. |
| S108 | Product workflow-gate slice complete | `sh scripts/ci/check-workflows.sh` first failed because PR/nightly workflows did not include `ocr-throughput`; after implementation, workflow guard, synthetic local OCR benchmark smoke plus redaction scan, shell syntax checks, fmt, diff, and `./scripts/ci/verify-local.sh` passed. | None for this OCR benchmark workflow wiring slice; it does not prove real scanned-resume OCR quality, full-library OCR completion, non-English OCR behavior, packaged OCR runtime distribution, 100k/1M corpus performance, or Windows/Linux behavior. |
| S109 | Product local OCR witness resilience slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_budget_reports_failed_documents_without_stopping_or_leaking_paths --locked -- --exact` first failed because a budgeted witness stopped as `blocked` on the first per-document OCR failure; after implementation, the focused exact, full `s9_import_search`, focused CLI clippy, fmt, diff, guard checks, marker scans, `./scripts/ci/verify-local.sh`, and private local-only PDF/Word witness runs passed with redacted aggregate output and temporary private data removal. | None for this bounded local witness resilience slice; it does not prove OCR quality, full-library OCR completion, non-English OCR behavior, packaged runtime distribution, 100k/1M corpus performance, or Windows/Linux behavior. |
| S110 | Product vector-quality gate slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner vector_quality_report_scores_labeled_samples_without_text_id_path_or_vector_leakage --locked -- --exact` first failed because vector-quality APIs did not exist, and `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_vector_quality_outputs_redacted_report_and_gate --locked -- --exact` first failed because `resume-benchmark` rejected `vector-quality`; after implementation, focused vector-quality tests, full benchmark-runner tests, focused benchmark-runner clippy, fmt, diff, guard checks, `./scripts/ci/verify-local.sh`, and private local-only bounded PDF/Word witness runs passed with redacted aggregate output and temporary private data removal. | None for this labeled vector-quality evaluator/gate slice; it does not supply real business labeled semantic datasets, choose/license/package a production embedding model, add ANN production indexing, prove large-corpus semantic latency, or validate Windows/Linux behavior. |
| S111 | Product vector workflow-gate slice complete | `./scripts/ci/check-workflows.sh` first failed because PR/nightly workflows did not include `vector-quality`; after implementation, workflow guard, strict local vector smoke/gate reproduction with redaction scan, shell syntax, workflow YAML parse, diff, public guard, marker scans, and `./scripts/ci/verify-local.sh` passed. | None for this vector-quality workflow wiring slice; it uses a synthetic labeled smoke dataset and temporary fixture embedding command, so it does not prove real semantic quality, licensed production model selection, ANN latency, 100k/1M corpus performance, or Windows/Linux behavior. |
| S112 | Product platform PR validation slice complete | `./scripts/ci/check-workflows.sh` first failed because `.github/workflows/ci-platform.yml` did not include a PR trigger; after implementation, workflow guard, workflow YAML parse, diff, public guard, and `./scripts/ci/verify-local.sh` passed. Hosted Platform CI then exposed two test-portability gaps, a hosted macOS test wait budget issue, a real Windows path-normalization bug in missing-file deletion propagation, Windows full-text snapshot publish instability during CLI imports, and Windows witness temp cleanup semantics. Local fixes now keep OCR/embedding command tests enabled on Windows with `.cmd` fixtures, extend daemon test waiting without changing product tick limits, compare deletion candidates using normalized paths, publish full-text snapshots before validation and retry transient publish locks, release witness metadata handles before cleanup, and retry witness cleanup. The final hosted PR checks passed: macOS Platform CI, Windows Platform CI, Rust workspace, dependency tree, license policy, runbook policy, and public repository guard. | None for this PR-triggered hosted build/test validation slice; it still does not prove installer packaging, signing, notarization, Windows service/MSI install/upgrade/uninstall/rollback, macOS pkg/dmg install/upgrade/uninstall/rollback, platform-specific service lifecycle behavior, real whole-machine scans, or complete release readiness. |
| S113 | Product local PDF/Word witness validation slice complete | Two authorized local-only witness runs over the private sample root passed without uploading or committing real resume data. The import-only run reported redacted aggregate import status and removed private witness data. The bounded OCR run used local `tesseract` and `pdftoppm`, reported redacted aggregate OCR status, and removed private witness data. No real resume data, filenames, paths, counts, raw text, or diagnostics were committed or uploaded. | None for this local-only private sample witness; it does not prove full-library OCR completion, OCR quality, non-English OCR quality, large-corpus latency/throughput, packaging/signing/installers, Windows/Linux real sample behavior, or production model/ANN readiness. |
| S114 | Product persistent vector ANN slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector persistent_vector_index_uses_hnsw_ann_backend_after_reopen_and_keeps_model_scope --locked -- --exact` first failed because `VectorSearchBackend` and `VectorSnapshot::search_backend()` did not exist. The CLI diagnostics exact tests first failed because vector status still reported `available (vector snapshot)`. After implementation, focused index-vector and CLI diagnostics tests, fmt, diff, focused clippy, license policy, and `./scripts/ci/verify-local.sh` passed. | None for this HNSW ANN backend slice; it does not choose/license/package a production embedding model, prove real semantic quality, prove ANN recall/latency on 100k/1M corpora, add durable serialized HNSW graph artifacts separate from the existing vector snapshot, or validate hosted Windows/macOS for the new dependency. |
| S115 | Product persistent vector writer-lock slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector persistent_vector_index_merges_writes_from_stale_concurrent_openers --locked -- --exact` first failed because a second stale `PersistentVectorIndex` opener rewrote the snapshot from old in-memory state and dropped the first opener's vector. After implementation, `cargo test -p index-vector --locked`, focused clippy, fmt, diff, license policy, and `./scripts/ci/verify-local.sh` passed. | None for this vector writer-lock slice; it uses cooperative local file locking and does not prove network filesystem locking semantics, durable serialized ANN graph artifacts, real large-corpus vector performance, production embedding model selection, or hosted Windows/macOS validation for this specific change. |
| S116 | Product Windows full-text read-open retry slice complete | Hosted Windows Platform CI for `f15ce1e` first failed in `published_snapshot_becomes_active_without_reading_staging_orphans` because immediate read-open of a just-inspected Tantivy snapshot returned `Access is denied. (os error 5)`. After implementation, the retry unit test, the hosted-failing full-text test, `cargo test -p index-fulltext --locked`, focused clippy, fmt, diff, public guard, `./scripts/ci/verify-local.sh`, and final hosted PR checks passed. | None for this hosted-Windows transient read-open retry; it does not prove installer/service behavior, real full-library scans, network filesystem semantics, or large-corpus full-text latency. |
| S117 | Product macOS service runtime witness slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli tests::launchctl_status_success_with_running_state_reports_running --locked -- --exact` first failed because service runtime state parsing did not exist. After implementation, the launchctl parser tests, service lifecycle integration tests, focused clippy, fmt, diff, public guard, `./scripts/ci/verify-local.sh`, and a local-only temporary macOS LaunchAgent install/start/status IPC/stop/uninstall witness passed. | None for this local macOS LaunchAgent runtime witness; it does not prove signed pkg/dmg packaging, notarization, upgrade/rollback behavior, Windows service/MSI behavior, or hosted release workflow execution. |
| S118 | Product service status cross-platform portability slice complete | Hosted Windows Platform CI for `288a4c9` first failed in `service_status_and_uninstall_are_redacted_and_preserve_user_data` because `service status` tried to derive a macOS launchctl domain through `/usr/bin/id` on Windows. After implementation, service lifecycle integration tests, launchctl parser tests, focused clippy, fmt, diff, public guard, and `./scripts/ci/verify-local.sh` passed. Hosted Rust Workspace for `c56e966` then exposed a non-macOS clippy dead-code gap that is handled in S119. | None for this portability fix; Windows service/MSI install/start/stop behavior remains not implemented or proven, and non-macOS service runtime status intentionally reports `unknown` for the macOS LaunchAgent command surface. |
| S119 | Product service runtime cfg portability slice complete | Hosted Rust Workspace for `c56e966` first failed on Ubuntu clippy because non-macOS binary builds treated macOS-only launchctl parser code and `running`/`loaded` runtime states as dead code, and newer clippy flagged a needless return in the non-macOS branch. After implementation, service lifecycle integration tests, launchctl parser tests, focused CLI clippy, fmt, diff, public guard, `./scripts/ci/verify-local.sh`, and final hosted PR checks passed. | None for this cfg portability fix; it proves the macOS LaunchAgent command surface remains portable across hosted clippy/builds, but it does not implement Windows services/MSI or prove Windows service lifecycle behavior. |
| S120 | Product OCR requested-language diagnostics slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_check_requested_ocr_language_without_language_dump --locked -- --exact` first failed because `doctor` did not accept OCR diagnostic arguments and diagnostics always reported only `eng`. After implementation, the focused exact test, full diagnostics suite, focused CLI clippy, fmt, diff, public guard, `./scripts/ci/verify-local.sh`, and final hosted PR checks passed. | None for this OCR runtime diagnostics slice; it does not distribute OCR engines or language packs, prove non-English OCR quality, complete full-library OCR, or validate Windows/macOS installed OCR runtime behavior beyond local/hosted command checks. |
| S121 | Product release dry-run manifest slice complete | `sh scripts/ci/check-release-artifacts.sh` first failed because `scripts/release/create-artifact-manifest.sh` did not exist. After implementation, the release artifact guard, workflow guard, runbook guard, diff check, and `./scripts/ci/verify-local.sh` passed. | None for this dry-run manifest/checksum slice; it does not build MSI/pkg/dmg installers, sign, notarize, generate an SBOM, create a GitHub Release, upload release binaries, or prove install/upgrade/uninstall/rollback behavior. |
| S122 | Product local witness search-probe slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_probe_search_runs_private_query_without_leaking_query_or_paths --locked -- --exact` first failed because `witness` rejected `--probe-search`. After implementation, the focused exact test, full `s9_import_search` suite, focused CLI clippy, fmt, diff, marker scan, public guard, `./scripts/ci/verify-local.sh`, private local-only import/search witness, private local-only bounded OCR/search witness, and final hosted PR checks passed. | None for this redacted witness search-probe slice; it does not prove full-library OCR completion, real search quality, real large-corpus latency/throughput, production embedding model readiness, Windows/Linux real sample behavior, or installer/release readiness. |
| S123 | Product local witness field-probe slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_probe_fields_reports_aggregate_counts_without_values_or_paths --locked -- --exact` first failed because `witness` rejected `--probe-fields`. After implementation, the focused exact test, full `s9_import_search` suite, focused CLI clippy, fmt, diff, marker scan, public guard, `./scripts/ci/verify-local.sh`, private local-only field witness, and private local-only bounded OCR/field witness passed with metadata-only field-type aggregation, redacted aggregate output, and temporary private data removal. | None for this redacted witness field-probe slice; it does not prove field extraction quality, real labeled field F1, full-library OCR completion, real search/ranking quality, large-corpus latency/throughput, Windows/Linux real sample behavior, or installer/release readiness. |
| S124 | Product release SBOM dry-run slice complete | `./scripts/ci/check-release-sbom.sh` first failed because the release SBOM guard did not exist. After implementation, the release SBOM guard, release artifact guard, workflow guard, runbook guard, shell syntax checks, diff check, public guard, `./scripts/ci/verify-local.sh`, and hosted PR checks passed. | None for this release SBOM dry-run slice; it does not build MSI/pkg/dmg installers, sign, notarize, create a GitHub Release, upload release binaries, validate installer lifecycle behavior, or prove release readiness. |
| S125 | Product workflow runtime compatibility slice complete | Hosted release dry-run run `26939532282` passed but emitted a GitHub annotation warning that Node.js 20 actions are deprecated for the tracked checkout and artifact upload actions. Official GitHub action release listings showed `actions/checkout` latest `v6.0.3` and `actions/upload-artifact` latest `v7.0.1`. After implementation, workflow YAML parsing, workflow guard, release artifact guard, release SBOM guard, `cargo fmt --check`, `git diff --check`, public repository guard, and `./scripts/ci/verify-local.sh` passed. | None for this workflow runtime compatibility slice; it does not build MSI/pkg/dmg installers, sign, notarize, create a GitHub Release, validate installer lifecycle behavior, prove release readiness, or prove the updated hosted release workflow until the branch is pushed and rerun. |
| S126 | Product hosted release dry-run evidence slice complete | Updated Release workflow run `26940230718` executed on commit `ea043fc`, passed in hosted GitHub Actions, produced a non-expired `release-dry-run` artifact, and its job log no longer contained the Node.js 20 action warning or v4 checkout/upload-artifact references. PR #9 hosted checks for the same commit also passed: Rust workspace, macOS Platform CI, Windows Platform CI, dependency tree, license policy, runbook policy, and public repository guard. | None for this hosted release dry-run evidence slice; it does not build MSI/pkg/dmg installers, sign, notarize, create/upload a GitHub Release, validate installer lifecycle behavior, prove release readiness, or clear the separate `Swatinem/rust-cache@v2`/Node `punycode` warning observed in the cache step logs. |
| S127 | Product local PDF/Word witness validation slice complete | Authorized local-only PDF/Word witnesses over the private sample root completed without uploading or committing real resume data. The import/search/field witness completed with redacted aggregate output and removed private temporary data. A second bounded OCR witness used local `tesseract` and `pdftoppm`, completed the configured OCR document budget without OCR failures, kept the remaining OCR queue budgeted rather than pretending full completion, and removed private temporary data. | None for this local-only private sample witness; it does not prove full-library OCR completion, OCR quality, non-English OCR quality, large-corpus latency/throughput, packaging/signing/installers, Windows/Linux real sample behavior, or production model/ANN readiness. |
| S128 | Product macOS package dry-run slice complete | `./scripts/ci/check-macos-package.sh` first failed because the guard did not exist. After implementation, the macOS package guard generated unsigned synthetic pkg/dmg dry-run artifacts with `pkgbuild`, `productbuild`, and `hdiutil`, validated the pkg/dmg, verified the redacted `macos-package.json` manifest, rejected invalid versions and missing binaries, and `./scripts/ci/verify-local.sh` passed with the new guard wired in. | None for this unsigned local macOS package dry-run slice; it does not sign, notarize, upload a GitHub Release, run install/upgrade/uninstall/rollback, prove Gatekeeper behavior, build Windows MSI, or prove production release readiness. |
| S129 | Product hosted macOS package workflow wiring slice complete | `./scripts/ci/check-workflows.sh` first failed because the Release workflow did not include the macOS package dry-run. After implementation, the Release workflow includes a hosted `macos-latest` job that builds release binaries, runs the unsigned macOS package dry-run, verifies the dmg with `hdiutil`, checks the public artifact boundary, uploads `macos-package-dry-run`, and keeps signing/notarization/release upload gated. Workflow guard, workflow YAML parsing, release artifact guard, release SBOM guard, macOS package guard, diff check, public repository guard, and `./scripts/ci/verify-local.sh` passed. | None for this workflow wiring slice; the updated hosted Release workflow still must run after push, and signing, notarization, installer lifecycle validation, Windows MSI, GitHub Release upload, and production release readiness remain absent or gated. |
| S130 | Product hosted macOS package dry-run evidence slice complete | PR #9 checks for commit `a7dc1c0` passed: dependency tree, license policy, public repository guard, runbook policy, Rust workspace, hosted macOS Platform CI, and hosted Windows Platform CI. Release workflow run `26942549866` executed on the same commit and passed both jobs: `macOS package dry run` and `release dry run`. The run produced non-expired `macos-package-dry-run` and `release-dry-run` artifacts; the macOS job log confirmed dmg checksum verification and unsigned/not-notarized manifest status, and the release job logs did not contain the Node.js 20 action warning or v4 checkout/upload-artifact references. | None for this hosted dry-run evidence slice; it does not sign, notarize, create a GitHub Release, validate install/upgrade/uninstall/rollback behavior, prove Gatekeeper behavior, build Windows MSI, or complete release readiness. |
| S131 | Product Windows MSI dry-run wiring slice complete | `./scripts/ci/check-workflows.sh` first failed because `verify-local` and the Release workflow did not include Windows package dry-run wiring. After implementation, `scripts/release/create-windows-package.ps1` can build an unsigned MSI dry-run through the WiX .NET tool on Windows, writes a redacted `windows-package.json`, rejects invalid versions and missing binaries, and keeps signing/release upload/service lifecycle gated. The Release workflow includes a hosted `windows-latest` job that installs WiX `7.0.0`, builds release binaries, runs the Windows package script, checks artifact boundaries, and uploads `windows-package-dry-run`. Focused workflow, Windows package, runbook, workflow YAML, diff, public guard checks, and `./scripts/ci/verify-local.sh` passed locally, with the Windows package guard explicitly skipped on this non-Windows host. | None for this wiring/local-guard slice; the updated hosted Release workflow still must run after push to prove MSI creation, and signing, service install/start/stop validation, installer lifecycle validation, GitHub Release upload, and production release readiness remain absent or gated. |
| S132 | Product hosted Windows daemon IPC wait-budget fix complete locally | Hosted Windows Platform CI for commit `cd26a04` failed in `daemon_serves_status_while_import_worker_processes_late_queued_task` because the late-queued import worker test did not observe `searchable_documents: 2` inside the previous short polling budget. The fix keeps the same daemon behavior and assertions, but raises this test's max request budget to match the adjacent command-IPC import-worker test. Focused local exact, full `s20_ipc`, diff, public guard, and `./scripts/ci/verify-local.sh` passed. | Hosted PR checks still must rerun after push; this is a CI stability/test-budget fix only and does not prove Windows MSI creation, service lifecycle, installer lifecycle, signing, GitHub Release upload, or production release readiness. |
| S133 | Product WiX package-tool pin slice complete locally | Hosted Release run `26944149485` passed the Ubuntu release dry-run and macOS package dry-run jobs, but the Windows MSI job failed at `Create unsigned Windows MSI dry run` because WiX `7.0.0` required OSMF EULA acceptance. The fix avoids accepting legal/fee terms in CI by pinning the Release workflow and runbook to WiX `6.0.2` and updating the workflow policy guard to reject a missing version pin. Focused workflow, Windows package, runbook, workflow YAML parse, diff, public guard, and `./scripts/ci/verify-local.sh` checks passed locally. | Hosted Release must rerun after push to prove Windows MSI creation; this does not sign artifacts, create/upload a GitHub Release, validate installer lifecycle, validate Windows service lifecycle, accept WiX v7 terms, or complete production release readiness. |
| S134 | Product macOS DMG verify retry slice complete locally | Hosted Release run `26944923353` proved the WiX `6.0.2` fix by passing the Windows package dry-run job, including MSI creation, boundary check, artifact upload, and release gate. The same run failed the macOS package dry-run boundary step because `hdiutil verify` immediately after DMG creation reported `Resource temporarily unavailable`. The fix adds a shared bounded `scripts/release/verify-macos-dmg.sh` helper and wires the Release workflow plus local macOS package guard to use it without skipping checksum verification. Focused workflow, macOS package, shell syntax, workflow YAML parse, diff, public guard, and `./scripts/ci/verify-local.sh` checks passed locally. | Hosted Release must rerun after push to prove the combined release dry-run, macOS package dry-run, and Windows package dry-run all pass on the same branch tip; this does not sign artifacts, create/upload a GitHub Release, validate installer lifecycle, validate Windows service lifecycle, or complete production release readiness. |
| S135 | Product hosted cross-platform Release dry-run evidence slice complete | PR #9 checks for commit `13f35a7` passed: dependency tree, license policy, public repository guard, runbook policy, Rust workspace, hosted macOS Platform CI, and hosted Windows Platform CI. Release workflow run `26945622774` executed successfully on the same commit and passed all three jobs: `release dry run`, `macOS package dry run`, and `Windows package dry run`. The run produced non-expired `release-dry-run`, `macos-package-dry-run`, and `windows-package-dry-run` artifacts without downloading or exposing artifact contents. | None for this hosted dry-run evidence slice; it still does not sign or notarize artifacts, create/upload a GitHub Release, validate install/upgrade/uninstall/rollback behavior, prove Gatekeeper behavior, install/register/start/stop a Windows service, or complete production release readiness. |
| S136 | Product private local PDF/Word witness refresh complete | PR #9 checks for commit `22e1adc` passed: dependency tree, license policy, public repository guard, runbook policy, Rust workspace, hosted macOS Platform CI, and hosted Windows Platform CI. A private local-only explicit-root PDF/Word witness then ran against the user-authorized sample directory with import, redacted search probe, redacted field probe, and bounded OCR through local `tesseract` plus `pdftoppm`. The witness completed without scan budget exhaustion or filesystem scan errors, kept unsupported formats out of scope, surfaced aggregate OCR outcomes under the local English-only OCR configuration, and removed private temporary data. | None for this local-only private sample witness; it does not prove full-library OCR completion, OCR quality, non-English OCR quality, production recall/precision, large-corpus latency/throughput, packaging/signing/installers, Windows/Linux real sample behavior, or production model/ANN readiness. |
| S137 | Product combined OCR language diagnostics slice complete locally | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_check_combined_ocr_languages_without_language_dump --locked -- --exact` first failed because OCR diagnostics treated `eng+chi_sim` as one language pack name. After implementation, diagnostics split Tesseract combined language requests, require every requested language pack to be installed, reject empty or invalid language components, and keep redacted output from dumping unrelated installed languages. The focused exact test, full diagnostics suite, fmt, focused CLI clippy, local `eng+chi_sim` doctor check, and local Apache-2.0 `tesseract-lang` install verification passed. | None for this local OCR language diagnostics slice; it does not distribute language packs in installers, prove non-English OCR quality, complete full-library OCR, validate Windows/macOS installed OCR runtime behavior, or clear the broader OCR quality and packaging blockers. |
| S138 | Product OCR missing-language worker preflight slice complete locally | Focused CLI and daemon OCR worker tests first failed because a Tesseract runtime missing one requested combined language pack could reach the OCR engine path and record a generic engine failure. After implementation, CLI and daemon OCR workers preflight requested Tesseract language packs after cache miss and before renderer/OCR invocation, persist retryable page-cache failures with `LanguageUnavailable`, keep document jobs retryable, and redact blocked-path output. Focused exact tests, full CLI OCR handoff tests, full daemon OCR worker tests, `ocr-client` tests, fmt, and focused clippy passed. | None for this missing-language preflight slice; it does not distribute language packs in installers, prove non-English OCR quality, complete full-library OCR, validate Windows/macOS installed OCR runtime behavior, or clear broader OCR quality and packaging blockers. |
| S139 | Product OCR missing-language remediation visibility slice complete locally | A focused CLI OCR worker test first failed because status, doctor, and redacted diagnostics did not surface the `LanguageUnavailable` blocker created by S138. After implementation, `status_summary` counts retryable OCR jobs with a linked `LanguageUnavailable` OCR page-cache failure; local status, doctor, export-diagnostics, daemon status IPC, and CLI status-over-IPC print redacted aggregate blocker/remediation fields without leaking requested language names, runtime paths, or OCR stderr. Focused exact tests, full CLI OCR handoff, CLI status IPC, daemon status IPC, meta-store tests, fmt, focused clippy, and diff checks passed. | None for this remediation-visibility slice; it does not distribute language packs in installers, prove non-English OCR quality, complete full-library OCR, validate Windows/macOS installed OCR runtime behavior, or clear broader OCR quality and packaging blockers. |
| S140 | Product metadata encryption diagnostic slice complete locally | Focused diagnostics tests first failed because doctor and redacted export did not surface the plaintext metadata-storage state. After implementation, `meta-store` exposes `MetadataEncryptionState::Plaintext`; doctor prints `metadata encryption: plaintext` plus SQLCipher remediation, and `export-diagnostics --redact` includes redacted `metadata_encryption` plus remediation fields without paths or secrets. Focused doctor/export tests, focused meta-store encryption-state test, full diagnostics, full meta-store tests, fmt, focused clippy, diff checks, public repo guard, and full local verification passed. | None for this diagnostic visibility slice; it does not implement SQLCipher, encrypt SQLite, rotate keys, prove forensic erase, or complete the encrypted local storage blocker. |
| S141 | Product OCR command process cleanup flake fix complete locally | GitHub PR #9 `rust workspace` failed on Ubuntu with `local_pdf_render_command_returns_page_bytes_without_payload_debug_leaks` returning `EngineFailed`; a Linux Rust container reproduced the same test-file flake as timeout tests returning `EngineFailed` under parallel execution. Root cause: successful OCR command paths signaled an exited child process group before first letting stdout/stderr readers drain, creating a stale PGID reuse window in parallel Linux tests. The fix defers process-group cleanup until output readers fail to drain within a grace window. Focused macOS `ocr-client` tests, focused clippy, rustfmt, diff check, public repo guard, a Linux container 10-run `s12_ocr_client` loop, and full local verification passed; hosted PR #9 checks also passed after push. | None for this CI/process cleanup slice; it does not change OCR quality, language-pack packaging, renderer selection, or metadata encryption. |
| S142 | Product contact hash key backup/restore slice complete locally | Privacy crate and CLI tests first failed because contact hash key backup/restore APIs and privacy subcommands did not exist. After implementation, contact hash key backup writes a local envelope file with owner-only permissions where supported, restore refuses to overwrite an existing key, Debug/CLI output stays redacted, and restored keys reproduce stable contact HMACs. Focused privacy/CLI tests, full privacy tests, fmt, diff checks, focused clippy, public repo guard, and full local verification passed. | None for this contact hash key backup/recovery slice; it does not rotate keys, encrypt SQLite metadata, protect backup files with passphrases, prove forensic erase, or clear SQLCipher/encrypted storage blockers. |
| S143 | Product passphrase-protected contact key backup slice complete locally | Privacy crate and CLI tests first failed because backup/restore still used the S142 unencrypted backup API and CLI syntax. After implementation, contact key backups require passphrase bytes or a local `--passphrase-file`, use Argon2id plus XChaCha20-Poly1305 to encrypt the key material, reject wrong passphrases without creating a target key, and keep backup files plus command/Debug/error output free of key material, passphrases, contacts, and paths. Focused privacy/CLI tests, full privacy tests, fmt, focused clippy, diff checks, public repo guard, and full local verification passed. | None for this backup-file protection slice; it does not rotate keys, encrypt SQLite metadata, prove forensic erase, or clear SQLCipher/encrypted storage blockers. |
| S144 | Product SQLCipher metadata-store foundation slice complete locally | A focused meta-store test first failed because `MetaStore::open_encrypted` and `MetadataEncryptionState::SqlCipher` did not exist. After implementation, `rusqlite` builds with bundled SQLCipher plus vendored OpenSSL, `MetaStore::open_encrypted` applies a 32-byte raw SQLCipher key before migrations, encrypted stores report `sqlcipher`, wrong-key opens fail with redacted errors, and the encrypted SQLite file lacks the plaintext SQLite header and synthetic document marker. Focused encrypted-store test, full meta-store tests, diagnostics tests, fmt, focused clippy, diff check, public repo guard, and full local verification passed. | None for this encrypted-store foundation slice; the default CLI/daemon data-dir open path still uses plaintext `MetaStore::open`, metadata key generation/storage/migration is not wired, and forensic erase proof remains incomplete. |
| S145 | Product default SQLCipher metadata data-dir slice complete locally | A focused diagnostics test first failed because `doctor` still reported plaintext metadata. After implementation, `MetaStore::open_data_dir` creates or reads an owner-only metadata SQLCipher key under `metadata-secrets/`, CLI and daemon default store opens use SQLCipher, daemon import heartbeats reopen encrypted stores correctly, doctor and redacted diagnostics report `sqlcipher` without remediation, raw default `metadata.sqlite3` lacks the plaintext SQLite header, plaintext opens fail, and metadata-key storage is isolated from contact-key permission failures. Focused default-encryption test, full CLI tests, full daemon tests, full meta-store tests, fmt, focused clippy, diff check, public repo guard, and full local verification passed. | None for this default encrypted metadata path slice; it does not back up or rotate metadata SQLCipher keys, prove plaintext-to-encrypted migration for old pre-release local stores, encrypt full-text/vector/OCR cache artifacts, prove forensic erasure, or clear non-metadata privacy blockers. |
| S146 | Product metadata SQLCipher key backup/restore slice complete locally | A focused CLI test first failed because `resume-cli privacy` did not expose metadata key backup/restore commands. After implementation, metadata key backup writes a passphrase-protected local envelope with Argon2id plus XChaCha20-Poly1305, restore recreates an owner-only metadata SQLCipher key for a copied encrypted metadata DB, wrong passphrases fail without creating a key, duplicate restores are refused, and backup files plus stdout/stderr stay free of passphrases, key material, local paths, and schema payloads. Focused metadata-key CLI tests, existing contact-key CLI tests, full meta-store tests, full CLI tests, fmt, focused clippy, diff check, public repo guard, and full local verification passed. | None for this metadata key backup/recovery slice; it does not rotate metadata SQLCipher keys, automatically sync backups, prove plaintext-to-encrypted migration for old pre-release local stores, encrypt full-text/vector/OCR cache artifacts, prove forensic erasure, or clear non-metadata privacy blockers. |
| S147 | Product metadata SQLCipher key rotation slice complete locally | A focused CLI test first failed because `resume-cli privacy rotate-metadata-key` did not exist. After implementation, metadata key rotation opens the encrypted metadata DB with the existing key, SQLCipher-rekeys it with fresh local key material, replaces the owner-only metadata key file, proves the old key can no longer open the DB, proves the new key can reopen schema version 16, and keeps CLI/doctor output free of local paths and old/new key material. Focused rotation CLI tests, existing metadata backup/restore CLI tests, full meta-store tests, full CLI tests, fmt, focused clippy, diff check, public repo guard, and full local verification passed. | None for this metadata key rotation slice; it does not automatically sync backups, prove crash recovery for every mid-rotation failure window, prove plaintext-to-encrypted migration for old pre-release local stores, encrypt full-text/vector/OCR cache artifacts, prove forensic erasure, or clear non-metadata privacy blockers. |
| S148 | Product plaintext metadata migration-to-SQLCipher slice complete locally | A focused meta-store test first failed because `MetaStore::open_data_dir` could not open an existing plaintext default metadata DB after creating the SQLCipher key. After implementation, the default data-dir open path detects a plaintext SQLite header, exports the plaintext DB to a SQLCipher temp DB with the local metadata key, atomically replaces the default DB, removes the plaintext file from the default path, preserves synthetic document/version rows, and proves plaintext open fails afterward while SQLCipher reopen succeeds. Full local verification then exposed a daemon IPC status-loop regression under SQLCipher WAL; the daemon now keeps a persistent IPC metadata connection, and the late-queued-task test seeds task/scope atomically. Focused migration test, full meta-store tests, full CLI tests, full daemon IPC tests, fmt, focused clippy, diff check, public repo guard, and full local verification passed. | None for this plaintext metadata migration slice; it does not encrypt full-text/vector/OCR cache artifacts, prove forensic erasure, prove crash recovery for every mid-migration failure window, run migration on real user stores, or clear non-metadata privacy blockers. |
| S149 | Product vector snapshot encryption slice complete locally | A focused `index-vector` test first failed because `vector.snapshot` was plaintext TSV with vector IDs, document IDs, model IDs, and float values. After implementation, persistent vector snapshots are written as XChaCha20-Poly1305 encrypted local envelopes with an owner-only local key file; raw snapshot files expose only an encrypted header, nonce, and ciphertext while reopen, inspection, HNSW ANN search, model-scoped semantic search, daemon embedding workers, and diagnostics continue to work without path/vector leaks. Focused RED/GREEN test, full `index-vector` tests, CLI embedding tests, daemon embedding worker/job tests, diagnostics test, fmt, focused clippy, diff check, public repo guard, and full local verification passed. | None for this vector snapshot artifact-encryption slice; it does not encrypt full-text snapshots or OCR-cache artifacts, select/license/distribute a real embedding model, prove large-corpus ANN latency/recall, or clear platform/signing blockers. |
| S150 | Product full-text snapshot encryption slice complete locally | A focused `index-fulltext` test first failed because published full-text snapshots were plaintext Tantivy directories and no `fulltext.snapshot.enc` envelope existed. After implementation, `publish_snapshot` validates the plaintext staging index, archives it, writes an XChaCha20-Poly1305 encrypted local envelope with an owner-only local snapshot key, removes plaintext staging before publication, and opens active/fallback published snapshots through a private temporary decrypt-and-open path. Raw `snapshots/<name>` artifacts expose only the encrypted envelope while CLI search, daemon search, diagnostics, active snapshot fallback, delete/purge, and import/search flows continue to work. Focused RED/GREEN test, full `index-fulltext` tests, related CLI/daemon full-text suites, fmt, focused clippy, diff check, public repo guard, and full local verification passed. | None for this full-text published-snapshot artifact-encryption slice; it does not prove OCR-cache encryption beyond the default SQLCipher metadata path, encrypt transient runtime decrypt directories while a search process is actively using Tantivy, prove large-corpus full-text latency with encrypted snapshot open, or clear platform/signing blockers. |
| S151 | Product OCR cache encryption proof slice complete locally | A focused diagnostics test first failed because doctor/export did not report OCR cache encryption even though the cache table lives inside the metadata store. After implementation, doctor and redacted diagnostics report `ocr cache encryption: sqlcipher` / `ocr_cache_encryption: "sqlcipher"` from the active metadata-store encryption state, and the OCR worker cache-write test proves raw default `metadata.sqlite3` lacks the SQLite header, synthetic OCR token, and engine-profile marker after a cache write. Full local verification also exposed an unrelated `s20_status_ipc` closed-port race, so that test now reserves a `127.0.0.1` port and connects to the same port on `127.0.0.2` to keep the loopback connect-failure path deterministic under concurrent fake daemons. Focused RED/GREEN test, full diagnostics, OCR handoff, and status IPC suites, fmt, focused clippy, diff check, public repo guard, and full local verification passed. | None for this OCR cache encryption proof and verification-stability slice; it does not add a separate OCR cache artifact format, encrypt active process memory, prove future bbox purge coverage, prove forensic erase, distribute OCR engines/language packs, or clear OCR quality, model, benchmark, installer, signing, notarization, or real cross-platform validation blockers. |
| S152 | Product OCR word-box purge proof complete locally | A focused delete/purge test first failed because `purge --deleted` did not report removal of persisted OCR word boxes from purged OCR page-cache rows. After implementation, the meta-store OCR page-cache purge API reports both purged cache entries and the count of OCR word boxes removed, and CLI purge prints `ocr word boxes purged` without paths or OCR text. Focused RED/GREEN test, full delete suite, full meta-store suite, fmt, focused clippy, diff check, public repo guard, and full local verification passed. | This slice covers current OCR cache word-box cleanup only. It does not prove forensic erasure of SQLite free pages, purge future unrelated PII surfaces, distribute OCR engines/language packs, prove OCR quality, or clear model, benchmark, installer, signing, notarization, or real cross-platform validation blockers. |
| S153 | Product embedding job-spec purge audit complete locally | A focused delete/purge test first failed because `purge --deleted` did not report removal of persisted embedding job specs linked to purged ingest jobs for deleted documents. After implementation, the meta-store ingest-job purge API reports both purged ingest jobs and the count of embedding job specs removed, and CLI purge prints `embedding job specs purged` without paths, model command details, or resume text. Focused RED/GREEN test, full delete suite, full meta-store suite, fmt, focused clippy, diff check, public repo guard, and full local verification passed. | This slice covers current embedding job-spec cleanup visibility only. It does not prove forensic erasure of SQLite free pages, choose/license/distribute a real embedding model, prove semantic quality, prove large-corpus ANN performance, or clear model, benchmark, installer, signing, notarization, or real cross-platform validation blockers. |
| S154 | Product Windows full-text snapshot publish stability complete locally | Hosted Windows PR #9 repeatedly failed on the previous commit with full-text snapshot publish returning `os error 33` during encrypted snapshot publication. A focused regression first failed because transient snapshot FS retry did not treat the Windows file-lock violation as retryable. After implementation, snapshot publish retries transient directory cleanup, encrypted snapshot publish cleanup, active-pointer replacement operations, and Windows file-lock diagnostics without exposing paths or payloads. Focused RED/GREEN tests, full `index-fulltext`, full import/search witness suite, fmt, focused clippy, diff check, public repo guard, and full local verification passed. | This slice covers transient Windows full-text snapshot filesystem locks only. It does not prove all platform installer/service flows, large-corpus performance, full OCR/model quality, signing, notarization, or release readiness. |
| S155 | Product private real-corpus benchmark release-gate slice complete locally | Focused benchmark gate tests first failed because `resume-benchmark gate` had no `--require-private-real-corpus` / `--require-million-scale` API and later allowed private real-corpus reports without fail-closed boundary validation or strict value typing. After implementation, synthetic smoke still requires `--allow-synthetic`, while any `dataset_kind: "private-real-corpus"` report must use strict aggregate-only JSON with local/redacted boundary fields, sha256 corpus/query digests, fixed target claim, and typed non-raw numeric/string fields; release gates can additionally require 1M scale. Focused RED/GREEN tests, full benchmark-runner tests, fmt, focused clippy, diff check, runbook policy, public repo guard, and full local verification passed. | This slice validates redacted private benchmark evidence format only. It does not create or upload real benchmark reports, run 100k/1M corpora, prove the P95 target on representative hardware, or clear real-corpus performance, model, installer, signing, notarization, or cross-platform blockers. |
| S156 | Product Windows delete-triggered full-text rebuild stability complete locally | Hosted Windows PR #9 failed after S155 in `delete_soft_tombstones_document_and_removes_it_from_default_search` with `resume-cli delete` returning `search index update failed`, pointing at encrypted full-text snapshot rebuild under transient Windows file locks. Focused regressions first failed because full-text index open did not retry Windows share violations (`os error 32`) and snapshot filesystem retry exhausted before a longer lock-release window. After implementation, full-text read-open and snapshot publish/cleanup retry the Windows 5/32/33 lock diagnostics for a bounded 1-second window. Focused RED/GREEN tests, full `index-fulltext`, the failing delete test, fmt, focused clippy, diff check, and full local verification passed. | This slice covers Windows transient file-lock stability for full-text snapshot rebuilds only. It does not prove hosted Windows CI has passed until the pushed branch check completes, nor does it clear large-corpus, model, installer, signing, notarization, or release blockers. |
| S157 | Product Windows full-text directory cleanup retry complete locally | The pushed S156 run moved the hosted Windows failure from the delete test to `multi_root_reimport_marks_missing_files_deleted_per_root`; the original delete test passed, but multi-root reimport still failed with `search index update failed`, consistent with Windows staging-directory cleanup returning `ERROR_DIR_NOT_EMPTY` / `os error 145` after partial recursive removal. A focused regression first failed because snapshot filesystem retry did not treat `DirectoryNotEmpty` as transient. After implementation, snapshot filesystem retry includes `DirectoryNotEmpty` / `os error 145` diagnostics and allows up to a bounded 5-second cleanup window while keeping full-text open retries unchanged. Focused RED/GREEN tests, full `index-fulltext`, full `s14_delete_search`, fmt, focused clippy, diff check, and full local verification passed. | This slice covers Windows transient full-text directory cleanup only. It does not prove hosted Windows CI has passed until the pushed branch check completes, nor does it clear large-corpus, model, installer, signing, notarization, or release blockers. |
| S158 | Product hosted Windows delete/search test harness stability complete locally | The pushed S157 run moved the hosted Windows failure again inside `s14_delete_search`: delete and multi-root tests passed, but purge failed during initial import with the same redacted `search index update failed`. This repeated movement across independent tests points to hosted Windows test-binary parallelism amplifying full-text CLI subprocess file-lock pressure, not a single remaining production branch. After implementation, the seven heavy delete/search integration tests in `s14_delete_search` take a local test-process mutex so their full-text rebuild/purge CLI subprocesses run serially within that test binary. Focused `s14_delete_search`, fmt, focused clippy, diff check, and full local verification passed. | This slice covers hosted Windows test harness stability only. It does not change production full-text behavior, prove hosted Windows CI has passed until the pushed branch check completes, or clear large-corpus, model, installer, signing, notarization, or release blockers. |
| S159 | Product hosted Ubuntu OCR process-test harness stability complete locally | The pushed S158 run cleared hosted macOS and Windows Platform CI, but hosted Ubuntu `rust workspace` failed in `local_command_worker_times_out_and_does_not_report_late_output`: the slow synthetic OCR command occasionally returned `EngineFailed` instead of `Timeout`. Local focused and full `s12_ocr_client` baseline passed, matching a CI-only process-test concurrency flake rather than a new product branch. After implementation, the Unix OCR/PDF command tests in `s12_ocr_client` take a local process-test mutex so timeout/cancel/process-group tests do not run concurrently inside the same test binary. Focused timeout test, full `s12_ocr_client`, fmt, focused clippy, diff check, and full local verification passed. | This slice covers hosted Ubuntu OCR command test harness stability only. It does not change production OCR/runtime behavior, prove hosted Rust Workspace CI has passed until the pushed branch check completes, or clear large-corpus, model, installer, signing, notarization, or release blockers. |
| S160 | Product hosted Windows import/search test harness stability complete locally | The pushed S159 run made hosted Ubuntu `rust workspace` and macOS Platform CI pass, but hosted Windows Platform CI failed in `local_discovery_root_preset_allows_explicit_file_budget_override_without_path_leak` with the same redacted `resume-cli: search index update failed` seen in prior Windows full-text/import flakes. After implementation, all `s9_import_search` tests take a Windows-only mutex so their CLI import/search subprocesses do not rebuild encrypted full-text snapshots concurrently inside that test binary; macOS/Linux test concurrency is unchanged. Focused `s9_import_search`, fmt, focused clippy, diff check, and full local verification passed. | This slice covers hosted Windows `s9_import_search` test harness stability only. It does not change production import/search behavior, prove hosted Windows CI has passed until the pushed branch check completes, or clear large-corpus, model, installer, signing, notarization, or release blockers. |
| S161 | Product release-readiness blocker gate complete locally | A focused CLI test first failed because `resume-cli` had no `release-readiness` command. After implementation, `resume-cli release-readiness` prints `stable release: blocked`, enumerates signing, notarization, platform installer lifecycle, real 100k/1M benchmark, OCR/model license/distribution, and cross-platform validation blockers without local path leaks, and exits nonzero so dry-run artifacts or green checks cannot be mistaken for stable release readiness. Focused release-readiness test, fmt, focused clippy, diff check, full local verification, and public repo guard passed. | This slice adds a fail-closed release-readiness blocker gate only. It does not clear signing certificates, notarization, real-corpus benchmark, licensed OCR/model distribution, platform installer/service lifecycle, or cross-platform validation blockers, so the complete production goal remains not complete. |
| S162 | Product machine-readable release-readiness evidence complete locally | A focused CLI test first failed because `resume-cli release-readiness --json` produced no JSON report. After implementation, `release-readiness --json` prints a stable `release-readiness.v1` JSON schema with `stable_release: "blocked"`, dry-run evidence status, eight blocked release criteria, details for each blocker, and the next gate, then exits nonzero without printing local data-dir paths. The focused release-readiness suite, fmt, focused clippy, diff check, full local verification, and public repo guard passed. | This slice makes the existing fail-closed release-readiness gate automation-readable only. It does not clear signing certificates, notarization, real-corpus benchmark, licensed OCR/model distribution, platform installer/service lifecycle, or cross-platform validation blockers, so the complete production goal remains not complete. |
| S163 | Product hosted Ubuntu embedder process-test stability complete locally | The pushed S162 run passed Security, macOS Platform CI, and Windows Platform CI, but hosted Ubuntu `rust workspace` failed in `local_command_embedder_times_out_and_keeps_input_file_private`: the slow synthetic local embedding command returned `EngineFailed` instead of `Timeout`. The failure matched the existing local process-test concurrency class previously seen in OCR command tests. After implementation, Unix local embedding command tests take a local process-test mutex so missing-binary, normal command, timeout, descendant-cleanup, and in-test parallel command scenarios do not run concurrently with each other in the same test binary. The hosted-failing exact test, full `s11_embedder`, fmt, focused clippy, diff check, full local verification, and public repo guard passed locally. | This slice covers hosted Ubuntu embedder command test harness stability only. It does not change production embedding runtime behavior, prove hosted Rust Workspace CI has passed until the pushed branch check completes, or clear licensed model, large-corpus, installer, signing, notarization, or release blockers. |
| S164 | Product release-readiness CI guard complete locally | A focused shell check first failed because `scripts/ci/check-release-readiness.sh` did not exist. After implementation, `verify-local` runs a local CI guard that executes `resume-cli release-readiness --json` against a private-looking synthetic data-dir, requires the command to exit nonzero, validates the `release-readiness.v1` blocked schema and all eight release blockers, checks stderr blocker messaging, and fails on local path or private marker leaks. Focused release-readiness guard, workflow guard, runbook guard, shell syntax, diff check, full local verification, and public repo guard passed. | This slice wires the blocked release-readiness gate into local CI only. It does not clear signing/notarization, real 100k/1M private benchmark, licensed OCR/model distribution, platform installer/service lifecycle, or cross-platform release validation blockers. |
| S165 | Product release workflow readiness gate complete locally | Focused guard checks first failed because `.github/workflows/release.yml` did not explicitly run `./scripts/ci/check-release-readiness.sh`; the release workflow only reached it indirectly through `verify-local`. After implementation, the Release dry-run job has a named stable-release blocked confirmation step after workspace verification, and both workflow policy plus release-readiness guard require that explicit release workflow wiring. Focused release-readiness guard, workflow guard, shell syntax, workflow YAML parse, diff check, full local verification, and public repo guard passed locally. | This slice strengthens hosted Release dry-run fail-closed behavior only. It does not clear signing, notarization, GitHub Release upload approval, real 100k/1M private benchmark evidence, licensed OCR/model distribution, installer/service lifecycle validation, or cross-platform release blockers. |
| S166 | Product hosted Windows full-text corruption-test stability complete locally | The pushed S165 run passed dependency, license, runbook, public guard, Rust Workspace, and macOS Platform CI, but hosted Windows Platform CI failed in `s8_fulltext::active_snapshot_corruption_falls_back_to_last_good_snapshot`: the test attempted to overwrite the freshly published active encrypted snapshot with `fs::write` and hit Windows `os error 33` because another process still held a region lock. Root cause was the test's corruption setup bypassing the existing transient snapshot filesystem retry policy. After implementation, the corruption write uses a bounded test helper that retries the same Windows file-lock diagnostics before asserting fallback behavior. The hosted-failing exact test, full `index-fulltext`, fmt, focused clippy, diff check, full local verification, and public repo guard passed locally. | This slice covers hosted Windows full-text test harness stability only. It does not change production full-text fallback behavior, prove hosted Windows CI has passed until the pushed branch check completes, or clear large-corpus, model, installer, signing, notarization, or release blockers. |
| S167 | Product incremental full-text snapshot update path complete locally | A focused full-text test first failed because `publish_incremental_snapshot` did not exist. After implementation, index-fulltext can synthesize a next snapshot from the active published snapshot by retaining unchanged documents, replacing same-doc_id delta documents, excluding deleted doc_ids, and publishing through the existing encrypted snapshot path. Import, OCR text indexing, and soft-delete now use this incremental snapshot document synthesis before falling back to metadata rebuild if the active snapshot is unreadable. A CLI regression corrupts the active encrypted snapshot and proves reimport rebuilds from metadata without leaking paths. Focused RED/GREEN, full index-fulltext, import-pipeline, S9 import/search, S14 delete/search, S15 OCR handoff, fmt, focused clippy, diff check, full local verification, and public repo guard passed locally. | This slice reduces full-text update work and preserves corrupt-active fallback behavior for synthetic local paths only. It does not prove million-scale incremental latency, real-corpus performance, cross-platform watcher soak, platform installer/service validation, signing, notarization, OCR/model licensing, or release readiness. |
| S168 | Product query telemetry observability complete locally | A focused meta-store test first failed because `record_query_observation` and `StoreStatusSummary.query_latency` did not exist. After implementation, metadata schema V17 adds a bounded `query_observation` table that stores mode, duration, result count, and timestamp only, never query text. Successful local CLI searches and daemon full-text IPC searches record best-effort samples. Local status, daemon status IPC, doctor, and `export-diagnostics --redact` report aggregate query telemetry sample count plus P50/P95/P99 and last result count without raw queries or paths. Focused RED/GREEN, full meta-store, S4 status, S9 import/search, S13 diagnostics, daemon S20 status IPC, daemon S48 search IPC, fmt, focused clippy, diff check, full local verification, and public repo guard passed locally. | This slice adds runtime observability only. It does not prove the `<200ms` hybrid P95 target, real 100k/1M corpus latency, real semantic/vector quality, cross-platform performance evidence, installer/service validation, signing, notarization, OCR/model licensing, or stable release readiness. |
| S169 | Product hardware fault-drill simulation coverage complete locally | A focused CLI fault test first failed because `fault-simulate` did not accept `battery-mode`. After implementation, the safe local fault-simulation CLI accepts `battery-mode --battery-state <battery|ac>` and `external-drive-disconnect --drive-state <disconnected|mounted>`, prints redacted degradation/recovery guidance, does not touch private paths, and explicitly marks the real hardware drill as blocked. Doctor and `export-diagnostics --redact` advertise the two new hooks, `release-readiness` plus its CI guard include a `hardware fault drills` blocker, and the fault-injection runbook/guard document the safe probes so safe simulation cannot be mistaken for release evidence. Focused RED/GREEN, full fault-injection tests, diagnostics tests, release-readiness tests, release-readiness guard, runbook guard, fmt, focused clippy, diff check, full local verification, and public repo guard passed locally. | This slice covers safe local synthetic drill surfaces only. It does not perform real battery-mode switching, physically disconnect external drives, prove platform-specific power/storage behavior, clear destructive ENOSPC/service-level fault drills, or clear Windows/macOS validation, signing, notarization, installer lifecycle, real benchmark, OCR/model licensing, or stable release readiness blockers. |
| S170 | Product active import daemon kill/restart recovery complete locally | A focused daemon scheduler test first failed because restart drills could not lower stale-running recovery or retry-backoff thresholds, so a freshly killed running import task could not be recovered and reprocessed deterministically. After implementation, `resume-daemon run --work-imports` accepts `--stale-import-task-seconds <n>` and `--import-retry-backoff-seconds <n>` with production defaults unchanged at 15 minutes and 60 seconds. The regression starts a foreground import over 1,024 synthetic files, waits until the task is `Running`, kills the daemon, restarts with zero-second drill thresholds, proves one stale task recovered, one import processed, 1,024 searchable documents indexed, full-text search succeeds, and stdout/stderr omit data paths. Focused RED/GREEN, full S4 daemon scheduler tests, fmt, full local verification, and public repo guard passed locally. | This slice covers synthetic active-import kill/restart recovery only. It does not prove destructive service-level chaos, real external storage interruption, 100k/1M import recovery latency, cross-platform service lifecycle, installer validation, signing, notarization, OCR/model licensing, or stable release readiness. |
| S171 | Product private local PDF/Word witness rerun complete locally | The explicit-root witness was rerun against the user-authorized local resume sample directory for PDF/Word import plus redacted search and field probes. A second bounded OCR witness used local `tesseract` and `pdftoppm` with document/page limits. Both runs completed locally, printed only redacted aggregate status, removed private temporary witness data, and did not emit source paths, filenames, raw text, private queries, diagnostics, or committed counts. | This slice is private local witness evidence only. It does not upload evidence, commit sample counts, prove full-library OCR completion, prove real-corpus quality/latency targets, clear platform installer/service validation, signing, notarization, OCR/model licensing, or stable release readiness. |
| S172 | Product hosted Windows full-text staging-orphan test stability complete locally | PR #9 hosted Windows Platform CI failed in `published_snapshot_becomes_active_without_reading_staging_orphans` during the synthetic staging-orphan fixture write with Windows `os error 33`. Root cause: the test used direct `fs::write` immediately after publishing a snapshot, while the same test file already has a bounded retry helper for transient Windows file locks. After implementation, the fixture write uses `write_snapshot_test_file_with_retry`, preserving the test's behavior while tolerating transient setup locks. The hosted-failing exact test, full `index-fulltext`, fmt, diff check, public repo guard, and full local verification passed locally. | This slice covers hosted Windows synthetic test harness stability only. It does not change production full-text behavior, prove hosted Windows CI has passed until the pushed branch check completes, or clear large-corpus, installer/service, signing, notarization, OCR/model licensing, or stable release blockers. |
| S173 | Product Windows service dry-run evidence surface complete locally | A focused service lifecycle test first failed because `resume-cli service` did not accept `--platform windows-service`, and focused release-readiness tests first failed because Windows service lifecycle was not tracked separately from MSI installer lifecycle. After implementation, explicit Windows Service dry-run mode reports redacted install/status/start/stop/uninstall command plans without touching LaunchAgent files, requiring `HOME`, or exposing local paths, and release-readiness plus its CI guard include a separate `Windows service lifecycle` blocker. Focused RED/GREEN service and readiness tests, service lifecycle suite, readiness guard, runbook guard, fmt, focused clippy, diff check, public repo guard, and full local verification passed locally. | This slice adds local redacted Windows Service command-plan evidence only. It does not register a Windows service, prove administrator-elevated service install/start/stop/status/uninstall, prove recovery/rollback/upgrade behavior, validate MSI lifecycle, or clear signing, notarization, platform validation, real benchmark, OCR/model licensing, or stable release blockers. |
| S174 | Product OCR runtime manifest validation gate complete locally | A focused OCR manifest test first failed because `resume-cli` had no `ocr validate-manifest` command. After implementation, local OCR runtime manifests use schema `resume-ir.ocr-runtime-manifest.v1`, require a runtime pack id, reviewed local component licenses, artifact sha256 checks for OCR engines/renderers/language packs, optional reviewed language-pack entries, and redacted output that omits runtime bytes, local paths, and full digests. The OCR worker runbook, release blocker runbook, runbook guard, and release-readiness OCR blocker detail now include the OCR runtime manifest gate. Focused RED/GREEN OCR manifest tests, release-readiness tests, runbook/readiness guards, fmt, focused clippy, public repo guard, and full local verification passed locally. | This slice adds local OCR runtime distribution governance only. It does not bundle or approve Tesseract/Poppler/language packs, prove non-English OCR quality, prove full-library scanned-resume OCR, prove large-corpus OCR throughput, validate platform installers/services, or clear signing, notarization, OCR/model licensing, benchmark, or stable release blockers. |
| S175 | Product hot-index hybrid private benchmark gate complete locally | A focused benchmark gate test first failed because a private real-corpus benchmark report without `query_mode`, retrieval-layer, hot-index, and hot-path exclusion evidence was accepted as release evidence. After implementation, private real-corpus benchmark reports must now prove `query_mode: hybrid`, `retrieval_layers: fulltext+field+vector+rrf`, `hot_index: true`, and false hot-path OCR/parsing/heavy-model-inference flags, while preserving the existing redacted local aggregate boundary and private corpus/query-set digests. Release blocker docs, runbook guard, and release-readiness blocker detail now name hot-index hybrid evidence explicitly. Focused RED/GREEN benchmark gate tests and CLI private report acceptance passed locally. | This slice tightens release evidence validation only. It does not run 100k or 1M real-corpus benchmarks, prove `<200ms` P95 on representative hardware, provide licensed embedding model distribution, prove semantic/vector quality, clear OCR/model licensing, platform validation, signing, notarization, or stable release readiness. |
| S176 | Product private business field-quality release gate complete locally | A focused field-quality gate test first failed because `FieldQualityGateConfig` had no `require_private_business_labeled` mode. After implementation, `resume-benchmark field-gate --require-private-business-labeled` rejects ordinary labeled reports, accepts only strict `private-business-labeled` redacted local aggregate reports, requires dataset and annotation manifest digests, false raw-data/path/field-value/sample-ID booleans, the `resume-ir.fields.v1` taxonomy, and production field metrics for email, phone, school, degree, company, title, skill, and date ranges. Release-readiness plus its CI guard now include a `field extraction quality` blocker, and the release blocker runbook documents the private field-quality evidence gate. Focused RED/GREEN field-quality tests, full benchmark-runner tests, release-readiness tests, readiness/runbook guards, fmt, diff check, focused clippy, public repo guard, and full local verification passed locally. | This slice tightens release evidence validation only. It does not create or upload private labels, run real business field-quality evaluation, prove production field F1 on representative resumes, improve extraction rules/models, clear OCR/model licensing, clear platform validation, or clear stable release readiness. |
| S177 | Product dedupe-quality evaluator and release gate complete locally | A focused dedupe-quality test first failed because `benchmark-runner` had no `run_dedupe_quality_jsonl`, `DedupeQualityGateConfig`, or `evaluate_dedupe_quality_gate_json` API. After implementation, `resume-benchmark dedupe-quality` scores labeled profile pairs through the existing `rank-fusion` soft-dedupe algorithm, emits only aggregate precision/recall/F1 and pair counts, and omits names, schools, companies, skills, sample IDs, document IDs, paths, and raw resume text. `resume-benchmark dedupe-gate --require-private-business-labeled` now rejects ordinary labeled reports and accepts only strict `private-business-labeled` redacted local aggregate reports with dataset and annotation manifest digests, false raw-data/path/profile-value/sample-ID/document-ID booleans, the `resume-ir.dedupe.v1` taxonomy, and aggregate dedupe metrics. Release-readiness plus its CI guard now include a `dedupe quality` blocker, and the release blocker runbook documents the private dedupe-quality evidence gate. Focused RED/GREEN dedupe-quality tests, full benchmark-runner tests, release-readiness tests, readiness/runbook guards, fmt, and focused clippy passed locally. | This slice adds quality evaluation and tightens release evidence validation only. It does not create or upload private labels, run real business dedupe-quality evaluation, prove production dedupe precision/recall on representative resumes, implement candidate merge review workflows, clear OCR/model licensing, clear platform validation, or clear stable release readiness. |
| S178 | Product local candidate-review workflow complete locally | A focused CLI test first failed because `resume-cli` did not recognize `candidate-review`. After implementation, `resume-cli candidate-review list` computes bounded same-name soft-dedupe suggestions from persisted metadata and prints only redacted version IDs, counts, confidence, and `paths: <redacted>`; `candidate-review merge` creates a manual local candidate for two or more unassigned searchable versions and default search folds them; `candidate-review split` clears those assignments and restores independent default search results. `MetaStore::unassign_candidate_versions` clears assignments transactionally and refreshes candidate version counts. Focused RED/GREEN CLI, full candidate-folding CLI, import candidate assignment, full meta-store, rank-fusion, fmt/diff/runbook/public guards, focused clippy, and full local verification passed locally. | This slice adds local manual review/merge/split workflow only. It does not prove dedupe precision/recall, create private business labels, resolve conflicting multi-contact candidates, add a UI, prove million-corpus review-list latency, clear dedupe-quality evidence, clear OCR/model licensing, clear platform validation, or clear stable release readiness. |
| S179 | Product scan-error breakdown diagnostics complete locally | Focused tests first failed because `MetaStore::import_scan_error_breakdown` and CLI scan-error breakdown output did not exist. After implementation, metadata can aggregate persisted import scan errors by redacted kind and filesystem operation, and local `status`, `doctor`, and `export-diagnostics --redact` report those aggregates without paths, path digests, filenames, or raw resume text. Focused RED/GREEN meta-store and CLI tests, full meta-store, full S9 import/search, S13 diagnostics, fmt/diff/public guards, focused clippy, and full local verification passed locally. | This slice improves scan-error observability only. It does not change scan retry policy, prove whole-machine discovery coverage, validate real external-drive disconnects, clear cross-platform watcher proof, clear large-corpus import evidence, clear OCR/model licensing, or clear stable release readiness. |
| S180 | Product hosted Windows full-text snapshot read-lock stability complete locally | PR #9 hosted Windows Platform CI failed in `s8_fulltext::incremental_snapshot_inherits_replaces_and_excludes_documents`: the first `publish_snapshot` returned Windows `os error 33` while reading freshly written snapshot files. A focused regression first failed because `read_snapshot_file_with_retry` did not exist. After implementation, snapshot archive file reads, encrypted snapshot envelope reads, and encrypted-header probes use the existing bounded transient Windows lock retry policy. Focused RED/GREEN, the hosted-failing exact test, full `index-fulltext`, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice covers hosted Windows full-text snapshot file-read stability only. It does not prove hosted Windows CI has passed until the pushed branch check completes, nor does it clear large-corpus, installer/service, signing, notarization, OCR/model licensing, or stable release blockers. |
| S181 | Product candidate contact conflict review complete locally | Focused tests first failed because `MetaStore::candidate_contact_conflicts` did not exist and automatic hashed-contact assignment still errored with `candidate.contact_hash` when email and phone matched different candidates. After implementation, schema V18 persists redacted `candidate_contact_conflict` rows with only version and candidate IDs, conflicting hashed-contact assignment returns `Ok(None)` without auto-folding, successful later assignment clears stale conflicts, and `resume-cli candidate-review conflicts --limit <count>` lists reviewable conflicts with contact values, contact hashes, and paths redacted. Focused RED/GREEN meta-store and CLI tests, full meta-store, full candidate-folding CLI tests, metadata key backup/rotation schema regression tests, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice adds local redacted conflict surfacing only. It does not auto-merge conflicting candidates, add a UI, prove real business dedupe-quality results, clear broader field normalization, clear future non-cache PII purge coverage, prove forensic erase, or clear stable release blockers. |
| S182 | Product deleted-document import-root path purge complete locally | Focused tests first failed because empty import roots retained `import_task` and `import_scan_scope` path metadata after document purge. After implementation, deleted-document purge removes import tasks whose roots contain deleted documents and no visible documents, cascading scan scope, scan error, and cancellation rows; roots with live documents are retained, and CLI purge reports only redacted counters. Focused RED/GREEN, full meta-store, full delete/search CLI tests, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice covers empty-root import task path cleanup only. It does not prove forensic erase, all possible future non-cache PII surfaces, real corpus purge audits, or stable release blockers. |
| S183 | Product hosted Windows import-root purge path matching complete locally | PR #9 hosted Windows Platform CI failed in `purge_deleted_removes_empty_import_root_task_paths_without_path_leak` because the purge output did not include `purged import tasks: 1`; the empty-root task was not matched when the import task root used a Windows canonical path shape and the document path used normalized slash/file-URI storage. A focused meta-store regression first failed for a `\\?\\C:\\...` root against `file://c:/...` / `c:/...` document paths. After implementation, import-root purge matching builds internal comparison keys that strip local file URI prefixes, remove Windows verbatim prefixes, normalize separators and dot segments, and compare Windows drive/UNC paths case-insensitively. Focused RED/GREEN, full meta-store, full delete/search CLI tests, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice fixes Windows path-shape matching for the S182 purge only. It does not prove hosted Windows CI has passed until the pushed branch check completes, prove forensic erase, audit real corpus purge behavior, or clear stable release blockers. |
| S184 | Product certificate alias extraction complete locally | Focused tests first failed because sectioned certificate aliases under `Certifications` / `认证` were missed while the Chinese header itself was persisted as a certificate. A follow-up focused test failed because fullwidth-colon labeled lines such as `认证：PMP` produced a span inside the delimiter byte sequence. After implementation, extractor-rules treats certificate headers as bounded context, extracts high-signal aliases such as PMP, CKA, CISSP, CFA Level I, AWS/Azure/Kubernetes certifications, and CPA with canonical normalized values and exact span evidence, suppresses section headers, and import persists those certificate mentions without CLI output/path/contact leaks. Focused RED/GREEN, full extractor-rules, full import-pipeline, full persisted-field CLI tests, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice improves certificate dictionaries/normalization only. It does not prove real business field-quality metrics, broad certificate dictionaries, multilingual normalization coverage, real corpus results, or stable release readiness. |
| S185 | Product skill alias extraction complete locally | Focused tests first failed because skill aliases under section headers such as `Skills` / `技术栈` were missed and therefore not persisted through import. After implementation, extractor-rules treats skill headers as bounded context, extracts high-signal aliases such as TypeScript, PostgreSQL, K8s/Kubernetes, Go/Golang, Redis, React, and Node.js with canonical normalized values and exact span evidence, suppresses section headers, and import persists those skill mentions without CLI output/path/contact leaks. Focused RED/GREEN, full extractor-rules, full import-pipeline, full persisted-field CLI tests, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice improves skill dictionaries/normalization only. It does not prove real business field-quality metrics, broad skill dictionaries, multilingual normalization coverage, real corpus results, or stable release readiness. |
| S186 | Product Chinese date-range extraction complete locally | Focused tests first failed because `2020年1月 - 2024年3月` produced no date-range or years-experience field, and import therefore persisted no DateRange/YearsExperience mentions for that evidence. After implementation, extractor-rules normalizes explicit Chinese year/month ranges to the existing `YYYY-MM/YYYY-MM` schema, keeps exact span evidence, derives years-experience from the normalized range, and import persists both DateRange and YearsExperience mentions without CLI output/path/contact leaks. Focused RED/GREEN, full extractor-rules, full import-pipeline, full persisted-field CLI tests, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice improves explicit Chinese date-range normalization only. It does not implement present/current-date ranges, prove real business field-quality metrics, broad multilingual date normalization coverage, real corpus results, or stable release readiness. |
| S187 | Product China mobile phone extraction complete locally | Focused tests first failed because compact `13800138000` produced no phone field and `139 0013 8001` was normalized as `+13900138001` instead of `+8613900138001`; import therefore persisted only one redacted phone mention. After implementation, extractor-rules recognizes China mainland mobile numbers with optional `+86`/`0086`, compact or separated local forms, claims those spans before the general phone rule to prevent misnormalization, normalizes them to E.164 `+86...`, and import persists the resulting phone mentions as `<redacted:phone>` without normalized contact plaintext or CLI output/path/contact leaks. Focused RED/GREEN, full extractor-rules, full import-pipeline, full persisted-field CLI tests, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice improves phone extraction/normalization only. It does not prove real business field-quality metrics, all international numbering plans, broad phone-format coverage, real corpus results, or stable release readiness. |
| S188 | Product open-ended present date-range extraction complete locally | Focused tests first failed because `2020年1月 - 至今`, `Jan 2021 - Present`, and `2022.03 - Current` produced no DateRange mentions, and import therefore persisted no DateRange/YearsExperience mentions for that evidence. After implementation, extractor-rules recognizes numeric, Chinese year/month, and English named-month open-ended present/current ranges, normalizes them to `YYYY-MM/PRESENT`, preserves exact span evidence, and derives years-experience from the current local month while import persists DateRange/YearsExperience mentions without CLI output/path/contact leaks. Focused RED/GREEN, full extractor-rules, full import-pipeline, full persisted-field CLI tests, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice improves present/current date normalization only. It does not prove real business field-quality metrics, broad multilingual date normalization coverage, all date idioms, real corpus results, or stable release readiness. |
| S189 | Product labeled company/title extraction complete locally | Focused tests first failed because `Company: Synthetic Commerce Inc.` normalized to `company: synthetic commerce`, `公司：合成科技有限公司` normalized with the label and unstripped Chinese suffix, and persisted company/title raw values still contained `Company:`/`公司：`/`Title:`/`职位：` labels. After implementation, extractor-rules strips common English and Chinese company/title labels before validation, points spans at only the field values, normalizes Chinese company suffixes such as `有限公司`, and import persists the stripped Company/Title mentions without CLI output/path/contact leaks. Focused RED/GREEN, full extractor-rules, full import-pipeline, full persisted-field CLI tests, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice improves labeled company/title extraction only. It does not prove real business field-quality metrics, broad company/title dictionaries, all multilingual employer/title idioms, real corpus results, or stable release readiness. |
| S190 | Product broader title alias extraction complete locally | Focused tests first failed because frontend, full-stack, machine-learning, data-scientist, DevOps, QA, engineering-manager, and solutions-architect title evidence produced no Title mentions, and import therefore persisted none of those title aliases. After implementation, extractor-rules maps those high-signal English and Chinese role families to canonical title values, keeps exact span evidence, rejects certificate-looking title candidates such as `AWS Certified Solutions Architect`, and import persists the title mentions without CLI output/path/contact leaks. Focused RED/GREEN, full extractor-rules, full import-pipeline, full persisted-field CLI tests, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice improves title dictionaries/normalization only. It does not prove real business field-quality metrics, broad multilingual title coverage, model-based role inference, real corpus results, or stable release readiness. |
| S191 | Product hosted Windows daemon-kill readiness test stability complete locally | PR #9 hosted Windows Platform CI failed in `foreground_daemon_can_be_killed_and_restarted_without_path_leak` because the test killed the foreground daemon after metadata readiness but before stdout readiness evidence was reliably captured, so `resume-daemon foreground ready` was absent from the killed-child stdout assertion. After implementation, the test uses a bounded background stdout reader to wait for the ready line before killing the daemon, then joins the reader after process exit and preserves the restart and path-redaction assertions. Focused exact daemon-kill test, full `resume-daemon`, focused daemon clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice stabilizes hosted Windows daemon-kill test evidence only. It does not change production daemon behavior, prove hosted Windows CI has passed until the pushed branch check completes, or clear platform installer/service, signing, notarization, OCR/model licensing, benchmark, or stable release blockers. |
| S192 | Product labeled school/degree extraction complete locally | Focused tests first failed because labeled school values kept `School:` / `学校：` in normalized values and persisted school raw values still contained label delimiters. After implementation, extractor-rules strips common English and Chinese school labels, points school spans at value text only, normalizes school whitespace/case, maps degree aliases such as MSc, BSc, PhD, `博士研究生`, and `硕士研究生` to canonical degree values, and prevents generic degree aliases from duplicating labeled degree spans. Import persists the stripped School/Degree mentions without CLI output/path/contact leaks. Focused RED/GREEN, full extractor-rules, full import-pipeline, full persisted-field CLI tests, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice improves school/degree dictionaries/normalization only. It does not prove real business field-quality metrics, broad school dictionaries, school tier/985/211 extraction, broad multilingual degree coverage, real corpus results, or stable release readiness. |
| S193 | Product context-aware degree extraction complete locally | Focused tests first failed because `MS SQL` in a skill section was extracted and persisted as a master's Degree mention. After implementation, extractor-rules extracts unlabeled degree aliases only inside bounded education sections while still extracting explicitly labeled degree lines anywhere, and import no longer persists `MS` from skill evidence as Degree while still persisting education-context `Bachelor of Science`. The field-quality benchmark fixture was updated to put synthetic bachelor evidence inside an `Education` section instead of relying on global degree scanning. Focused RED/GREEN, full extractor-rules, full import-pipeline, full persisted-field CLI tests, focused benchmark regression, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice reduces degree false positives only. It does not prove real business field-quality metrics, broad degree-context coverage, all multilingual education layouts, broad school dictionaries, school tier/985/211 extraction, real corpus results, or stable release readiness. |
| S194 | Product hosted Windows full-text commit retry stability complete locally | PR #9 hosted Windows Platform CI failed in `top_n_snippets_are_generated_only_for_returned_hits` at `index.commit().unwrap()` with Tantivy `Access is denied. (os error 5)`. Root cause: full-text open and snapshot filesystem operations already had bounded transient Windows retry handling, but `FullTextIndex::commit` called `writer.commit()` directly. A focused regression first failed before the mutation retry helper existed. After implementation, full-text writer commits use the same bounded transient Windows operation policy for access-denied diagnostics. Focused RED/GREEN, the hosted-failing exact test, full `index-fulltext`, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice stabilizes transient Windows full-text commit access-denied errors only. It does not prove hosted Windows CI has passed until the pushed PR check completes, nor does it clear large-corpus, platform validation, installer/service, signing, notarization, OCR/model licensing, or stable release blockers. |
| S195 | Product hosted Windows daemon startup-queue test stability complete locally | The pushed S194 run cleared the previous full-text commit failure but hosted Windows Platform CI then failed in `foreground_import_scheduler_processes_task_enqueued_after_startup`: after the test waited for daemon metadata readiness, its helper reopened the same live data-dir and reran migrations, producing `MetaStoreError { kind: Migration }` while the foreground import worker loop was active. Root cause: the test was doing redundant migration DDL after daemon readiness had already proved the store was migrated. After implementation, the startup-queue test uses a ready-store helper that opens the migrated data-dir and inserts the queued task without rerunning migrations, while the existing helper still migrates fresh stores for pre-daemon setup. Hosted-failing exact test, full `s4_daemon`, focused daemon clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice stabilizes hosted Windows daemon test harness startup-queue evidence only. It does not change production daemon behavior, prove hosted Windows CI has passed until the pushed PR check completes, or clear platform installer/service, signing, notarization, OCR/model licensing, benchmark, large-corpus, or stable release blockers. |
| S196 | Product private business vector-quality release evidence gate complete locally | Focused regression first failed because `VectorQualityGateConfig::require_private_business_labeled` did not exist. After implementation, vector-quality gates can require `private-business-labeled` reports, reject ordinary labeled reports for release evidence, reject private reports that contain raw query/candidate/path/sample/candidate-id/vector surfaces or missing dataset/annotation/model manifest digests, and accept only redacted aggregate private-local vector-quality evidence with recall/MRR/NDCG metrics and fixed taxonomy. Focused RED/GREEN, full `benchmark-runner`, focused clippy, fmt, diff check, public guard, and full local verification passed locally. | This slice adds the release-evidence boundary only. It does not create or upload private labels, run a real vector-quality evaluation, select or license a production model, prove model distribution, prove real semantic quality, prove real 100k/1M ANN latency, validate platforms, or clear stable release readiness. |
| S197 | Product release-readiness vector-quality blocker complete locally | Focused release-readiness tests first failed because stable-release blockers did not include `vector quality`: text output omitted `vector quality: blocked`, and JSON still reported 12 blockers. After implementation, `release-readiness` reports a separate `vector quality` blocker, the release-readiness CI check asserts the label and detail, and the release blockers runbook plus runbook policy guard document the private business `vector-gate` evidence boundary with dataset/annotation/model manifest digests and no raw query/candidate/path/id/vector payloads. Focused RED/GREEN, full release-readiness CLI tests, focused clippy, fmt, diff check, public guard, policy scripts, and full local verification passed locally. | This slice wires vector-quality evidence into release readiness only. It does not create private labels, run real semantic evaluation, select/license/distribute a model, prove real vector quality or ANN latency, clear field/dedupe/benchmark/platform blockers, or make stable release ready. |
| S198 | Product private real-corpus OCR throughput release evidence gate complete locally | Focused OCR gate tests first failed because `OcrThroughputGateConfig` had no private-real-corpus release mode and `resume-benchmark ocr-gate` did not accept `--require-private-real-corpus`. After implementation, OCR throughput gates can reject synthetic reports for release evidence, accept only strict `private-real-corpus` redacted local aggregate reports with dataset/OCR-runtime/renderer/language-pack manifest digests, aggregate latency/throughput metrics, target claim `ocr_throughput_target_met`, and explicit false raw OCR text/page image/resume-path/document-ID/page-ID/command-path booleans. Release-readiness now reports an `OCR throughput` blocker, and the release blockers runbook plus policy guards document the private OCR evidence boundary. Focused RED/GREEN, full `benchmark-runner`, release-readiness tests, runbook/readiness checks, focused clippy, fmt/diff checks, public guard, and full local verification passed locally. | This slice adds the release-evidence validator and blocker only. It does not run a real OCR benchmark, upload or sanitize private OCR reports, choose/license/distribute OCR runtimes or language packs, prove full-library scanned OCR throughput, validate platforms, sign/release, or make stable release ready. |
| S199 | Product synthetic query benchmark streaming complete locally | A focused CLI regression first failed because redacted synthetic benchmark reports did not include `generation_mode: "streaming"`. After implementation, `run_synthetic_query_benchmark` streams generated synthetic documents directly into the full-text index instead of collecting the full document set into a `Vec`, and synthetic benchmark reports expose the streaming generation mode for runbook audit. The release blockers runbook and runbook policy guard now require that boundary. Focused RED/GREEN, full `benchmark-runner`, runbook guard, focused clippy, fmt/diff checks, public guard, and full local verification passed locally. | This slice improves scalable synthetic pressure-run feasibility only. It does not run a real 100k/1M private benchmark, prove production P95, produce representative real-corpus evidence, clear OCR/model licensing, validate platforms, or make stable release ready. |
| S200 | Product school-tier extraction and filtering complete locally | Focused tests first failed because `FieldType::SchoolTier`, rank-fusion `SchoolTier` filters, and import mapping for the new field did not exist. After implementation, extractor-rules recognizes explicit `985`, `211`, `双一流`, overseas, and regular-school evidence inside bounded education/school context, persists canonical `school_tier` entity mentions, wires the SQLite entity whitelist and benchmark field labels, and supports `--school-tier` filtering through direct CLI search plus CLI/daemon IPC search payloads. Focused RED/GREEN, full extractor/rank/import/meta-store/benchmark tests, persisted-field and search-IPC CLI tests, daemon search-IPC tests, focused clippy, and fmt passed locally. | This slice uses synthetic/temp fixtures only. It does not infer school tier from broad school dictionaries, prove real business field-quality metrics, evaluate private resume corpora, clear broad multilingual education coverage, or make stable release ready. |
| S201 | Product field-quality school-tier release gate complete locally | Focused RED tests first failed because private-business field-quality reports without `school_tier` metrics were accepted by both the library gate and CLI gate. After implementation, `PRODUCTION_FIELD_QUALITY_THRESHOLDS` requires `school_tier` metrics for private business release evidence, complete strict reports include the metric, reports missing it are rejected, and the release blockers runbook documents the updated field evidence boundary. Focused RED/GREEN, full `benchmark-runner`, and runbook guard passed locally. | This slice tightens release-evidence validation only. It does not create private labels, run real business field-quality evaluation, prove production `school_tier` F1 on representative resumes, infer tier from broad school dictionaries, or make stable release ready. |
| S202 | Product unknown school-tier filtering complete locally | Focused tests first failed because `SchoolTier::Unknown` did not match profiles with no school-tier evidence, CLI `--school-tier unknown` returned no target when known-tier decoys consumed the full-text top-k window, `MetaStore` had no missing-entity prefilter helper, and daemon IPC had the same post-filter-only top-k gap. After implementation, `unknown` means no high-confidence persisted `school_tier` mention on a searchable visible version, known and unknown tier filters are unioned before intersecting with other field filters, and both CLI and daemon full-text search apply the prefilter before top-k truncation. Focused RED/GREEN and related rank/meta-store/CLI/daemon suites passed locally. | This slice uses synthetic/temp fixtures only. It does not infer unknown as an extracted entity, read real resumes, prove broad school dictionaries, produce private field-quality evidence, or make stable release ready. |
| S203 | Product certificate search filtering complete locally | Focused tests first failed because rank-fusion had no certificate profile/filter API, CLI `--certificate` was rejected by search usage, CLI IPC did not emit `certificates_any`, and daemon IPC ignored certificate filters until after full-text top-k retrieval. After implementation, certificate filters normalize common certificate aliases to extractor canonical values, CLI supports `--certificate` and `--certificates-any`, CLI/daemon IPC carry `certificates_any`, persisted profiles hydrate certificate mentions, and both CLI and daemon prefilter certificate doc IDs before full-text top-k truncation. Focused RED/GREEN and related rank/CLI/daemon suites passed locally. | This slice uses synthetic/temp fixtures only. It does not broaden certificate extraction beyond existing aliases, implement certificate level/date filters, produce private field-quality evidence, or make stable release ready. |
| S204 | Product hosted Rust workspace school-tier debug assertion stability complete locally | PR #9 hosted Rust workspace failed in `import_persists_school_tier_mentions_and_filters_search_without_output_leaks` because the test checked that the entire `EntityMention` Debug string did not contain `985`; the Debug string already redacts raw and normalized values, but opaque hex IDs can legitimately contain that digit sequence. The test now asserts the `raw_value` and `normalized_value` Debug fields are redacted while keeping the normalized school-tier value and filtered-search assertions. Focused persisted-field tests, fmt, focused clippy, diff check, public guard, and full local verification passed. | This slice stabilizes a flaky privacy assertion only. It does not change production search/filter behavior, read private resumes, broaden school-tier extraction, prove hosted CI has passed until PR #9 reruns, or make stable release ready. |
| S205 | Product company/title search filtering complete locally | Focused tests first failed because rank-fusion had no company/title profile filter API, CLI search rejected `--company` and `--title`, CLI IPC did not emit `companies_any` or `titles_any`, and daemon IPC ignored those filters until after full-text top-k retrieval. After implementation, company filters normalize common legal suffixes, title filters normalize common English and Chinese aliases, CLI supports `--company`/`--companies-any` and `--title`/`--titles-any`, CLI/daemon IPC carry `companies_any` and `titles_any`, persisted profiles hydrate company/title entity mentions, and both CLI and daemon prefilter matching document IDs before full-text top-k truncation. Focused RED/GREEN and related rank/CLI/daemon suites passed locally. | This slice uses synthetic/temp fixtures only. It does not broaden company or title extraction beyond currently persisted entity evidence, prove real business field-quality metrics, evaluate private resume corpora, clear multilingual coverage, or make stable release ready. |
| S206 | Product release-readiness fault-drill blocker coverage complete locally | Focused release-readiness tests and the release-readiness CI guard first failed because the current blocker detail did not explicitly include actual ENOSPC and service-level daemon kill drills, and the release blockers runbook did not list hardware fault drills in the current blocked items. After implementation, `release-readiness` text/JSON and the runbook consistently keep hardware fault drills blocked until actual ENOSPC, service-level daemon kill, battery-mode, and external-drive disconnect drills are proven on release platforms. Focused RED/GREEN, release-readiness guard, runbook guard, fmt, and focused clippy passed locally. | This slice tightens fail-closed release readiness evidence only. It does not run destructive ENOSPC tests, install or kill a real platform service, switch real battery state, disconnect external drives, clear platform validation, signing, notarization, benchmark, OCR/model licensing, or make stable release ready. |
| S207 | Product school search filtering complete locally | Focused tests first failed because rank-fusion lacked school profile/filter API, CLI search rejected `--school`, CLI IPC did not emit `schools_any`, and daemon IPC ignored school filters until after full-text top-k retrieval. After implementation, school filters normalize persisted school evidence, CLI supports `--school`/`--schools-any`, CLI/daemon IPC carry `schools_any`, persisted profiles hydrate school mentions, and both CLI and daemon prefilter school document IDs before full-text top-k truncation. Focused RED/GREEN, related rank/CLI/daemon suites, fmt, diff check, public guard, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not broaden school extraction beyond currently persisted evidence, prove real business field-quality metrics, evaluate private resume corpora, clear broad school dictionaries, or make stable release ready. |
| S208 | Product date-range search filtering complete locally | Focused tests first failed because rank-fusion had no date-range profile/filter API, metadata could not return searchable document IDs by overlapping `date_range` evidence, CLI search rejected `--date-range-overlaps`, CLI IPC did not emit `date_range_overlaps`, and daemon IPC ignored date ranges until after full-text top-k retrieval. After implementation, `DateRange` supports `YYYY-MM/YYYY-MM`, `YYYY-MM..YYYY-MM`, and `YYYY-MM/PRESENT`; metadata prefilters visible searchable documents by overlapping persisted `date_range` mentions; CLI supports `--date-range-overlaps`; CLI/daemon IPC carry and parse `date_range_overlaps`; persisted profiles hydrate date ranges; and both CLI and daemon prefilter date-range document IDs before full-text top-k truncation. Focused RED/GREEN, related meta/rank/CLI/daemon suites, fmt, and focused clippy passed locally. | This slice uses synthetic/temp fixtures only. It does not add separate `edu_start`/`edu_end`/`work_start`/`work_end`/`certificate_date` columns, infer certificate-specific dates, prove real business date-range F1, evaluate private resume corpora, clear broad multilingual date coverage, or make stable release ready. |
| S209 | Product contact search filtering complete locally | Focused tests first failed because metadata could not return searchable document IDs from candidate contact hashes, CLI search rejected `--email`/`--phone`, CLI IPC could not hash contact filters before submitting a request, and daemon IPC ignored `contact_hashes_any` until after full-text top-k retrieval. After implementation, CLI contact filters normalize email/phone locally, hash them with the existing data-dir contact HMAC key, never put raw contacts in `SearchFilters` or IPC, CLI/daemon IPC carry only `contact_hashes_any`, metadata prefilters visible searchable candidate-assigned documents by email/phone hash, and full-text/semantic/hybrid local search plus daemon full-text IPC apply the metadata prefilter before result filtering. Focused RED/GREEN, related meta/rank/CLI/daemon suites, fmt, diff check, public guard, focused clippy, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not expose fuzzy contact matching, prove real business contact recall, evaluate private resume corpora, guarantee manual IPC against a different data-dir contact key, clear broader field-quality evidence blockers, or make stable release ready. |
| S210 | Hosted Rust workspace contact IPC assertion stability complete locally | PR #9 hosted `rust workspace` failed in `search_ipc_hashes_contact_filters_before_submitting_request` because the test compared `contact_hashes_any` arrays in exact order. The filter is an ANY set, and production only requires raw contacts to be hashed locally and transmitted without raw contact leakage. After implementation, the test compares sorted actual and expected hashes while keeping the raw email/phone and hash output leak checks. Hosted-failing exact test, full CLI search IPC test file, fmt, diff check, public guard, and full local verification passed locally. | This is a test-only stability fix for S209's IPC assertion. It does not change production search behavior, prove hosted CI has passed until PR #9 reruns, read private resumes, evaluate real contact recall, clear field-quality blockers, or make stable release ready. |
| S211 | Product location extraction and filtering complete locally | Focused tests first failed because extractor-rules had no `FieldType::Location`, rank-fusion had no profile/filter API for locations, CLI search rejected `--location`, CLI IPC did not emit `locations_any`, and daemon IPC ignored location filters until after full-text top-k retrieval. After implementation, explicitly labeled location lines are extracted with span-backed evidence and canonical common city aliases, import persists `location` entity mentions, benchmark field labels include location, rank-fusion hydrates and matches location profiles, CLI supports `--location`/`--locations-any`, CLI/daemon IPC carry `locations_any`, and both CLI and daemon prefilter matching document IDs before full-text top-k truncation. Focused RED/GREEN, related extractor/rank/import/benchmark/CLI/daemon suites, fmt, diff check, public guard, focused clippy, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not parse arbitrary addresses or unlabeled city mentions, prove real business location recall/F1, evaluate private resume corpora, broaden multilingual geography coverage beyond common aliases, clear field-quality evidence blockers, or make stable release ready. |
| S212 | Product location field-quality release gate complete locally | Focused RED tests first failed because private-business field-quality reports missing `location` metrics were accepted by both library and CLI field gates. After implementation, `PRODUCTION_FIELD_QUALITY_THRESHOLDS` requires `location` metrics, complete strict private-business fixtures include the metric, reports missing it are rejected, and the release blockers runbook documents the updated field evidence boundary. Focused RED/GREEN, complete private-business acceptance regressions, full `benchmark-runner`, focused clippy, fmt, runbook guard, diff check, public guard, and full local verification passed locally. | This slice tightens release-evidence validation only. It does not create or upload private labels, run real business location-quality evaluation, prove production location recall/F1 on representative resumes, broaden geography/address parsing, clear field-quality blockers, or make stable release ready. |
| S213 | Product certificate field-quality release gate complete locally | Focused RED tests first failed because private-business field-quality reports missing `certificate` metrics were accepted by both library and CLI field gates. After implementation, `PRODUCTION_FIELD_QUALITY_THRESHOLDS` requires `certificate` metrics, complete strict private-business fixtures include the metric, reports missing it are rejected, and the release blockers runbook documents the updated field evidence boundary. Focused RED/GREEN, complete private-business acceptance regressions, full `benchmark-runner`, focused clippy, fmt, and runbook guard passed locally. | This slice tightens release-evidence validation only. It does not create or upload private labels, run real business certificate-quality evaluation, prove production certificate recall/F1 on representative resumes, broaden certificate dictionaries, clear field-quality blockers, or make stable release ready. |
| S214 | Product years-experience field-quality release gate complete locally | Focused RED tests first failed because private-business field-quality reports missing `years_experience` metrics were accepted by both library and CLI field gates. After implementation, `PRODUCTION_FIELD_QUALITY_THRESHOLDS` requires `years_experience` metrics, complete strict private-business fixtures include the metric, reports missing it are rejected, and the release blockers runbook documents the updated field evidence boundary. Focused RED/GREEN, complete private-business acceptance regressions, full `benchmark-runner`, focused clippy, fmt, and runbook guard passed locally. | This slice tightens release-evidence validation only. It does not create or upload private labels, run real business years-experience quality evaluation, prove production years-experience recall/F1 on representative resumes, improve date arithmetic coverage, clear field-quality blockers, or make stable release ready. |
| S215 | Product expanded field-alias extraction complete locally | Focused RED tests first failed because high-signal production aliases for Spark, Hadoop, Airflow, TensorFlow, PyTorch, scikit-learn, Vue.js, Angular, GraphQL, AWS Security Specialty, Google Professional Data Engineer, CCNA, platform engineer, security engineer, mobile engineer, and business analyst were not extracted and `Vue.js` was misclassified as JavaScript through the old `js` suffix alias. After implementation, extractor-rules maps these aliases with span-backed evidence, prevents known certificate aliases from being title mentions, and import persists the new skill/certificate/title entity mentions without CLI output, path, contact, or raw-value leaks. Focused RED/GREEN, full `extractor-rules`, full `resume-cli --test s16_persisted_fields`, full `resume-cli`, fmt, focused clippy, diff check, public guard, and full local verification passed locally. | This slice broadens high-signal rule dictionaries only. It does not prove real business field-quality metrics, create/upload private labels, complete broad multilingual dictionaries, clear private field-quality blockers, validate million-scale behavior, or make stable release ready. |
| S216 | Product major extraction and filtering complete locally | Focused RED tests first failed because `FieldType::Major` was missing and rank-fusion lacked `with_majors` / `with_majors_any`. After implementation, labeled `Major:`, `Field of Study:`, and `专业：` lines extract span-backed normalized `major` mentions, SQLite schema v19 accepts and indexes `major` entity mentions, import persists them without output/path/contact/raw-value leaks, CLI supports `--major`/`--majors-any`, CLI/daemon IPC carry `majors_any`, local plus daemon full-text search prefilter matching document IDs before top-k truncation, and benchmark field-quality scoring accepts `major` as an ordinary labeled field. Focused RED/GREEN, full extractor/rank/meta/import/benchmark tests, full `resume-cli`, full `resume-daemon`, fmt, focused clippy, and diff check passed locally. | This slice uses synthetic/temp fixtures only. It does not prove real business major-field F1, broaden all education-major dictionaries, add major to private field-quality release gates, evaluate private resume corpora, validate million-scale behavior, clear platform/signing/model/OCR blockers, or make the complete product ready. |
| S217 | Product major field-quality release gate complete locally | Focused RED tests first failed because private-business field-quality reports missing `major` metrics were accepted by both the library gate and CLI gate. After implementation, `PRODUCTION_FIELD_QUALITY_THRESHOLDS` requires `major` metrics for private business release evidence, complete strict private-business fixtures include the metric, reports missing it are rejected, and the release blockers runbook documents the updated field evidence boundary. Focused RED/GREEN, full benchmark-runner runner/CLI suites, focused clippy, fmt, runbook guard, public guard, and full local verification passed locally. | This slice tightens release-evidence validation only. It does not create or upload private labels, run real business major-quality evaluation, prove production major recall/F1 on representative resumes, broaden major dictionaries, clear field-quality blockers, or make stable release ready. |
| S218 | Product broader major alias extraction complete locally | Focused RED tests first failed because high-signal major aliases such as artificial intelligence, computer engineering, cybersecurity, network engineering, communication engineering, mechanical engineering, automation, accounting, marketing, and human resources were not extracted inside education context, and search filters did not normalize Chinese major inputs such as `人工智能` / `网络工程` / `会计学` to canonical values. After implementation, extractor-rules maps those aliases with span-backed evidence, rank-fusion normalizes matching filter/profile aliases, and import persists broader major mentions without CLI output, path, contact, or raw-value leaks. Focused RED/GREEN, full extractor/rank/import/CLI persisted-field suites, fmt, focused clippy, diff check, public guard, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not prove real business major-field F1, complete all education-major dictionaries, create/upload private labels, evaluate private resume corpora, validate million-scale behavior, clear platform/signing/model/OCR blockers, or make stable release ready. |
| S219 | Product broader location alias normalization complete locally | Focused RED tests first failed because high-signal labeled location values such as San Francisco Bay Area, New York City, Hong Kong, Singapore, and Chongqing were persisted as raw normalized strings or Chinese text, and search filters such as `SF Bay Area`, `纽约`, and `Hong Kong` did not match equivalent profile locations. After implementation, extractor-rules and rank-fusion normalize those aliases to stable canonical location keys while keeping extraction limited to explicit location labels, and import persists the canonical aliases with span evidence so `--location "SF Bay Area"` prefilters before full-text top-k truncation. Focused RED/GREEN, full extractor/rank/import/CLI persisted-field/search-filter suites, fmt, focused clippy, diff check, public guard, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not prove real business location-field F1, complete arbitrary address parsing, create/upload private labels, evaluate private resume corpora, validate million-scale behavior, clear platform/signing/model/OCR blockers, or make stable release ready. |
| S220 | Product labeled-address city extraction complete locally | Focused RED tests first failed because explicit address lines such as `Address: 123 Market St, San Francisco, CA`, `地址：北京市海淀区...`, and `Current Address: 88 Queen's Road, Hong Kong` produced no location evidence. After implementation, address labels are recognized separately from ordinary location labels, extractor-rules scans delimited address components and high-signal city substrings, persists only the city evidence span such as `San Francisco`, `北京市`, or `Hong Kong`, and import/search can filter those documents by canonical location without storing the full street-address span as location raw evidence. Focused RED/GREEN, full extractor/CLI persisted-field/search-filter/import suites, fmt, focused clippy, diff check, public guard, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not parse arbitrary unlabeled addresses, complete all global address formats, prove real business location-field F1, create/upload private labels, evaluate private resume corpora, validate million-scale behavior, clear platform/signing/model/OCR blockers, or make stable release ready. |
| S221 | Product address city substring normalization complete locally | Focused RED tests first failed because explicit address labels containing lowercase/no-delimiter English city substrings such as `123 market st san francisco ca` and `88 queen's road hong kong`, plus Chinese district-style values such as `北京海淀区...` and `深圳南山区...`, produced no location evidence. After implementation, address city substring matching is case-insensitive for English aliases, recognizes high-signal Chinese city aliases without requiring `市`, preserves the original matched city span, and import/search persists canonical city locations without storing full street-address spans as location raw evidence. Focused RED/GREEN, full extractor/CLI persisted-field/search-filter/import suites, fmt, focused clippy, diff check, public guard, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not parse arbitrary unlabeled addresses, complete all global address formats, prove real business location-field F1, create/upload private labels, evaluate private resume corpora, validate million-scale behavior, clear platform/signing/model/OCR blockers, or make stable release ready. |
| S222 | Product broader degree alias extraction complete locally | Focused RED tests first failed because high-signal engineering and technical degree aliases such as `MEng`, `M.Tech`, `MPhil`, `B.Tech`, and `B.E.` were not extracted as canonical degree mentions, and `--degree MEng` was not accepted as a master-level filter. After implementation, extractor-rules maps those aliases to `master` or `bachelor` with exact span evidence, rank-fusion parses the same filter aliases through compact punctuation-insensitive normalization while rejecting ambiguous bare `BE`, and import/search persists the broader degree mentions without CLI output, path, contact, or raw-value leaks. Focused RED/GREEN, full extractor/rank/import/CLI persisted-field/search-filter suites, fmt, focused clippy, diff check, public guard, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not prove real business degree-field F1, complete all global education credential aliases, parse ambiguous bare `BE`, create/upload private labels, evaluate private resume corpora, validate million-scale behavior, clear platform/signing/model/OCR blockers, or make stable release ready. |
| S223 | Product broader company suffix normalization complete locally | Focused RED tests first failed because company values such as `Synthetic AI Co., Ltd.` and `Example Systems Pte Ltd` kept legal suffix fragments, `Alpine Search GmbH` and `合成科技有限合伙` were not recognized as company evidence, `--company "Synthetic AI"` did not match the persisted target, and unrelated labeled lines containing the word company could be misclassified as company evidence. After implementation, extractor-rules recognizes and strips high-signal legal suffixes including `Co., Ltd.`, `Pte Ltd`, `GmbH`, `S.A.`, and `有限合伙`, avoids non-company labeled-line fallback extraction, rank-fusion normalizes the same suffixes for filters/profiles, and import/search persists canonical company mentions without CLI output, path, contact, or raw-value leaks. Focused RED/GREEN, full extractor/rank/import/CLI persisted-field/search-filter suites, fmt, focused clippy, diff check, public guard, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not prove real business company-field F1, complete all global legal-entity suffixes, infer employers from arbitrary prose, create/upload private labels, evaluate private resume corpora, validate million-scale behavior, clear platform/signing/model/OCR blockers, or make stable release ready. |
| S224 | Product broader school-tier alias normalization complete locally | Focused RED tests first failed because explicit school-tier aliases such as `C9 League`, `Project 211`, `Ivy League`, and `Russell Group` were not extracted or parsed as canonical school tiers, and `--school-tier "C9 League"` could not match the persisted target. After implementation, extractor-rules maps `C9 League` and Project 985/211 aliases to canonical 985/211 evidence, maps Double First-Class phrases plus `双一流建设高校` to `double_first_class`, maps Ivy League/Russell Group to `overseas`, preserves exact span evidence, and rank-fusion parses the same aliases for filters. Import/search persists the broader school-tier aliases without CLI output, path, contact, or raw-value leaks. Focused RED/GREEN, full extractor/rank/import/CLI persisted-field/search-filter suites, fmt, focused clippy, diff check, public guard, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not infer tiers from arbitrary school names, prove real business school-tier F1, complete all global ranking systems, create/upload private labels, evaluate private resume corpora, validate million-scale behavior, clear platform/signing/model/OCR blockers, or make stable release ready. |
| S225 | Product broader certificate alias normalization complete locally | Focused RED tests first failed because CKS, Terraform Associate, Google Associate Cloud Engineer, AZ-204, and RHCSA aliases were not extracted as the intended canonical certificate mentions, rank-fusion preserved raw normalized aliases such as `terraform_associate` and `az_204`, and `--certificate "Terraform Associate"` could not match the persisted target. After implementation, extractor-rules maps CKS/Certified Kubernetes Security Specialist, HashiCorp Certified Terraform Associate/Terraform Associate, Google/GCP Associate Cloud Engineer, AZ-204/Azure Developer, and RHCSA/Red Hat Certified System Administrator to canonical certificate values with exact span evidence, and rank-fusion normalizes the same aliases for filters/profiles. Import/search persists the broader certificate aliases without CLI output, path, contact, or raw-value leaks. Focused RED/GREEN, full extractor/rank/import/CLI persisted-field/search-filter suites, fmt, focused clippy, diff check, public guard, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not prove real business certificate-field F1, complete all global certification dictionaries, infer certification levels or dates, create/upload private labels, evaluate private resume corpora, validate million-scale behavior, clear platform/signing/model/OCR blockers, or make stable release ready. |
| S226 | Product broader skill alias extraction and filtering complete locally | Focused RED tests first failed because high-signal cloud/data/DevOps skill aliases such as Amazon Web Services, Microsoft Azure, Google Cloud Platform, Terraform, Ansible, Jenkins, GitLab CI, Kafka, Flink, Elastic Search, Mongo DB, and Snowflake were not extracted as skill mentions, rank-fusion did not normalize user skill aliases such as K8s, Golang, Postgres, NodeJS, React.js, TS, sklearn, Amazon Web Services, Google Cloud Platform, Elastic Search, Mongo DB, and GitLab CI/CD to persisted canonical skill keys, and `--skills-any "Amazon Web Services"` could not match the persisted target. After implementation, extractor-rules maps those high-signal skill aliases to canonical skill values with exact span evidence, rank-fusion normalizes common filter/profile aliases to the same keys, and import/search persists broader skill aliases without CLI output, path, contact, or raw-value leaks. Focused RED/GREEN, full extractor/rank/import/CLI persisted-field/search-filter suites, fmt, focused clippy, diff check, public guard, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not prove real business skill-field F1, complete all global skill dictionaries, infer skills from arbitrary prose, create/upload private labels, evaluate private resume corpora, validate million-scale behavior, clear platform/signing/model/OCR blockers, or make stable release ready. |
| S227 | Product candidate-name field filtering complete locally | Focused RED tests first failed because rank-fusion had no `with_names`/`with_names_any` API, direct CLI search rejected `--name`, CLI IPC did not transmit `names_any`, and daemon IPC ignored name filters until after full-text top-k retrieval. After implementation, rank-fusion normalizes persisted and requested names, CLI supports `--name`/`--names-any`, CLI IPC emits canonical `names_any`, daemon IPC parses `names_any`, and both CLI plus daemon prefilter `EntityType::Name` document IDs before the full-text top-k cutoff while hydrated profiles still verify the name match. Focused RED/GREEN, full rank-fusion, full CLI search-filter and search-IPC suites, full daemon search-IPC suite, full `resume-cli`, full `resume-daemon`, fmt, focused clippy, diff check, public guard, and full local verification passed locally. | This slice uses synthetic/temp fixtures only. It does not prove real business name-field precision/recall, add fuzzy/person-alias matching, change search snippet redaction, create/upload private labels, evaluate private resume corpora, validate million-scale behavior, clear platform/signing/model/OCR blockers, or make stable release ready. |
| S228 | Product name field-quality release gate complete locally | Focused RED tests first failed because private-business field-quality reports missing `name` metrics were accepted by both the library gate and CLI gate. After implementation, `PRODUCTION_FIELD_QUALITY_THRESHOLDS` requires `name` metrics for private business release evidence, complete strict private-business fixtures include the metric, reports missing it are rejected, and the release blockers runbook documents the updated field evidence boundary. Focused RED/GREEN, full benchmark-runner runner/CLI suites, focused clippy, fmt, runbook guard, public guard, and full local verification passed locally. | This slice tightens release-evidence validation only. It does not create or upload private labels, run real business name-quality evaluation, prove production name precision/recall/F1 on representative resumes, add fuzzy/person-alias matching, clear field-quality blockers, or make stable release ready. |

## Command Log

### S228

Design target:

- Require `name` metrics in strict private-business field-quality release
  evidence now that candidate-name filtering is production-visible.
- Keep the report boundary local/redacted aggregate only, with no raw field
  values, sample IDs, local paths, or resume text.
- Use synthetic fixtures only; this slice validates the release gate shape and
  does not claim real business name-quality evidence exists.

Observed RED:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_private_business_report_without_name_metric --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_requires_private_business_name_metric --locked -- --exact
```

Output summary:

- The library exact test failed because `evaluate_field_quality_gate_json`
  accepted a strict private-business report that lacked `name` metrics.
- The CLI exact test failed because `resume-benchmark field-gate
  --require-private-business-labeled` exited successfully for the same missing
  `name` evidence.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_private_business_report_without_name_metric --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_requires_private_business_name_metric --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p benchmark-runner --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings
```

Output summary:

- The exact library and CLI tests passed after adding `name` to
  `PRODUCTION_FIELD_QUALITY_THRESHOLDS` and the complete strict
  private-business field-quality fixtures.
- `cargo test -p benchmark-runner --locked`: exit 0; 21 CLI benchmark tests, 43
  runner tests, and doc-tests passed.
- `cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings`:
  exit 0.

Final local gate:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --all --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `cargo fmt --all --check`: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0, `public repo guard passed`.
- `verify-local.sh`: exit 0; workspace clippy/tests/doc-tests, license/runbook/
  workflow/release-readiness checks, release artifact and SBOM checks, macOS
  package DMG verification, and public repository guard passed. Windows package
  check was skipped on this non-Windows host.

Scope note:

- S228 uses synthetic fixtures only. It does not read, print, commit, or upload
  real resumes, local data directories, tokens, diagnostics, model caches, or
  raw personal data.
- This slice requires private business release evidence to include `name`
  metrics. It does not create private labels, run private evaluation, prove real
  business name precision/recall/F1, clear field-quality blockers, validate
  million-scale behavior, clear platform/signing/model/OCR blockers, or make
  the complete product ready.

### S227

Design target:

- Add explicit candidate-name field filtering over already-persisted `name`
  entity mentions.
- Keep the filter exact after whitespace/lowercase normalization; do not add
  fuzzy matching or inferred person aliases in this slice.
- Apply the same filter through direct CLI search, CLI search IPC, and daemon
  search IPC before full-text top-k retrieval.
- Use synthetic/temp fixtures only.

Observed RED:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion --test s10_rank_fusion field_filters_match_candidate_name_any --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_name_before_fulltext_top_k_cutoff --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_name_before_fulltext_top_k_cutoff --locked -- --exact
```

Output summary:

- The rank-fusion exact test failed to compile because `SearchFilters` had no
  `with_names_any` method and `ResumeProfile` had no `with_names` method.
- The direct CLI exact test failed with search usage output because
  `resume-cli search` did not accept `--name`.
- The daemon IPC exact test returned a high-scoring decoy document because
  `names_any` was ignored by daemon request parsing/prefiltering.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion --test s10_rank_fusion field_filters_match_candidate_name_any --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_name_before_fulltext_top_k_cutoff --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_name_before_fulltext_top_k_cutoff --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --all
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s10_search_filters --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s48_search_ipc --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-daemon --test s48_search_ipc --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p rank-fusion -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-daemon --locked
```

Output summary:

- The exact rank-fusion test passed after adding `ResumeProfile::with_names`,
  `SearchFilters::with_names_any`, `names_any()`, and exact normalized name
  matching.
- The exact direct CLI test passed: `resume-cli search --name "Synthetic
  Target"` now returns the low-frequency target before full-text top-k cutoff
  and does not return the noisy decoys.
- The exact daemon IPC test passed: `names_any` is parsed, normalized, applied
  as a metadata document-ID prefilter, and the response excludes the decoys.
- The exact CLI search IPC test passed with request-body evidence that
  `--name "Synthetic Candidate"` is serialized as canonical `names_any:
  ["synthetic candidate"]`.
- `cargo test -p rank-fusion --locked`: exit 0; 21 S10 tests, 2 S11 hybrid RRF
  tests, and doc-tests passed.
- `cargo test -p resume-cli --test s10_search_filters --locked`: exit 0; 10
  search-filter tests passed.
- `cargo test -p resume-cli --test s48_search_ipc --locked`: exit 0; 8 CLI
  search IPC tests passed.
- `cargo test -p resume-daemon --test s48_search_ipc --locked`: exit 0; 14
  daemon search IPC tests passed.
- `cargo clippy -p rank-fusion -p resume-cli -p resume-daemon --all-targets
  --locked -- -D warnings`: exit 0.
- `cargo test -p resume-cli --locked`: exit 0; all CLI tests passed, including
  S10 search filters, S16 persisted fields, OCR handoff, semantic/hybrid search,
  diagnostics, IPC, delete/purge, candidate review, release-readiness, and
  fault-simulation suites.
- `cargo test -p resume-daemon --locked`: exit 0; all daemon tests passed,
  including status/import/search/detail IPC, import scheduler, OCR worker,
  embedding worker, and daemon-kill restart coverage.

Final local gate:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `cargo fmt --all --check`: exit 0.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0, `public repo guard passed`.
- `verify-local.sh`: exit 0; workspace clippy/tests/doc-tests, license/runbook/
  workflow/release-readiness checks, release artifact and SBOM checks, macOS
  package DMG verification, and public repository guard passed. Windows package
  check was skipped on this non-Windows host.

Scope note:

- S227 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload real resumes, local data directories, tokens, diagnostics, model
  caches, or raw personal data.
- This slice adds exact normalized candidate-name filtering over persisted
  field evidence. It does not prove real business name-field precision/recall,
  implement fuzzy/person-alias matching, change search snippet redaction, clear
  private field-quality blockers, validate million-scale behavior, clear
  platform/signing/model/OCR blockers, or make the complete product ready.

### S226

Design target:

- Broaden explicit high-signal cloud, data, and DevOps skill aliases across
  extraction, persisted profiles, and search filters.
- Preserve exact span evidence and redacted Debug/output behavior.
- Normalize common user search aliases to the same canonical skill keys used by
  persisted profiles.
- Use synthetic fixtures only.

Observed RED:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p extractor-rules --test s10_fields extracts_broader_cloud_data_and_devops_skill_aliases --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion --test s10_rank_fusion field_filters_normalize_broader_skill_aliases --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_skill_aliases_and_filters_without_output_leaks --locked -- --exact
```

Output summary:

- The extractor exact failed before implementation with no skill evidence for
  Amazon Web Services, Microsoft Azure, Google Cloud Platform, Terraform,
  Ansible, Jenkins, GitLab CI, Kafka, Flink, Elastic Search, Mongo DB, or
  Snowflake.
- The rank-fusion exact failed before implementation because filters containing
  aliases such as K8s, Golang, Postgres, NodeJS, React.js, TS, sklearn, Amazon
  Web Services, Google Cloud Platform, Elastic Search, Mongo DB, and GitLab
  CI/CD did not match profiles using the persisted canonical skill values.
- The CLI persisted-field exact failed before implementation because no broader
  skill aliases were persisted for the target, so the intended
  `--skills-any "Amazon Web Services"` filter path could not match it.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p extractor-rules --test s10_fields extracts_broader_cloud_data_and_devops_skill_aliases --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion --test s10_rank_fusion field_filters_normalize_broader_skill_aliases --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_skill_aliases_and_filters_without_output_leaks --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p extractor-rules --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p import-pipeline --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s16_persisted_fields --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s10_search_filters --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --all
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p extractor-rules -p rank-fusion -p import-pipeline -p resume-cli --all-targets --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN tests passed after adding the broader explicit skill aliases to
  extractor-rules and rank-fusion skill normalization.
- Full `extractor-rules` passed: 27 S10 tests plus 5 S7 tests and doc-tests.
- Full `rank-fusion` passed: 20 S10 tests plus 2 S11 tests and doc-tests.
- Full `import-pipeline` passed: 7 tests plus doc-tests.
- Full `resume-cli --test s16_persisted_fields` passed: 23 tests.
- Full `resume-cli --test s10_search_filters` passed: 9 tests.
- `cargo fmt --all`, `cargo fmt --all --check`, and focused clippy passed.
- `git diff --check`, the public repo guard, and full local verification
  passed. Full local verification included workspace clippy/tests/doc-tests,
  license, runbook, workflow, release readiness, release artifact, SBOM, macOS
  package, and public-repo guard checks. Windows package check was skipped on
  the non-Windows host.

Scope note:

- S226 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, skill values, or
  private corpus evidence.

### S225

Design target:

- Broaden explicit high-signal certificate aliases across extraction, persisted
  profiles, and search filters.
- Preserve exact span evidence and redacted Debug/output behavior.
- Keep extraction dictionary-based and high-signal; do not infer certificate
  dates, levels, or broad families beyond explicit aliases.
- Use synthetic fixtures only.

Observed RED:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p extractor-rules --test s10_fields extracts_broader_certificate_aliases_with_exact_spans --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion --test s10_rank_fusion field_filters_normalize_broader_certificate_aliases --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_certificate_aliases_and_filters_without_output_leaks --locked -- --exact
```

Output summary:

- The extractor exact failed before implementation with no broader certificate
  evidence for CKS, Terraform Associate, Google Associate Cloud Engineer,
  AZ-204, or RHCSA.
- The rank-fusion exact failed before implementation because aliases normalized
  to raw values such as `certified_kubernetes_security_specialist`,
  `terraform_associate`, `google_associate_cloud_engineer`, and `az_204`
  instead of the intended canonical certificate keys.
- The CLI persisted-field exact failed before implementation because no broader
  certificate aliases were persisted for the target, so the intended
  `--certificate "Terraform Associate"` filter path could not match it.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p extractor-rules --test s10_fields extracts_broader_certificate_aliases_with_exact_spans --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion --test s10_rank_fusion field_filters_normalize_broader_certificate_aliases --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_certificate_aliases_and_filters_without_output_leaks --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p extractor-rules --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p import-pipeline --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s16_persisted_fields --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s10_search_filters --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --all
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p extractor-rules -p rank-fusion -p import-pipeline -p resume-cli --all-targets --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN tests passed after adding the broader explicit certificate
  aliases to extractor-rules and rank-fusion normalization.
- Full `extractor-rules` passed: 26 S10 tests plus 5 S7 tests and doc-tests.
- Full `rank-fusion` passed: 19 S10 tests plus 2 S11 tests and doc-tests.
- Full `import-pipeline` passed: 7 tests plus doc-tests.
- Full `resume-cli --test s16_persisted_fields` passed: 22 tests.
- Full `resume-cli --test s10_search_filters` passed: 9 tests.
- `cargo fmt --all`, `cargo fmt --all --check`, and focused clippy passed.
- `git diff --check`, the public repo guard, and full local verification
  passed. Full local verification included workspace clippy/tests/doc-tests,
  license, runbook, workflow, release readiness, release artifact, SBOM, macOS
  package, and public-repo guard checks. Windows package check was skipped on
  the non-Windows host.

Scope note:

- S225 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, certificate values, or
  private corpus evidence.

### S224

Design target:

- Broaden explicit school-tier aliases across extraction, persisted profiles,
  and search filters.
- Keep aliases high-signal and bounded to education/school context; do not infer
  tiers from arbitrary school names.
- Preserve span evidence and redacted Debug/output behavior.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_broader_school_tier_aliases_with_exact_spans --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion field_filters_parse_broader_school_tier_aliases --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_school_tier_aliases_and_filters_without_output_leaks --locked -- --exact
```

Output summary:

- The extractor exact failed before implementation with only `211` and
  `double_first_class` evidence, missing `C9 League` and `Ivy League`.
- The rank-fusion exact failed before implementation because `C9 League` parsed
  as no school tier.
- The CLI persisted-field exact failed before implementation because only
  `double_first_class` was persisted for the target and the broader filter could
  not complete the intended search path.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_broader_school_tier_aliases_with_exact_spans --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion field_filters_parse_broader_school_tier_aliases --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_school_tier_aliases_and_filters_without_output_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --locked
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters --locked
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p rank-fusion -p import-pipeline -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN tests passed after adding the broader explicit school-tier
  aliases to extractor-rules and rank-fusion parsing.
- Full `extractor-rules` passed: 25 S10 tests plus 5 S7 tests and doc-tests.
- Full `rank-fusion` passed: 18 S10 tests plus 2 S11 tests and doc-tests.
- Full `resume-cli --test s16_persisted_fields` passed: 21 tests.
- Full `resume-cli --test s10_search_filters` passed: 9 tests.
- Full `import-pipeline` passed: 7 tests plus doc-tests.
- `cargo fmt --all`, `cargo fmt --all --check`, and focused clippy passed.
- `git diff --check`, the public repo guard, and full local verification
  passed. Full local verification included workspace clippy/tests/doc-tests,
  license, runbook, workflow, release readiness, release artifact, SBOM, macOS
  package, and public-repo guard checks. Windows package check was skipped on
  the non-Windows host.

Scope note:

- S224 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, school names,
  school-tier values, or private corpus evidence.

### S223

Design target:

- Broaden company legal suffix recognition and normalization across extraction,
  persisted profiles, and search filters.
- Keep company evidence span-backed and prevent unrelated labeled lines such as
  contact lines from becoming company evidence.
- Preserve redacted Debug and CLI output behavior.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_broader_company_suffixes_with_exact_spans --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion field_filters_normalize_broader_company_suffixes --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_company_suffixes_and_filters_without_output_leaks --locked -- --exact
```

Output summary:

- The extractor exact failed before implementation with normalized company
  values `["synthetic ai co.,", "example systems pte"]` instead of the expected
  canonical company values, and missed `GmbH` plus `有限合伙` company evidence.
- The rank-fusion exact failed before implementation because the broader
  company suffix filters did not match canonical profile companies.
- The CLI persisted-field exact failed before implementation because a
  non-company labeled contact line containing `company` was misclassified as
  company evidence, proving the fallback extraction was too broad.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_broader_company_suffixes_with_exact_spans --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion field_filters_normalize_broader_company_suffixes --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_company_suffixes_and_filters_without_output_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --locked
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters --locked
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p rank-fusion -p import-pipeline -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN tests passed after broadening company suffix normalization and
  disabling non-company labeled-line fallback extraction.
- Full `extractor-rules` passed: 24 S10 tests plus 5 S7 tests and doc-tests.
- Full `rank-fusion` passed: 17 S10 tests plus 2 S11 tests and doc-tests.
- Full `resume-cli --test s16_persisted_fields` passed: 20 tests.
- Full `resume-cli --test s10_search_filters` passed: 9 tests.
- Full `import-pipeline` passed: 7 tests plus doc-tests.
- `cargo fmt --all`, `cargo fmt --all --check`, and focused clippy passed.
- `git diff --check`, the public repo guard, and full local verification
  passed. Full local verification included workspace clippy/tests/doc-tests,
  license, runbook, workflow, release readiness, release artifact, SBOM, macOS
  package, and public-repo guard checks. Windows package check was skipped on
  the non-Windows host.

Scope note:

- S223 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, company values, or
  private corpus evidence.

### S222

Design target:

- Broaden canonical `degree` extraction and search-filter parsing for
  high-signal engineering/technical degree aliases.
- Keep unlabeled degree extraction bounded to education context, and avoid
  accepting ambiguous bare `BE`.
- Preserve span evidence and redacted Debug/output behavior.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_broader_degree_aliases_with_exact_spans --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion degree_level_parse_accepts_broader_engineering_degree_aliases --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_degree_aliases_and_filters_without_output_leaks --locked -- --exact
```

Output summary:

- The extractor exact failed before implementation with no canonical degree
  matches for `MEng`, `B.Tech`, `M.Tech`, `MPhil`, and `B.E.`.
- The rank-fusion exact failed before implementation because `MEng` parsed as
  no degree level.
- The CLI persisted-field exact failed before implementation because the
  target document had no canonical `degree` mentions from the broader aliases.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_broader_degree_aliases_with_exact_spans --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion degree_level_parse_accepts_broader_engineering_degree_aliases --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_degree_aliases_and_filters_without_output_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --locked
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters --locked
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p rank-fusion -p import-pipeline -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN tests passed after adding broader degree aliases to
  extractor-rules and rank-fusion parsing, with ambiguous bare `BE` rejected.
- Full `extractor-rules` passed: 23 S10 tests plus 5 S7 tests and doc-tests.
- Full `rank-fusion` passed: 16 S10 tests plus 2 S11 tests and doc-tests.
- Full `resume-cli --test s16_persisted_fields` passed: 19 tests.
- Full `resume-cli --test s10_search_filters` passed: 9 tests.
- Full `import-pipeline` passed: 7 tests plus doc-tests.
- `cargo fmt --all`, `cargo fmt --all --check`, and focused clippy passed.
- `git diff --check`, the public repo guard, and full local verification
  passed. Full local verification included workspace clippy/tests/doc-tests,
  license, runbook, workflow, release readiness, release artifact, SBOM, macOS
  package, and public-repo guard checks. Windows package check was skipped on
  the non-Windows host.

Scope note:

- S222 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, degree values, or
  private corpus evidence.

### S221

Design target:

- Extend explicit address-label city extraction to case-insensitive English
  city substrings and Chinese district-style city substrings without requiring
  `市`.
- Preserve original city-only evidence spans and avoid persisting full
  street-address spans as location raw evidence.
- Preserve the conservative boundary: no location inference from unlabeled
  experience text.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_case_insensitive_and_district_city_evidence_from_address_values --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_case_insensitive_and_district_address_city_locations --locked -- --exact
```

Output summary:

- The extractor exact failed before implementation with zero `location`
  matches from lowercase/no-delimiter English address city substrings and
  Chinese district-style address values.
- The CLI persisted-field exact failed before implementation because the target
  document had no canonical city `location` mentions from those address lines.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_case_insensitive_and_district_city_evidence_from_address_values --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_case_insensitive_and_district_address_city_locations --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters --locked
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p import-pipeline -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN tests passed after making address city substring matching
  case-insensitive for English and adding district-style Chinese city aliases.
- Full `extractor-rules` passed: 22 S10 tests plus 5 S7 tests and doc-tests.
- Full `resume-cli --test s16_persisted_fields` passed: 18 tests.
- Full `resume-cli --test s10_search_filters` passed: 9 tests.
- Full `import-pipeline` passed: 7 tests plus doc-tests.
- `cargo fmt --all`, `cargo fmt --all --check`, and focused clippy passed.
- `git diff --check`, the public repo guard, and full local verification
  passed. Full local verification included workspace clippy/tests/doc-tests,
  license, runbook, workflow, release readiness, release artifact, SBOM, macOS
  package, and public-repo guard checks. Windows package check was skipped on
  the non-Windows host.

Scope note:

- S221 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, street
  addresses, raw contact values, contact hashes, private labels, field values,
  location values, or private corpus evidence.

### S220

Design target:

- Extract city-level `location` evidence from explicit address labels without
  persisting full street-address spans as location raw evidence.
- Preserve the existing conservative boundary: do not infer locations from
  unlabeled experience text.
- Keep import/search outputs free of local paths, contacts, and raw filter
  strings; persisted field Debug output remains redacted.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_city_evidence_from_labeled_address_values_without_street_spans --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_labeled_address_city_locations_without_street_evidence_leaks --locked -- --exact
```

Output summary:

- The extractor exact failed before implementation with zero `location`
  matches from labeled address values.
- The CLI persisted-field exact failed before implementation because the
  target document had no canonical city `location` mentions from its address
  lines.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_city_evidence_from_labeled_address_values_without_street_spans --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_labeled_address_city_locations_without_street_evidence_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters --locked
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p import-pipeline -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN tests passed after adding address-label city extraction.
- Full `extractor-rules` passed: 21 S10 tests plus 5 S7 tests and doc-tests.
- Full `resume-cli --test s16_persisted_fields` passed: 17 tests.
- Full `resume-cli --test s10_search_filters` passed: 9 tests.
- Full `import-pipeline` passed: 7 tests plus doc-tests.
- `cargo fmt --all`, `cargo fmt --all --check`, and focused clippy passed.
- `git diff --check`, the public repo guard, and full local verification
  passed. Full local verification included workspace clippy/tests/doc-tests,
  license, runbook, workflow, release readiness, release artifact, SBOM, macOS
  package, and public-repo guard checks. Windows package check was skipped on
  the non-Windows host.

Scope note:

- S220 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, street
  addresses, raw contact values, contact hashes, private labels, field values,
  location values, or private corpus evidence.

### S219

Design target:

- Broaden explicit labeled `location` normalization and search-filter
  normalization for high-signal English and Chinese city aliases.
- Keep extraction limited to explicit location labels so experience text such
  as customer geography does not become location evidence.
- Preserve privacy: import/status/search output should not echo local paths,
  contacts, or raw filter strings; persisted field Debug output remains
  redacted.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_broader_labeled_location_aliases_with_exact_spans --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion field_filters_normalize_broader_location_aliases --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_location_aliases_and_filters_without_output_leaks --locked -- --exact
```

Output summary:

- The extractor exact failed before implementation because broader labeled
  location aliases persisted as raw normalized strings such as
  `san francisco bay area`, `new york city`, `香港`, and `重庆市`.
- The rank-fusion exact failed before implementation because `SF Bay Area`
  did not match a San Francisco Bay Area profile location.
- The CLI persisted-field exact failed before implementation because the
  broader location aliases were not persisted as canonical `location` entity
  mentions.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_broader_labeled_location_aliases_with_exact_spans --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion field_filters_normalize_broader_location_aliases --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_location_aliases_and_filters_without_output_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --locked
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters --locked
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p rank-fusion -p import-pipeline -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN tests passed after adding broader location aliases to
  extractor-rules and rank-fusion normalization.
- Full `extractor-rules` passed: 20 S10 tests plus 5 S7 tests and doc-tests.
- Full `rank-fusion` passed: 15 S10 tests plus 2 S11 tests and doc-tests.
- Full `resume-cli --test s16_persisted_fields` passed: 16 tests.
- Full `resume-cli --test s10_search_filters` passed: 9 tests.
- Full `import-pipeline` passed: 7 tests plus doc-tests.
- `cargo fmt --all`, `cargo fmt --all --check`, and focused clippy passed.
- `git diff --check`, the public repo guard, and full local verification
  passed. Full local verification included workspace clippy/tests/doc-tests,
  license, runbook, workflow, release readiness, release artifact, SBOM, macOS
  package, and public-repo guard checks. Windows package check was skipped on
  the non-Windows host.

Scope note:

- S219 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, location values, or
  private corpus evidence.

### S218

Design target:

- Broaden `major` extraction and search-filter normalization for high-signal
  English and Chinese major aliases while preserving evidence spans.
- Keep extraction bounded to education context for unlabeled aliases so skill or
  experience text does not become major evidence.
- Preserve privacy: no CLI output should include local paths, contacts, or raw
  major values.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_broader_major_aliases_inside_education_context --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion field_filters_normalize_broader_major_aliases --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_major_aliases_without_output_leaks --locked -- --exact
```

Output summary:

- The extractor exact failed before implementation with no broader major
  matches for the new education-context aliases.
- The rank-fusion exact failed before implementation because `人工智能` did not
  normalize to `artificial_intelligence`.
- The CLI persisted-field exact failed before implementation because the
  broader major aliases were not persisted as `major` entity mentions.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_broader_major_aliases_inside_education_context --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion field_filters_normalize_broader_major_aliases --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_major_aliases_without_output_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --locked
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields --locked
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p rank-fusion -p import-pipeline -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN tests passed after adding broader major aliases to
  extractor-rules and rank-fusion normalization.
- Full `extractor-rules` passed: 19 S10 tests plus 5 S7 tests and doc-tests.
- Full `rank-fusion` passed: 14 S10 tests plus 2 S11 tests and doc-tests.
- Full `resume-cli --test s16_persisted_fields` passed: 15 tests.
- Full `import-pipeline` passed: 7 tests plus doc-tests.
- `cargo fmt --all` and focused clippy passed.
- `cargo fmt --all --check`, `git diff --check`, and the public repo guard
  passed.
- Full local verification passed, including workspace clippy/tests/doc-tests,
  license, runbook, workflow, release readiness, release artifact, SBOM, macOS
  package, and public-repo guard checks. Windows package check was skipped on
  the non-Windows host.

Scope note:

- S218 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, major values, or
  private corpus evidence.

### S217

Design target:

- Close the S216 release-gate gap by making `major` a required production
  field metric in private business labeled field-quality evidence.
- Preserve the redacted local aggregate boundary: reports still must not
  include raw text, paths, field values, sample IDs, filenames, or notes.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_private_business_report_without_major_metric --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_requires_private_business_major_metric --locked -- --exact
```

Output summary:

- The library exact test failed before implementation because a
  `private-business-labeled` field-quality report without a `major` metric was
  accepted.
- The CLI exact test failed before implementation because
  `resume-benchmark field-gate --require-private-business-labeled` accepted a
  report without a `major` metric.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_private_business_report_without_major_metric --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_requires_private_business_major_metric --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner --locked
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN tests passed after adding `major` to
  `PRODUCTION_FIELD_QUALITY_THRESHOLDS` and strict private-business report
  fixtures.
- Full `s17_benchmark_runner` passed: 42 tests.
- Full `s17_benchmark_cli` passed: 20 tests.
- Full `benchmark-runner` passed: CLI 20 tests, runner 42 tests, and doc-tests.
- Focused benchmark-runner clippy, `cargo fmt --all --check`, `git diff
  --check`, runbook guard, and public repository guard passed.
- Full local verification passed locally, including workspace clippy and tests,
  doc-tests, license/runbook/workflow/release-readiness checks, release
  artifact and SBOM checks, macOS package check, and final public repository
  guard. Windows package check was skipped on this non-Windows host.

Scope note:

- S217 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, major values, or
  private corpus evidence.

### S216

Design target:

- Close part of the documented education-field gap by adding `major` as a
  first-class structured field with evidence spans, persistence, and search
  filtering.
- Preserve hot-path constraints by using persisted `entity_mention` metadata
  for CLI and daemon prefiltering before full-text top-k truncation, without
  query-time extraction.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_labeled_major_values_with_alias_normalization --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion field_filters_match_any_major --locked -- --exact
```

Output summary:

- The extractor exact test failed before implementation because
  `FieldType::Major` did not exist.
- The rank-fusion exact test failed before implementation because
  `SearchFilters::with_majors_any`, `ResumeProfile::with_majors`, and
  `SearchFilters::majors_any` did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --test s10_fields extracts_labeled_major_values_with_alias_normalization --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion field_filters_match_any_major --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite entity_mentions_accept_major_values_for_searchable_prefilter --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_labeled_major_mentions_and_filters_search_without_output_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_major_before_fulltext_top_k_cutoff --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_report_scores_labeled_samples_without_raw_value_leakage --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules --locked
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p core-domain -p extractor-rules -p rank-fusion -p meta-store -p import-pipeline -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner -p extractor-rules --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN tests passed after adding `major` to extractor rules,
  domain/import mapping, SQLite schema v19 storage, rank-fusion profile/filter
  matching, CLI search parsing/IPC payloads, and daemon IPC parsing/prefiltering.
- Full `extractor-rules` passed: 18 S10 tests plus 5 S7 tests and doc-tests.
- Full `rank-fusion` passed: 13 S10 tests plus 2 S11 tests and doc-tests.
- Full `meta-store` passed: 55 SQLite tests plus identity/doc-tests,
  including direct `major` storage and prefilter coverage.
- Full `import-pipeline` passed.
- Full `benchmark-runner` passed after adding `major` to ordinary
  field-quality label scoring. The private-business field-quality release gate
  thresholds were not expanded in this slice.
- Full `resume-cli --test s16_persisted_fields`, `resume-cli --test
  s48_search_ipc`, `resume-cli --test s10_search_filters`, and
  `resume-daemon --test s48_search_ipc` passed.
- Full `resume-cli` and full `resume-daemon` passed locally.
- `cargo fmt --all --check`, focused clippy, and `git diff --check` passed.
- The first full local verification run caught a missing
  `benchmark-runner` `FieldType::Major` label arm; after adding benchmark
  field-quality scoring coverage for `major`, the focused benchmark tests and
  clippy passed.
- Final `guard-public-repo.sh` passed.
- Final full local verification passed locally, including workspace clippy and
  tests, doc-tests, license/runbook/workflow/release-readiness checks, release
  artifact and SBOM checks, macOS package check, and the final public
  repository guard. Windows package check was skipped on this non-Windows host.

Scope note:

- S216 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, major values, or
  private corpus evidence.

### S215

Design target:

- Close part of the P2 field-extraction dictionary gap from the system design:
  high-confidence skill, certificate, and title aliases should normalize to
  stable structured values with evidence spans and confidence.
- Preserve precision and privacy: avoid extracting `.js` suffixes in framework
  names as JavaScript, avoid classifying known certificate aliases as titles,
  and keep import output free of local paths, contacts, and raw field values.
- Use synthetic fixtures only.

Observed RED:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p extractor-rules --test s10_fields extracts_expanded_production_skill_certificate_and_title_aliases --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s16_persisted_fields import_persists_expanded_production_alias_mentions_without_output_leaks --locked -- --exact
```

Output summary:

- The extractor exact test failed before implementation because the only skill
  extracted from the expanded fixture was `JavaScript`, caused by `Vue.js`
  matching the old `js` suffix alias.
- The import exact test failed for the same reason: persisted skill mentions
  contained only `JavaScript` instead of the expanded skill set, and the new
  certificate/title aliases were absent.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p extractor-rules --test s10_fields extracts_expanded_production_skill_certificate_and_title_aliases --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s16_persisted_fields import_persists_expanded_production_alias_mentions_without_output_leaks --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p extractor-rules --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s16_persisted_fields --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --all --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p extractor-rules -p resume-cli --all-targets --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --locked
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused RED tests passed after expanding skill/certificate/title alias
  dictionaries, ordering framework aliases before the generic `js` alias, and
  suppressing title extraction for known certificate aliases.
- Full `extractor-rules` passed: 22 tests/doc-tests total, including the new
  expanded alias coverage.
- Full `resume-cli --test s16_persisted_fields` passed: 13 tests, including
  import persistence for the new aliases without output/path/contact leaks.
- Full `resume-cli` passed locally.
- `cargo fmt --all --check` and focused clippy passed locally.
- `git diff --check` and `guard-public-repo.sh` passed locally.
- Full local verification passed locally, including workspace tests and
  doc-tests, license/runbook/workflow/release-readiness checks, release
  artifact and SBOM checks, macOS package check, and the final public
  repository guard. Windows package check was skipped on this non-Windows host.

Scope note:

- S215 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, certificate values,
  skill values, title values, or private corpus evidence.

### S214

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_private_business_report_without_years_experience_metric -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_requires_private_business_years_experience_metric -- --exact
```

Output summary:

- The library exact test failed before implementation because removing
  `years_experience` metrics from a strict private-business field-quality
  report still returned `Ok(FieldQualityGateEvaluation { ... })`.
- The CLI exact test failed before implementation because
  `resume-benchmark field-gate --require-private-business-labeled` exited
  successfully for a private-business report missing `years_experience`
  metrics.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_private_business_report_without_years_experience_metric -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_requires_private_business_years_experience_metric -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_accepts_private_business_labeled_release_evidence -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_accepts_private_business_labeled_report -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
./scripts/ci/check-runbooks.sh
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Missing-`years_experience` private-business field-quality reports are now
  rejected by both the library gate and CLI gate without printing the report
  path.
- Complete strict private-business field-quality reports still pass for both
  the library and CLI acceptance regressions.
- Full `benchmark-runner`, focused clippy, `cargo fmt --all --check`, and
  `check-runbooks.sh` passed locally.
- `git diff --check` and `guard-public-repo.sh` passed locally.
- Full local verification passed locally, including workspace tests and
  doc-tests, license/runbook/workflow/release-readiness checks, release
  artifact and SBOM checks, macOS package check, and the final public
  repository guard. Windows package check was skipped on this non-Windows host.

Scope note:

- S214 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, certificate values,
  years-experience values, or location values from private resumes.

### S213

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_private_business_report_without_certificate_metric -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_requires_private_business_certificate_metric -- --exact
```

Output summary:

- The library exact test failed before implementation because removing
  `certificate` metrics from a strict private-business field-quality report
  still returned `Ok(FieldQualityGateEvaluation { ... })`.
- The CLI exact test failed before implementation because
  `resume-benchmark field-gate --require-private-business-labeled` exited
  successfully for a private-business report missing `certificate` metrics.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_private_business_report_without_certificate_metric -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_requires_private_business_certificate_metric -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_accepts_private_business_labeled_release_evidence -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_accepts_private_business_labeled_report -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
./scripts/ci/check-runbooks.sh
```

Output summary:

- Missing-`certificate` private-business field-quality reports are now rejected
  by both the library gate and CLI gate without printing the report path.
- Complete strict private-business field-quality reports still pass for both
  the library and CLI acceptance regressions.
- Full `benchmark-runner`, focused clippy, `cargo fmt --all --check`, and
  `check-runbooks.sh` passed locally.

Scope note:

- S213 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, certificate values, or
  location values from private resumes.

### S212

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_private_business_report_without_location_metric -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_requires_private_business_location_metric -- --exact
```

Output summary:

- The library exact test failed before implementation because removing
  `location` metrics from a strict private-business field-quality report still
  returned `Ok(FieldQualityGateEvaluation { ... })`.
- The CLI exact test failed before implementation because
  `resume-benchmark field-gate --require-private-business-labeled` exited
  successfully for a private-business report missing `location` metrics.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_private_business_report_without_location_metric -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_requires_private_business_location_metric -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_accepts_private_business_labeled_release_evidence -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_accepts_private_business_labeled_report -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
./scripts/ci/check-runbooks.sh
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Missing-`location` private-business field-quality reports are now rejected by
  both the library gate and CLI gate without printing the report path.
- Complete strict private-business field-quality reports still pass for both
  the library and CLI acceptance regressions.
- Full `benchmark-runner`, focused clippy, `cargo fmt --all --check`,
  `check-runbooks.sh`, `git diff --check`, and `guard-public-repo.sh` passed
  locally.
- Full local verification passed locally, including workspace tests and
  doc-tests, license/runbook/workflow/release-readiness checks, release
  artifact and SBOM checks, macOS package check, and the final public
  repository guard. Windows package check was skipped on this non-Windows host.

Scope note:

- S212 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, private labels, field values, or location values from
  private resumes.

### S211

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_labeled_location_values_with_exact_spans -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_any_location -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_location_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_location_before_fulltext_top_k_cutoff -- --exact
```

Output summary:

- The extractor focused test failed before implementation because
  `FieldType::Location` did not exist.
- The rank-fusion focused test failed before implementation because
  `SearchFilters::with_locations_any`, `ResumeProfile::with_locations`, and
  `locations_any()` did not exist.
- The direct CLI focused test failed before implementation because search
  rejected `--location` and printed the existing usage.
- The CLI IPC focused test failed before implementation because the CLI never
  connected to the fake daemon after rejecting the new location flag.
- The daemon IPC focused test failed before implementation because the
  unhandled `locations_any` filter allowed high-BM25 decoys to win the top-k
  window.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_labeled_location_values_with_exact_spans -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_any_location -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_labeled_location_mentions_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_location_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_location_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p rank-fusion -p import-pipeline -p resume-cli -p resume-daemon -p benchmark-runner --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused extractor, rank-fusion, CLI persisted-field, direct CLI filtered
  search, CLI IPC, and daemon IPC location tests passed after implementation.
- Full related suites passed: extractor-rules, rank-fusion, import-pipeline,
  benchmark-runner, CLI search filters, CLI persisted fields, CLI search IPC,
  and daemon search IPC.
- Focused clippy passed for the touched packages.
- `cargo fmt --all --check`, `git diff --check`, and
  `guard-public-repo.sh` passed locally.
- Full local verification passed locally, including workspace tests and
  doc-tests, license/runbook/workflow/release-readiness checks, release
  artifact and SBOM checks, macOS package check, and the final public
  repository guard. Windows package check was skipped on this non-Windows host.

Scope note:

- S211 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, contact hashes, or location values from private resumes.

### S210

Hosted RED check:

```bash
gh run view 27013854775 --log-failed
```

Output summary:

- PR #9 hosted `rust workspace` failed in
  `search_ipc_hashes_contact_filters_before_submitting_request`.
- The fake daemon assertion received the two valid `contact_hashes_any` hashes
  in a different order from the exact expected JSON array, then the test failed
  while joining the fake daemon thread.
- The failure was in the test assertion only: `contact_hashes_any` is an ANY
  filter, raw email/phone values were still absent from the request, and no raw
  contact or contact hash output leak was reported.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_hashes_contact_filters_before_submitting_request -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The hosted-failing exact test passed locally after comparing sorted actual
  and expected contact hash arrays.
- The full CLI search IPC test file passed locally with 8 tests.
- `cargo fmt --all --check`, `git diff --check`, and
  `guard-public-repo.sh` passed locally.
- Full local verification passed locally, including workspace tests and
  doc-tests, license/runbook/workflow/release-readiness checks, release
  artifact and SBOM checks, macOS package check, and the final public
  repository guard. Windows package check was skipped on this non-Windows host.

Scope note:

- S210 is test-only and uses synthetic/temp fixtures only. It does not read,
  print, commit, or upload private resumes, filenames, paths, raw text,
  diagnostics, tokens, model caches, OCR text, page images, command paths,
  vectors, raw contact values, or contact hashes.

### S209

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store searchable_document_ids_with_contact_hashes_matches_visible_versions_only -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_contact_hash_before_fulltext_top_k_cutoff_without_contact_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_hashes_contact_filters_before_submitting_request -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_contact_hash_before_fulltext_top_k_cutoff -- --exact
```

Output summary:

- The meta-store focused test failed before implementation because
  `searchable_document_ids_with_contact_hashes` did not exist.
- The direct CLI focused test failed before implementation because `search`
  rejected `--email` and `--phone`.
- The CLI IPC focused test failed before implementation because the rejected
  contact arguments prevented the request from reaching the fake daemon.
- The daemon IPC focused test failed before implementation because the daemon
  ignored `contact_hashes_any` and returned a high-BM25 decoy instead of the
  matching synthetic target.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store searchable_document_ids_with_contact_hashes_matches_visible_versions_only -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_contact_hash_before_fulltext_top_k_cutoff_without_contact_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_hashes_contact_filters_before_submitting_request -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_contact_hash_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p rank-fusion -p resume-cli -p resume-daemon --all-targets -- -D warnings
git diff --check
```

Output summary:

- Meta-store now returns visible searchable document IDs for candidate-assigned
  versions whose candidate email or phone hash matches a requested HMAC hash,
  excluding deleted documents, hidden versions, and non-searchable documents.
- Direct CLI search supports `--email`/`--emails-any` and `--phone`/`--phones-any`;
  the CLI normalizes contact values locally and hashes them through the existing
  contact HMAC key before search planning.
- CLI IPC search submits only `contact_hashes_any` in the authenticated loopback
  request and does not include raw email or phone values in the request body,
  stdout, or stderr.
- Daemon IPC parses and validates `contact_hashes_any`, prefilters matching
  document IDs before full-text top-k truncation, and keeps raw contact values
  plus contact hashes out of the response body.
- Related suites and focused clippy passed locally after formatting.

Scope note:

- S209 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, vectors, raw contact
  values, or contact hashes.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

Final checkpoint verification:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `cargo fmt --all --check`: exit 0.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0, `public repo guard passed`.
- `verify-local.sh`: exit 0, including workspace tests and doc-tests,
  license/runbook/workflow/release-readiness checks, release artifact and SBOM
  checks, macOS package check, and the final public repository guard. Windows
  package check was skipped on this non-Windows host.

### S208

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store searchable_document_ids_with_date_range_overlap_matches_visible_versions_only -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_overlapping_date_range -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_date_range_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_date_range_before_fulltext_top_k_cutoff -- --exact
```

Output summary:

- The meta-store focused test failed before implementation because
  `searchable_document_ids_with_date_range_overlap` did not exist.
- The rank-fusion focused test failed before implementation because
  `SearchFilters::with_date_range_overlaps`, `SearchFilters::date_range_overlaps`,
  and `ResumeProfile::with_date_ranges` did not exist.
- The direct CLI focused test failed before implementation because `search`
  rejected `--date-range-overlaps`.
- The CLI IPC focused test failed before implementation because the rejected
  date-range argument prevented the request from reaching the fake daemon.
- The daemon IPC focused test failed before implementation because the daemon
  applied no date-range prefilter before the full-text top-k cutoff and returned
  a high-BM25 decoy instead of the matching synthetic target.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store searchable_document_ids_with_date_range_overlap_matches_visible_versions_only -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_overlapping_date_range -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_date_range_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_date_range_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p rank-fusion -p resume-cli -p resume-daemon --all-targets -- -D warnings
```

Output summary:

- Rank-fusion now parses and matches overlapping date ranges, including
  open-ended `PRESENT` ranges.
- Meta-store now returns visible searchable document IDs whose high-confidence
  persisted `date_range` mentions overlap a query range, excluding deleted,
  hidden, low-confidence, and non-overlapping evidence.
- Direct CLI search supports `--date-range-overlaps` and applies the metadata
  document-ID prefilter before full-text top-k truncation.
- CLI and daemon IPC search requests carry `date_range_overlaps`; daemon IPC
  parsing, persisted profile hydration, and document-ID prefiltering now handle
  date-range entity mentions before full-text retrieval.
- Focused RED/GREEN tests, related meta-store, rank-fusion, CLI search-filter,
  CLI IPC, and daemon IPC suites, and focused clippy passed locally.

Final checkpoint verification:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `cargo fmt --all --check`: exit 0.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0, public repo guard passed.
- `verify-local.sh`: exit 0, including workspace tests/doc-tests,
  license/runbook/workflow/release-readiness checks, release artifact/SBOM
  checks, macOS package check, and final public repo guard.

Scope note:

- S208 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, or vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S207

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_any_school -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_school_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_school_before_fulltext_top_k_cutoff -- --exact
```

Output summary:

- The rank-fusion focused test failed before implementation because
  `SearchFilters::with_schools_any`, `SearchFilters::schools_any`, and
  `ResumeProfile::with_schools` did not exist.
- The direct CLI focused test failed before implementation because `search`
  rejected `--school`.
- The CLI IPC focused test failed before implementation because the rejected
  `--school` argument prevented the request from reaching the fake daemon.
- The daemon IPC focused test failed before implementation because the daemon
  applied no school prefilter before the full-text top-k cutoff and returned a
  school decoy instead of the matching synthetic target.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_any_school -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_school_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_school_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo clippy -p rank-fusion -p resume-cli -p resume-daemon --all-targets -- -D warnings
```

Output summary:

- School field filters now normalize persisted school entity values and match
  profile school evidence in rank-fusion.
- Direct CLI search supports `--school` and `--schools-any`, persists school
  profile evidence from metadata, and applies the school document-ID prefilter
  before full-text top-k truncation.
- CLI and daemon IPC search requests carry `schools_any`; daemon IPC parsing,
  persisted profile hydration, and document-ID prefiltering now handle school
  entity mentions before full-text retrieval.
- Focused RED/GREEN tests, related rank-fusion, CLI search-filter, CLI IPC, and
  daemon IPC suites, and focused clippy passed locally.

Final checkpoint verification:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `cargo fmt --all --check`: exit 0.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0, public repo guard passed.
- `verify-local.sh`: exit 0, including workspace tests/doc-tests,
  license/runbook/workflow/release-readiness checks, release artifact/SBOM
  checks, macOS package check, and final public repo guard.

Scope note:

- S207 uses synthetic/temp fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, diagnostics, tokens,
  model caches, OCR text, page images, command paths, or vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S206

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/check-release-readiness.sh
```

Output summary:

- The focused release-readiness suite failed before implementation because the
  hardware fault-drill blocker detail did not contain `actual ENOSPC`.
- The release-readiness CI guard failed before implementation because the JSON
  output also lacked `actual ENOSPC`.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/check-release-readiness.sh
./scripts/ci/check-runbooks.sh
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings
```

Output summary:

- Release-readiness text and JSON now explicitly list actual ENOSPC,
  service-level daemon kill, battery-mode, and external-drive disconnect drills
  as unproven release-platform blockers.
- The release blockers runbook now lists the same hardware fault-drill blocker
  in `Current BLOCKED Items`.
- The release-readiness guard and runbook guard passed without local path or
  private data marker leaks.

Final checkpoint verification:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Scope note:

- S206 uses synthetic/temp release-readiness data only. It does not read, print,
  commit, or upload private resumes, filenames, paths, raw text, diagnostics,
  tokens, model caches, OCR text, page images, command paths, or vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S205

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_company_and_title -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_company_and_title_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_company_and_title_before_fulltext_top_k_cutoff -- --exact
```

Output summary:

- The rank-fusion regression failed before implementation because company and
  title profile/filter APIs did not exist.
- The direct CLI regression failed before implementation because search usage
  rejected `--company` and `--title`.
- The CLI IPC regression failed before implementation because the command did
  not connect to the fake daemon after rejecting the new field filters.
- The daemon IPC regression failed before implementation because company/title
  filters were ignored and a decoy full-text hit won before post-filtering.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_company_and_title -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_company_and_title_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_company_and_title_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo clippy -p rank-fusion -p resume-cli -p resume-daemon --all-targets -- -D warnings
```

Output summary:

- Rank-fusion now normalizes company suffixes and title aliases before matching
  persisted profile evidence against field filters.
- A post-review Chinese company suffix regression first failed for
  `股份有限公司`, then passed after company normalization started stripping more
  specific Chinese legal suffixes before shorter suffixes.
- CLI direct search and daemon search IPC now prefilter `company` and `title`
  evidence before full-text top-k truncation.
- CLI search IPC serializes redacted filter payloads with `companies_any` and
  `titles_any`, and persisted-profile hydration carries company/title mentions
  into rank-fusion filtering.

Final checkpoint verification:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Scope note:

- S205 uses synthetic test fixtures only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, local diagnostics, tokens,
  model caches, private labels, OCR text, page images, command paths, or
  vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S204

Hosted CI failure evidence:

```bash
gh run view 27004152277 --job 79691476659 --log
git fetch origin pull/9/merge:refs/remotes/origin/pr/9/merge
git worktree add /tmp/resume-ir-pr9-merge refs/remotes/origin/pr/9/merge
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_school_tier_mentions_and_filters_search_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields
```

Output summary:

- Hosted PR #9 Rust workspace failed on Ubuntu in
  `import_persists_school_tier_mentions_and_filters_search_without_output_leaks`
  at the assertion that the whole `EntityMention` Debug string did not contain
  `985`.
- The PR merge worktree exact test and full local `s16_persisted_fields` test
  passed on macOS, pointing at a flaky assertion rather than a deterministic
  production behavior failure.
- Root cause: `EntityMention` Debug already redacts `raw_value` and
  `normalized_value`, but it also prints opaque hex IDs; those IDs can
  legitimately include the digit sequence `985`.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `s16_persisted_fields` passed with 11 tests, including the school-tier import
  and filter regression.
- Formatter check, whitespace diff check, public repo guard, and focused
  `resume-cli` clippy passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness, release artifact check,
  release SBOM check, macOS package check, Windows package skip on non-Windows,
  and final public repo guard.

Scope note:

- S204 uses synthetic test data and hosted CI logs only. It does not read,
  print, commit, or upload private resumes, filenames, paths, raw text, local
  diagnostics, tokens, model caches, private labels, OCR text, page images,
  command paths, or vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S203

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_any_certificate -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_certificates_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_certificates_before_fulltext_top_k_cutoff -- --exact
```

Output summary:

- The rank-fusion regression failed before implementation because
  `with_certificates`, `with_certificates_any`, and `certificates_any` did not
  exist.
- The direct CLI regression failed before implementation because search usage
  rejected `--certificate`.
- The CLI IPC regression failed before implementation because the command did
  not connect to the fake daemon after rejecting the certificate filter.
- The daemon IPC regression failed before implementation because
  `certificates_any` was ignored and a decoy hit won before post-filtering.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_any_certificate -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_certificates_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_certificates_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_sectioned_certificate_alias_mentions_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc
```

Output summary:

- Certificate profile/filter matching now accepts normalized persisted
  certificate values and common user input aliases such as `PMP` and `CKA`.
- CLI direct search and daemon search IPC now prefilter `certificate` evidence
  before full-text top-k truncation.
- CLI search IPC serializes `certificates_any`, and existing certificate
  extraction/persistence tests remain green.

Final checkpoint verification:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p rank-fusion -p resume-cli -p resume-daemon --all-targets -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Formatter check, focused rank-fusion/CLI/daemon clippy, whitespace diff check,
  and public repo guard passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S203 uses synthetic temp files and synthetic stores only. It does not read,
  print, commit, or upload private resumes, filenames, paths, raw text, local
  diagnostics, tokens, model caches, private labels, OCR text, page images,
  command paths, or vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S202

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_unknown_school_tier_when_no_tier_evidence_exists -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_unknown_school_tier_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite searchable_document_ids_without_entity_type_matches_visible_versions_only -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_unknown_school_tier_before_fulltext_top_k_cutoff -- --exact
```

Output summary:

- The rank-fusion regression failed before implementation because
  `SchoolTier::Unknown` did not match a profile with no school-tier evidence.
- The CLI regression failed before implementation because
  `--school-tier unknown` returned zero results after known-tier decoys consumed
  the full-text top-k candidate window.
- The meta-store regression failed before implementation because the missing
  entity-type helper did not exist.
- The daemon IPC regression failed before implementation because full-text
  search filtered only after top-k candidate retrieval, returning zero results.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_unknown_school_tier_when_no_tier_evidence_exists -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite searchable_document_ids_without_entity_type_matches_visible_versions_only -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_unknown_school_tier_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_prefilters_unknown_school_tier_before_fulltext_top_k_cutoff -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_school_tier_mentions_and_filters_search_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc
```

Output summary:

- `unknown` school-tier filters now match profiles with no tier evidence while
  excluding profiles with known tiers.
- The SQLite helper returns searchable visible versions without high-confidence
  `school_tier` mentions, treating low-confidence tier mentions as absent and
  excluding deleted, discovered, and hidden-only evidence.
- CLI and daemon full-text search prefilter `unknown` before top-k truncation,
  while existing known-tier import/filter and IPC behavior remain covered.

Final checkpoint verification:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p rank-fusion -p meta-store -p resume-cli -p resume-daemon --all-targets -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Formatter check passed.
- Focused clippy initially reported a collapsible nested `if` introduced in
  `rank-fusion`; after collapsing the condition, the same clippy command passed
  for rank-fusion, meta-store, resume-cli, and resume-daemon.
- Whitespace diff check and public repo guard passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S202 uses synthetic temp files and synthetic in-memory/file-backed stores
  only. It does not read, print, commit, or upload private resumes, filenames,
  paths, raw text, local diagnostics, tokens, model caches, private labels, OCR
  text, page images, command paths, or vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S201

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_private_business_report_without_school_tier_metric -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_requires_private_business_school_tier_metric -- --exact
```

Output summary:

- The library regression failed before implementation because a
  `private-business-labeled` field-quality report without `school_tier` metrics
  returned `Ok(FieldQualityGateEvaluation { ... })`.
- The CLI regression failed before implementation because
  `resume-benchmark field-gate --require-private-business-labeled` accepted a
  report missing `school_tier`.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_private_business_report_without_school_tier_metric -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_requires_private_business_school_tier_metric -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_accepts_private_business_labeled_release_evidence -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_gate_accepts_private_business_labeled_report -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner
./scripts/ci/check-runbooks.sh
```

Output summary:

- Private-business field-quality reports missing `school_tier` are now rejected
  with the existing production field metrics error.
- Complete strict private-business field-quality reports with `school_tier`
  remain accepted by both the library and CLI gate.
- Full `benchmark-runner` and runbook guard passed locally.

Final checkpoint verification:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Formatter check, focused benchmark-runner clippy, whitespace diff check, and
  public repo guard passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S201 uses synthetic test reports only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, local diagnostics, tokens,
  model caches, private labels, OCR text, page images, command paths, or
  vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S200

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_school_tier_values_inside_education_context -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_any_school_tier -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_school_tier_mentions_and_filters_search_without_output_leaks -- --exact
```

Output summary:

- The extractor regression failed before implementation because
  `FieldType::SchoolTier` did not exist.
- The rank-fusion regression failed before implementation because
  `SchoolTier`, `with_school_tiers`, and `with_school_tiers_any` did not exist.
- The CLI persisted-field regression failed before full implementation because
  import-pipeline did not map `FieldType::SchoolTier` to an entity type.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_school_tier_values_inside_education_context -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion field_filters_match_any_school_tier -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_school_tier_mentions_and_filters_search_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_authenticates_filters_and_redacts_results -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules -p rank-fusion -p import-pipeline -p meta-store -p benchmark-runner
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p rank-fusion -p import-pipeline -p meta-store -p benchmark-runner -p resume-cli -p resume-daemon --all-targets -- -D warnings
```

Output summary:

- Extractor-rules produced canonical `school_tier` values for synthetic
  `985/211/双一流` and overseas education evidence while ignoring a later
  non-education `985` skill sentence.
- Import persisted `school_tier` mentions with spans and redacted debug output,
  and direct `resume-cli search --school-tier 985` returned the synthetic Java
  candidate while `--school-tier overseas` returned zero results.
- CLI IPC emits `school_tiers_any` with canonical values, and daemon IPC parses
  that filter and applies it to persisted `SchoolTier` mentions.
- The related crate/test files, focused clippy, and formatter passed locally.

Final checkpoint verification:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Formatter check, whitespace diff check, and public repo guard passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S200 uses synthetic/temp data only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, local diagnostics, tokens,
  model caches, private labels, OCR text, page images, command paths, or
  vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S199

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_outputs_redacted_synthetic_json -- --exact
./scripts/ci/check-runbooks.sh
```

Output summary:

- The focused CLI regression failed before implementation because the redacted
  synthetic query benchmark report did not include
  `generation_mode: "streaming"`.
- The runbook guard failed before documentation because
  `docs/runbooks/release-blockers.md` did not mention
  `generation_mode: "streaming"`.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_outputs_redacted_synthetic_json -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner
./scripts/ci/check-runbooks.sh
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
```

Output summary:

- `run_synthetic_query_benchmark` now passes the synthetic document iterator
  directly to `FullTextIndex::replace_documents` instead of pre-collecting the
  whole document set into a `Vec`.
- Redacted synthetic benchmark JSON includes
  `generation_mode: "streaming"` while continuing to omit raw synthetic resume
  text, paths, and query strings.
- Full `benchmark-runner`, runbook guard, focused clippy, fmt, and diff checks
  passed locally.

Final checkpoint verification:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S199 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, model
  caches, private labels, OCR text, page images, runtime paths, command paths,
  or vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S198

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner ocr_throughput_gate_requires_private_real_release_boundary -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_ocr_gate_requires_private_real_corpus_report -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness release_readiness_json_reports_blockers_without_local_path_leaks -- --exact
./scripts/ci/check-runbooks.sh
```

Output summary:

- The focused OCR library regression failed before implementation because
  `OcrThroughputGateConfig::require_private_real_corpus` did not exist.
- The focused OCR CLI regression failed before implementation because
  `resume-benchmark ocr-gate --require-private-real-corpus` did not reject
  synthetic reports with `private real-corpus OCR benchmark required` or accept
  the strict private real-corpus report.
- The release-readiness JSON regression failed before implementation because
  the blocker list still had 13 items instead of the expected 14 with
  `OCR throughput`.
- The runbook policy check failed before documentation because
  `docs/runbooks/release-blockers.md` did not mention
  `ocr-gate --report private-ocr-throughput.json`.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner ocr_throughput_gate_requires_private_real_release_boundary -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_ocr_gate_requires_private_real_corpus_report -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness
./scripts/ci/check-runbooks.sh
./scripts/ci/check-release-readiness.sh
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `ocr-gate --require-private-real-corpus` rejects synthetic OCR throughput
  reports and accepts strict private real-corpus redacted aggregate OCR
  throughput reports.
- Private OCR reports must include dataset, OCR runtime, renderer, and language
  pack manifest digests, aggregate page latency/throughput, target claim
  `ocr_throughput_target_met`, and false raw OCR text/page image/resume-path/
  document-ID/page-ID/command-path booleans.
- Full `benchmark-runner`, full `s161_release_readiness`, runbook/readiness
  policy checks, focused `benchmark-runner` clippy, and focused `resume-cli`
  clippy passed locally.

Final checkpoint verification:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S198 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, model
  caches, private labels, OCR text, page images, runtime paths, command paths,
  or vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S197

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness release_readiness_reports_blocked_evidence_without_local_path_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness release_readiness_json_reports_blockers_without_local_path_leaks -- --exact
```

Output summary:

- The text regression failed before implementation because stdout did not
  contain `vector quality: blocked`.
- The JSON regression failed before implementation because release-readiness
  still emitted 12 blockers instead of 13.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness release_readiness_reports_blocked_evidence_without_local_path_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness release_readiness_json_reports_blockers_without_local_path_leaks -- --exact
./scripts/ci/check-runbooks.sh
./scripts/ci/check-release-readiness.sh
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
```

Output summary:

- The focused release-readiness text and JSON regressions passed after
  implementation.
- Full `s161_release_readiness`, focused CLI clippy, `cargo fmt --all --check`,
  `git diff --check`, `guard-public-repo.sh`, `check-runbooks.sh`, and
  `check-release-readiness.sh` passed locally.

Final checkpoint verification:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S197 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, model
  caches, private labels, or vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S196

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner vector_quality_gate_requires_private_business_labeled_release_boundary -- --exact
```

Output summary:

- The focused regression failed before implementation because
  `VectorQualityGateConfig::require_private_business_labeled` did not exist.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner vector_quality_gate_requires_private_business_labeled_release_boundary -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_vector_gate_requires_private_business_labeled_report -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
```

Output summary:

- The ordinary `labeled` vector-quality report is rejected when release
  evidence requires `private-business-labeled`.
- A private local redacted aggregate report with dataset, annotation, and model
  manifest digests, explicit false payload/leak booleans, fixed vector taxonomy,
  and aggregate recall/MRR/NDCG metrics is accepted.
- Full `benchmark-runner`, focused clippy, `cargo fmt --all --check`,
  `git diff --check`, and `guard-public-repo.sh` passed locally.

Final checkpoint verification:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S196 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, model
  caches, private labels, or vectors.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S195

Hosted CI failure evidence:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 30
gh run view 26997152668 --repo FrankQDWang/resume-ir --job 79669166277 --log
```

Output summary:

- PR #9 hosted Windows Platform CI failed after S194 while running
  `foreground_import_scheduler_processes_task_enqueued_after_startup`.
- The previously failing full-text and import/search tests passed in that
  hosted run before the daemon failure.
- The daemon failure was at `crates/daemon/tests/s4_daemon.rs:737`:
  `store.run_migrations().unwrap()` returned `MetaStoreError { kind:
  Migration }` after daemon readiness had already been observed.
- Root cause: the test helper reran metadata migrations from the test process
  against the same live data-dir while the foreground import worker loop was
  active, even though daemon readiness already proved migrations completed.

Focused checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_processes_task_enqueued_after_startup -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_processes_task_enqueued_after_startup -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
```

Output summary:

- The hosted-failing exact test already passed locally, matching a
  hosted-Windows timing race rather than a deterministic local product bug.
- After implementation, the exact startup-queue test passed locally.
- Full `s4_daemon`, focused daemon clippy, and `cargo fmt --all --check`
  passed locally.

Final checkpoint verification:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S195 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, or
  model caches.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S194

Hosted CI failure evidence:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 30
gh run view 26996795412 --repo FrankQDWang/resume-ir --job 79668116152 --log
```

Output summary:

- PR #9 hosted Windows Platform CI failed while running
  `top_n_snippets_are_generated_only_for_returned_hits`.
- The failure was at `index.commit().unwrap()` with Tantivy reporting
  `Access is denied. (os error 5)`.
- The root cause was a missing bounded retry around writer commit; adjacent
  full-text open and snapshot filesystem operations already retried transient
  Windows lock/access diagnostics.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext index_mutation_retries_transient_windows_access_denied -- --exact
```

Output summary:

- The focused regression failed before implementation because
  `retry_transient_index_mutation` did not exist.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext tests::index_mutation_retries_transient_windows_access_denied -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext top_n_snippets_are_generated_only_for_returned_hits -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
```

Output summary:

- The internal retry regression passed after implementation.
- The hosted-failing exact full-text snippet test passed locally.
- Full `index-fulltext`, focused clippy, `cargo fmt --all --check`,
  `git diff --check`, and `guard-public-repo.sh` passed locally.

Final checkpoint verification:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S194 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, or
  model caches.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S193

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules avoids_degree_aliases_outside_education_context -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_does_not_persist_degree_aliases_from_skill_lines -- --exact
```

Output summary:

- The extractor regression failed before implementation because `MS SQL` in a
  skill section produced Degree normalized values `["master", "bachelor"]`
  instead of only `["bachelor"]`.
- The CLI persisted-field regression failed before implementation because
  import persisted `MS` as a Degree raw value.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules avoids_degree_aliases_outside_education_context -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_does_not_persist_degree_aliases_from_skill_lines -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
```

Output summary:

- The exact degree-context extractor and exact CLI persisted-field regressions
  passed after implementation.
- Full `extractor-rules`, full persisted-field CLI tests, full
  `import-pipeline`, focused clippy, `cargo fmt --all --check`,
  `git diff --check`, and `guard-public-repo.sh` passed locally.

Benchmark regression and final checkpoint verification:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_report_scores_labeled_samples_without_raw_value_leakage -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The field-quality benchmark regression passed after the synthetic labeled
  fixture was updated to put bachelor evidence inside an `Education` section.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S193 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, or
  model caches.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S192

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_labeled_school_and_degree_values_with_alias_normalization -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_labeled_school_and_degree_mentions_without_output_leaks -- --exact
```

Output summary:

- The extractor regression failed before implementation because labeled school
  values kept `School:` / `学校：` in normalized values instead of value-only
  school evidence.
- The CLI persisted-field regression failed before implementation because
  persisted School raw values still contained label delimiters.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_labeled_school_and_degree_values_with_alias_normalization -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_labeled_school_and_degree_mentions_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
```

Output summary:

- The exact labeled school/degree extractor and exact CLI persisted-field
  regressions passed after implementation.
- Full `extractor-rules`, full persisted-field CLI tests, full
  `import-pipeline`, focused clippy, `cargo fmt --all --check`, and
  `git diff --check` passed locally before final guard/full-local verification.

Final checkpoint verification:

```bash
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `guard-public-repo.sh` passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S192 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, or
  model caches.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S191

Hosted CI failure:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 30
gh run view 26995104639 --repo FrankQDWang/resume-ir --job 79663111886 --log
```

Output summary:

- The PR Windows check failed in
  `foreground_daemon_can_be_killed_and_restarted_without_path_leak`.
- The failure was `assertion failed:
  killed.stdout.contains("resume-daemon foreground ready")` after the foreground
  daemon was killed.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s81_daemon_kill foreground_daemon_can_be_killed_and_restarted_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
```

Output summary:

- The exact daemon-kill test passed locally after the test waits for the
  foreground ready line before killing the child process.
- Full `resume-daemon`, focused daemon clippy, `cargo fmt --all --check`, and
  `git diff --check` passed locally before final guard/full-local verification.

Final checkpoint verification:

```bash
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `guard-public-repo.sh` passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S191 uses synthetic/temp data only. It does not read, print, commit, or
  upload private resumes, filenames, paths, raw text, local diagnostics,
  tokens, or model caches.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S190

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_broader_title_aliases_without_certificate_title_noise -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_title_alias_mentions_without_output_leaks -- --exact
```

Output summary:

- The extractor regression failed before implementation because no Title
  mentions were produced for the broader frontend, full-stack,
  machine-learning, data-scientist, DevOps, QA, engineering-manager, or
  solutions-architect role aliases.
- The CLI persisted-field regression failed before implementation because
  import persisted no Title mentions for those aliases.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_broader_title_aliases_without_certificate_title_noise -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_broader_title_alias_mentions_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
```

Output summary:

- The exact broader-title extractor and exact CLI persisted-field regressions
  passed after implementation.
- Full `extractor-rules`, full persisted-field CLI tests, full
  `import-pipeline`, focused clippy, `cargo fmt --all --check`, and
  `git diff --check` passed locally before final guard/full-local verification.

Final checkpoint verification:

```bash
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `guard-public-repo.sh` passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S190 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, or
  model caches.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S189

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_labeled_company_and_title_values_with_exact_spans -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_labeled_company_and_title_mentions_without_output_leaks -- --exact
```

Output summary:

- The extractor regression failed before implementation because labeled company
  values kept `Company:` / `公司：` in normalized values and Chinese `有限公司`
  suffixes were not stripped.
- The CLI persisted-field regression failed before implementation because
  persisted Company/Title raw values still contained label delimiters.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_labeled_company_and_title_values_with_exact_spans -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_labeled_company_and_title_mentions_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
```

Output summary:

- The exact labeled company/title extractor and exact CLI persisted-field
  regressions passed after implementation.
- Full `extractor-rules`, full persisted-field CLI tests, full
  `import-pipeline`, focused clippy, `cargo fmt --all --check`, and
  `git diff --check` passed locally before final guard/full-local verification.

Final checkpoint verification:

```bash
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `guard-public-repo.sh` passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S189 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, or
  model caches.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S188

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_open_ended_present_date_ranges_with_years_evidence -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_present_date_range_and_years_mentions_without_output_leaks -- --exact
```

Output summary:

- The extractor regression failed before implementation because no DateRange
  mentions were produced for `2020年1月 - 至今`, `Jan 2021 - Present`, or
  `2022.03 - Current`.
- The CLI persisted-field regression failed before implementation because no
  DateRange/YearsExperience mentions were persisted for the same synthetic
  present/current evidence.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_open_ended_present_date_ranges_with_years_evidence -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_present_date_range_and_years_mentions_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
```

Output summary:

- The exact present/current date-range extractor and exact CLI persisted-field
  regressions passed after implementation.
- Full `extractor-rules`, full persisted-field CLI tests, full
  `import-pipeline`, focused clippy, `cargo fmt --all --check`, and
  `git diff --check` passed locally before final guard/full-local verification.

Final checkpoint verification:

```bash
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `guard-public-repo.sh` passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S188 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, or
  model caches.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S187

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_chinese_mobile_numbers_without_country_prefix_or_separators -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_chinese_mobile_mentions_without_output_leaks -- --exact
```

Output summary:

- The extractor regression failed before implementation because compact
  `13800138000` produced no phone field, and separated local
  `139 0013 8001` was normalized as `+13900138001` instead of
  `+8613900138001`.
- The CLI persisted-field regression failed before implementation because only
  one redacted phone mention was persisted for the two synthetic mobile phone
  values.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_chinese_mobile_numbers_without_country_prefix_or_separators -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_chinese_mobile_mentions_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
```

Output summary:

- The exact China mobile phone extractor and exact CLI persisted-field
  regressions passed after implementation.
- Full `extractor-rules`, full persisted-field CLI tests, full
  `import-pipeline`, focused clippy, `cargo fmt --all --check`, and
  `git diff --check` passed locally before final guard/full-local verification.

Final checkpoint verification:

```bash
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `guard-public-repo.sh` passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S187 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, or
  model caches.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S186

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_chinese_year_month_date_ranges_with_years_evidence -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_chinese_date_range_and_years_mentions_without_output_leaks -- --exact
```

Output summary:

- The extractor regression failed before implementation because no DateRange was
  produced for `2020年1月 - 2024年3月`.
- The CLI persisted-field regression failed before implementation because no
  DateRange/YearsExperience mentions were persisted for the same synthetic
  evidence.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_chinese_year_month_date_ranges_with_years_evidence -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_chinese_date_range_and_years_mentions_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
```

Output summary:

- The exact Chinese date-range extractor and exact CLI persisted-field
  regressions passed after implementation.
- Full `extractor-rules`, full persisted-field CLI tests, full
  `import-pipeline`, focused clippy, `cargo fmt --all --check`, and
  `git diff --check` passed locally before final guard/full-local verification.

Final checkpoint verification:

```bash
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `guard-public-repo.sh` passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S186 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, or
  model caches.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S185

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_sectioned_skill_aliases_without_header_or_context_noise -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_sectioned_skill_alias_mentions_without_output_leaks -- --exact
```

Output summary:

- The sectioned extractor test failed before implementation with no skill
  values instead of `["Python", "TypeScript", "PostgreSQL", "Kubernetes", "Go",
  "Redis"]`, proving aliases under skill section headers were missed.
- The CLI persisted-field regression failed before implementation with no skill
  mentions instead of `["Go", "Kubernetes", "PostgreSQL", "Python", "Redis",
  "TypeScript"]`, proving the import path persisted the same missing evidence.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_sectioned_skill_aliases_without_header_or_context_noise -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_sectioned_skill_alias_mentions_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
```

Output summary:

- The exact sectioned skill and exact CLI persisted-field regressions passed
  after implementation.
- Full `extractor-rules`, full persisted-field CLI tests, full
  `import-pipeline`, focused clippy, `cargo fmt --all --check`, and
  `git diff --check` passed locally before final guard/full-local verification.

Final checkpoint verification:

```bash
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `guard-public-repo.sh` passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S185 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, or
  model caches.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S184

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_sectioned_certificate_aliases_without_header_noise -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_sectioned_certificate_alias_mentions_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_fullwidth_labeled_certificate_alias_with_exact_span -- --exact
```

Output summary:

- The sectioned extractor test failed before implementation with normalized
  certificate values `["认证"]` instead of `["pmp", "cka", "cissp",
  "cfa_level_1"]`, proving aliases under certificate section headers were
  missed and a header was extracted as a value.
- The CLI persisted-field regression failed before implementation with
  `["认证"]` instead of `["cfa_level_1", "cissp", "cka", "pmp"]`, proving the
  import path persisted the same bad certificate evidence.
- The fullwidth-labeled extractor regression failed before the span fix because
  byte index 7 was inside the `：` delimiter, proving labeled certificate spans
  needed UTF-8 delimiter-width accounting.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_sectioned_certificate_aliases_without_header_noise -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_fullwidth_labeled_certificate_alias_with_exact_span -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields import_persists_sectioned_certificate_alias_mentions_without_output_leaks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s16_persisted_fields
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo clippy -p extractor-rules -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
```

Output summary:

- The exact sectioned certificate, exact fullwidth-labeled certificate, and exact
  CLI persisted-field regressions passed after implementation.
- The first full extractor-rules pass failed because the existing AWS
  certificate test still expected the old free-text normalized value; since the
  product has not shipped, the expectation was updated to the canonical
  `aws_solutions_architect` value.
- The first focused clippy pass failed on a test-only `op_ref`; the assertion
  was simplified, then focused clippy passed.
- Full `extractor-rules`, full persisted-field CLI tests, full
  `import-pipeline`, focused clippy, `cargo fmt --all --check`, and
  `git diff --check` passed locally before final guard/full-local verification.

Final checkpoint verification:

```bash
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- `guard-public-repo.sh` passed.
- Full `verify-local.sh` passed, including workspace tests/doc-tests,
  license/runbook/workflow checks, release readiness check, release artifact
  check, release SBOM check, macOS package check, Windows package skip on
  non-Windows, and the final public repo guard.

Scope note:

- S184 uses synthetic data only. It does not read, print, commit, or upload
  private resumes, filenames, paths, raw text, local diagnostics, tokens, or
  model caches.
- Subagent-driven guidance was used as implementation discipline only; no
  separate subagent execution owner was spawned for this slice.

### S183

Remote red evidence:

```bash
gh run view 26990679379 --job 79649961389 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- PR #9 hosted Windows Platform CI failed in
  `purge_deleted_removes_empty_import_root_task_paths_without_path_leak`.
- The failure was `assertion failed: stdout.contains("purged import tasks: 1")`.
- The same pushed commit passed Rust workspace, macOS Platform CI, public
  repository guard, dependency tree, license policy, and runbook policy.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store purge_import_tasks_matches_windows_canonical_root_to_normalized_document_path -- --exact
```

Output summary:

- Failed before implementation with `left: 0` and `right: 1`, proving the purge
  did not match a Windows `\\?\\C:\\...` canonical import root against stored
  `file://c:/...` and `c:/...` document paths.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store purge_import_tasks_matches_windows_canonical_root_to_normalized_document_path -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store purge_import_tasks_for_deleted_document_roots_keeps_roots_with_visible_documents -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The Windows path-shape meta-store regression passed after implementation.
- The original empty-root retention/purge regression still passed.
- Full `s14_delete_search` passed: 8 tests.
- Full `meta-store` passed: 51 tests plus doc-tests.
- Focused clippy, `cargo fmt --all --check`, `git diff --check`,
  `guard-public-repo.sh`, and full `verify-local.sh` passed.

Scope note:

- S183 changes only internal purge matching keys; it does not print paths,
  upload data, relax purge counters, or change the redacted CLI surface.

### S182

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store purge_import_tasks_for_deleted_document_roots_keeps_roots_with_visible_documents -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_empty_import_root_task_paths_without_path_leak -- --exact
```

Output summary:

- The meta-store focused test failed before implementation because
  `MetaStore::purge_import_tasks_for_deleted_document_roots` did not exist.
- The first CLI test draft also proved `resume-cli import` does not accept a
  single-file root, so the regression was corrected to import a directory
  containing one synthetic DOCX fixture. The corrected focused test then failed
  before implementation because `purge --deleted` did not report
  `purged import tasks: 1`.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_empty_import_root_task_paths_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store purge_import_tasks_for_deleted_document_roots_keeps_roots_with_visible_documents -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The focused CLI purge regression passed after implementation and proved
  stdout reported import task/scope purge counts while omitting the data dir,
  private temp import root, requested root, and fixture filename.
- The focused meta-store regression passed and proved a deleted-document root
  with another visible document keeps its import task, while a now-empty root
  purges the task.
- Full `meta-store` passed: 50 tests plus doc-tests.
- Full `s14_delete_search` passed: 8 tests.
- Focused clippy, `cargo fmt --all --check`, `git diff --check`,
  `guard-public-repo.sh`, and full `verify-local.sh` passed.

Scope note:

- S182 removes import-root path surfaces only when deleted documents empty that
  root. It does not wipe SQLite free pages, audit every possible future metadata
  surface, or use real resume data.

### S181

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store contact_hash_assignment_records_conflict_without_hash_or_contact_leakage -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding candidate_review_conflicts_lists_multi_contact_conflicts_without_contact_or_hash_leak -- --exact
```

Output summary:

- The meta-store focused test failed before implementation because
  `MetaStore::candidate_contact_conflicts` did not exist.
- The CLI focused test failed before implementation because conflicting email
  and phone hashes still returned
  `InvalidPersistedValue { field: "candidate.contact_hash" }` instead of a
  reviewable conflict.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store contact_hash_assignment_records_conflict_without_hash_or_contact_leakage -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding candidate_review_conflicts_lists_multi_contact_conflicts_without_contact_or_hash_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s146_metadata_key_cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s147_metadata_key_rotation_cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The redacted conflict persistence test passed after implementation.
- The CLI conflict review test passed and proved output omitted raw contact
  values, contact hashes, and local paths.
- Full `meta-store` passed: 49 tests plus doc-tests.
- Full `s18_candidate_folding` passed: 4 tests.
- Focused clippy, `cargo fmt --all --check`, `git diff --check`, and
  `guard-public-repo.sh` passed.
- Full local verification first exposed stale V17 schema assertions in
  metadata-key backup and rotation CLI tests; after updating those expectations
  to V18, `s146_metadata_key_cli`, `s147_metadata_key_rotation_cli`, and full
  `verify-local.sh` passed, including workspace tests, doc-tests,
  license/runbook/workflow/release-readiness guards, release artifact/SBOM
  checks, macOS package check, Windows package skip on non-Windows, and public
  repo guard.

Scope note:

- S181 stores only version ID plus email/phone candidate IDs for contact
  conflicts. It does not store contact values, contact hashes, paths, names, or
  resume text, and it leaves the conflict for manual review instead of
  auto-merging candidates.

### S180

Remote red evidence:

```bash
gh run view 26988681026 --job 79643880780 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- PR #9 hosted Windows Platform CI failed in `cargo test --workspace --locked`.
- The failing test was
  `s8_fulltext::incremental_snapshot_inherits_replaces_and_excludes_documents`.
- The failure happened at the first `publish_snapshot(...).unwrap()` and
  returned `Io { diagnostic: "The process cannot access the file because
  another process has locked a portion of the file. (os error 33)" }`.
- Other PR checks for that pushed S179 commit had passed except
  `windows-latest`.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext snapshot_file_read_retries_transient_windows_lock_violation -- --exact
```

Output summary:

- Failed before implementation because `read_snapshot_file_with_retry` did not
  exist.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext tests::snapshot_file_read_retries_transient_windows_lock_violation -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --test s8_fulltext incremental_snapshot_inherits_replaces_and_excludes_documents -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The snapshot file-read retry regression passed after implementation.
- The hosted-failing exact `s8_fulltext` test passed locally.
- Full `index-fulltext` passed: 9 unit tests, 14 integration tests, and
  doc-tests.
- Focused clippy for `index-fulltext` passed.
- `cargo fmt --all --check`, `git diff --check`, and
  `guard-public-repo.sh` passed.
- Full `verify-local.sh` passed, including workspace tests and doc-tests,
  license/runbook/workflow/release-readiness guards, release artifact/SBOM
  checks, macOS package check, Windows package skip on non-Windows, and public
  repo guard.

Scope note:

- S180 extends the existing bounded transient Windows snapshot filesystem retry
  policy to snapshot file-read paths used during publish, fallback, and header
  probes. It does not change snapshot contents, expose local paths, upload real
  data, or clear production release blockers.

### S179

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_scan_errors_replace_and_query_without_exposing_path_digest -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_persists_scan_errors_without_path_leak -- --exact
```

Output summary:

- The meta-store test failed before implementation because
  `ImportScanErrorSummary` and `MetaStore::import_scan_error_breakdown` did not
  exist.
- The CLI test failed before implementation because `status` did not print
  `import scan error breakdown: permission_denied/read_directory=1`.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_scan_errors_replace_and_query_without_exposing_path_digest -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_persists_scan_errors_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused meta-store and CLI RED/GREEN tests passed after implementation.
- Full `meta-store` passed: 48 tests.
- Full `s9_import_search` passed: 26 tests.
- Full `s13_diagnostics` passed: 14 tests.
- Focused clippy for `meta-store` and `resume-cli` passed.
- `cargo fmt --all --check`, `git diff --check`, and
  `guard-public-repo.sh` passed.
- Full `verify-local.sh` passed, including workspace tests and doc-tests,
  license/runbook/workflow/release-readiness guards, release artifact/SBOM
  checks, macOS package check, Windows package skip on non-Windows, and public
  repo guard.

Scope note:

- S179 makes persisted scan errors actionable through redacted aggregates only.
  It does not prove whole-machine discovery coverage, real external-drive
  recovery, real cross-platform watcher behavior, or release readiness.

### S178

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding candidate_review_merge_and_split_control_default_search_folding_without_value_leak -- --exact
```

Output summary:

- The first run failed to compile because the new test was missing its local
  `searchable_versions` helper; after fixing the test harness, the same focused
  test failed correctly because `resume-cli` did not recognize
  `candidate-review`.

Focused implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding candidate_review_merge_and_split_control_default_search_folding_without_value_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store unassign_candidate_versions_clears_assignments_and_refreshes_count -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p rank-fusion -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The focused RED/GREEN candidate-review CLI test passed after implementation,
  proving redacted suggestion listing, explicit merge-driven search folding,
  split-driven unfolding, and no private value/path leakage in command output.
- The focused meta-store split test passed and proves candidate assignments are
  cleared transactionally while version counts are refreshed.
- Full `meta-store`, full `s18_candidate_folding`, and
  `s21_import_candidate_assignment` passed.
- Focused clippy for `meta-store`, `rank-fusion`, and `resume-cli` passed.
- `cargo fmt --all --check`, `git diff --check`, `rank-fusion`,
  `check-runbooks.sh`, and `guard-public-repo.sh` passed.
- Full `verify-local.sh` passed, including workspace tests and doc-tests,
  license/runbook/workflow/release-readiness guards, release artifact/SBOM
  checks, macOS package check, Windows package skip on non-Windows, and public
  repo guard.

Scope note:

- S178 is a local manual review workflow only. It does not certify dedupe
  quality, create private labels, resolve multi-contact conflicts, add a UI,
  prove million-corpus review-list latency, or clear stable release blockers.

### S177

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner dedupe_quality_report_scores_labeled_pairs_without_profile_leakage -- --exact
```

Output summary:

- The focused test failed with unresolved imports because `benchmark-runner` had
  no `run_dedupe_quality_jsonl`, `DedupeQualityGateConfig`, or
  `evaluate_dedupe_quality_gate_json` API.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner dedupe_quality_ -- --nocapture
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_dedupe_ -- --nocapture
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/check-release-readiness.sh
./scripts/ci/check-runbooks.sh
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner -p resume-cli --all-targets -- -D warnings
```

Output summary:

- The dedupe-quality focused runner suite passed for aggregate labeled-pair
  scoring without profile leakage, low-recall rejection, ordinary labeled gate
  acceptance, strict private-business report acceptance, strict private-business
  report rejection without release boundary, and unsupported extra payload field
  rejection.
- The CLI dedupe-quality focused suite passed for redacted report generation,
  ordinary gate acceptance, and strict private-business release evidence
  acceptance.
- The full benchmark-runner suite passed with 13 CLI tests and 35 runner tests.
- Release-readiness focused tests plus the release-readiness and runbook guards
  passed with the new `dedupe quality` blocker.
- Fmt and focused benchmark/CLI clippy passed.

Scope note:

- S177 is a quality evaluator and release-evidence validator only. It does not
  generate, sanitize, upload, or certify private dedupe-quality reports and does
  not clear the missing real business labeled dedupe-quality blocker.

### S176

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_rejects_release_evidence_without_private_business_boundary -- --exact
```

Output summary:

- The focused test failed with `E0599` because `FieldQualityGateConfig` did not
  have `require_private_business_labeled`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality_gate_ -- --nocapture
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_ -- --nocapture
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/check-release-readiness.sh
./scripts/ci/check-runbooks.sh
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner -p resume-cli --all-targets -- -D warnings
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The field-quality gate suite passed for ordinary labeled smoke acceptance,
  strict private-business acceptance, missing production field rejection, missing
  redacted boundary rejection, and unsupported extra payload field rejection.
- The CLI field-quality suite passed for normal labeled smoke gate behavior and
  the strict `--require-private-business-labeled` release evidence mode.
- The full benchmark-runner suite passed with 11 CLI tests and 29 runner tests.
- Release-readiness focused tests plus the release-readiness and runbook guards
  passed with the new `field extraction quality` blocker. The runbook guard was
  also hardened so required text that starts with `--` is parsed as text, not a
  `grep` option.
- Fmt, diff check, focused benchmark/CLI clippy, public repo guard, and full
  local verification passed.

Scope note:

- S176 is a release-evidence validator only. It does not generate, sanitize,
  upload, or certify private field-quality reports and does not clear the
  missing real business labeled field-quality blocker.

### S175

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner benchmark_gate_rejects_private_real_report_without_hot_hybrid_evidence -- --exact
```

Output summary:

- The focused test failed because the benchmark gate accepted a
  `private-real-corpus` report without hot-index hybrid query evidence:
  `unwrap_err()` received an `Ok` `BenchmarkGateEvaluation`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner benchmark_gate_ -- --nocapture
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_gate_accepts_private_real_corpus_release_report -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/check-release-readiness.sh
./scripts/ci/check-runbooks.sh
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner -p resume-cli --all-targets -- -D warnings
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The benchmark gate suite passed for synthetic allowance, private boundary
  enforcement, duplicate-key rejection, million-scale proof rejection, and the
  new hot-index hybrid evidence requirement.
- The CLI private real-corpus release-report gate fixture passed only after it
  included redacted hot-index hybrid query evidence.
- Release-readiness focused tests plus the release-readiness and runbook guards
  passed with the tightened hot-index hybrid benchmark blocker text.
- Full benchmark-runner tests, fmt, diff check, and focused benchmark/CLI clippy
  passed.
- Public repo guard and full local verification passed.

Scope note:

- S175 is a release-evidence validator only. It does not generate, sanitize,
  upload, or certify private benchmark reports and does not clear the missing
  real 100k/1M benchmark blocker.

### S174

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s174_ocr_manifest
```

Output summary:

- The focused suite failed because `resume-cli` did not recognize the `ocr`
  top-level command. Negative tests also failed through the same missing command
  path before the OCR runtime manifest validator existed.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s174_ocr_manifest
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/check-release-readiness.sh
./scripts/ci/check-runbooks.sh
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The focused OCR manifest suite passed: valid reviewed runtime, checksum
  mismatch rejection, and unreviewed-license rejection all avoid local path and
  payload leaks.
- The release-readiness suite and readiness/runbook guards passed.
- `cargo fmt --all --check`, `git diff --check`, focused CLI clippy, public repo
  guard, and full local verification passed.

Scope note:

- S174 is local governance evidence only. It does not bundle OCR runtimes, approve
  distribution, prove OCR quality, prove non-English language-pack behavior,
  prove full-library OCR throughput, or clear stable release readiness.

### S173

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle windows_service_dry_run_actions_do_not_touch_disk_or_leak_local_paths
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness
```

Output summary:

- `s66_service_lifecycle` failed because `resume-cli service` rejected
  `--platform windows-service`.
- `s161_release_readiness` failed because `Windows service lifecycle` was not
  emitted and the JSON blocker count was still 9.
- After the first platform implementation, the Windows Service dry-run test was
  tightened to remove `HOME` and omit `--launch-agent-dir`; it failed until the
  Windows Service platform stopped requiring a macOS LaunchAgent default.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle windows_service_dry_run_actions_do_not_touch_disk_or_leak_local_paths
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s161_release_readiness
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/check-release-readiness.sh
./scripts/ci/check-runbooks.sh
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The focused Windows Service dry-run test passed.
- The focused release-readiness suite passed with the separate Windows service
  lifecycle blocker.
- The full service lifecycle integration suite passed.
- Release-readiness and runbook guards passed.
- `cargo fmt --all --check`, `git diff --check`, focused CLI clippy, public repo
  guard, and full local verification passed.

Scope note:

- S173 is a redacted local dry-run evidence surface only. It does not perform
  real Windows service registration, administrator-elevated service control,
  service recovery, installer upgrade/uninstall/rollback, signing,
  notarization, or stable release validation.

### S172

Debug target:

- Investigate PR #9 hosted Windows Platform CI failure after S171.
- Fix root cause, not the symptom, and keep the change limited to the failing
  test harness path.

Observed failure:

- Hosted Windows `Test workspace` failed in
  `published_snapshot_becomes_active_without_reading_staging_orphans`.
- The failure was a direct synthetic fixture `fs::write` returning Windows
  `os error 33` while preparing an orphan staging file after publishing a
  snapshot.

Root cause:

- The test was bypassing the existing
  `write_snapshot_test_file_with_retry` helper that already handles transient
  Windows file-lock diagnostics for synthetic snapshot fixture writes.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --test s8_fulltext published_snapshot_becomes_active_without_reading_staging_orphans -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The hosted-failing exact test passed locally after the fixture write was moved
  to the retry helper.
- Full `index-fulltext`, formatting, diff check, public repo guard, and full
  local verification passed.

Scope note:

- S172 is a synthetic hosted-Windows test harness stability fix only. It does
  not change production full-text behavior or clear stable release blockers.

### S171

Design target:

- Rerun the existing explicit-root PDF/Word witness against the
  user-authorized local resume sample directory.
- Respect the privacy boundary: do not commit real paths, filenames, sample
  counts, raw text, private queries, diagnostics, or witness output.
- Bound OCR work while still proving the local `tesseract` plus `pdftoppm`
  witness path is usable on this machine.

Private local witness commands:

```bash
./target/debug/resume-cli witness --root <user-authorized-local-resume-root> --probe-search --probe-fields
./target/debug/resume-cli witness --root <user-authorized-local-resume-root> --max-files <bounded> --run-ocr --ocr-tesseract-command <local-tesseract> --ocr-pdftoppm-command <local-pdftoppm> --ocr-max-documents <bounded> --ocr-max-pages-per-document <bounded> --ocr-page-timeout-ms <bounded>
```

Output summary:

- The import/search/field witness completed locally and removed private witness
  data. Output was aggregate and redacted.
- The bounded OCR witness completed locally with local `tesseract` and
  `pdftoppm`, then removed private witness data.
- Real paths, filenames, raw resume text, private queries, diagnostics, and
  sample counts were intentionally not recorded in this repository.

Scope note:

- S171 is private local witness evidence only. It does not prove full-library
  OCR completion, large-corpus performance, semantic/vector quality,
  cross-platform service lifecycle, installer validation, signing,
  notarization, OCR/model licensing, or stable release readiness.

### S170

Design target:

- Close part of the P6 daemon resilience gap from the acceptance docs: prove a
  daemon killed during an active import can be restarted and recover the
  interrupted running import task.
- Preserve production defaults: the normal stale-running threshold remains 15
  minutes, and the normal retry backoff remains 60 seconds.
- Preserve privacy: use synthetic fixtures only and keep daemon output free of
  local data-dir/root paths.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_recovers_active_import_after_kill_and_restart -- --nocapture
```

Output summary:

- The focused daemon test failed before implementation because the restarted
  worker rejected `--stale-import-task-seconds` and
  `--import-retry-backoff-seconds`, leaving no deterministic local drill path for
  freshly killed active imports.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_recovers_active_import_after_kill_and_restart -- --nocapture
/Users/frankqdwang/.cargo/bin/cargo fmt --all --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The active-import kill/restart regression passed after daemon worker-loop
  options for stale-running recovery and retry backoff were added.
- Full S4 daemon tests passed: 13 tests, 0 failures.
- Full local verification passed, including workspace tests, doc-tests,
  license/runbook/workflow/release-readiness/release-artifact/SBOM/package
  guards, and public repo guard.

Scope note:

- S170 is synthetic active-import daemon recovery coverage only. It does not
  prove real service-level destructive chaos, platform installer/service
  lifecycle, real external-drive interruption, real-corpus scale recovery,
  signing, notarization, OCR/model licensing, or stable release readiness.

### S169

Design target:

- Close part of the P6 fault-injection coverage gap from the acceptance docs:
  battery-mode and external-drive disconnect should have local drill surfaces.
- Preserve privacy and safety: do not switch host power state, unmount drives,
  fill disks, print local paths, or claim real hardware evidence.
- Keep the release gate fail-closed: real battery/external-drive hardware drills
  remain blocked until platform-specific evidence exists.

Observed RED:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s71_fault_injection fault_simulate_battery_mode_reproduces_degradation_without_path_leak --locked -- --exact
```

Output summary:

- The focused fault-injection test failed before implementation because
  `fault-simulate` rejected `--case battery-mode` and printed the old usage.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s71_fault_injection fault_simulate_battery_mode_reproduces_degradation_without_path_leak --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s71_fault_injection fault_simulate_external_drive_disconnect_reproduces_without_path_leak --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s161_release_readiness --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s71_fault_injection --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s13_diagnostics --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/check-release-readiness.sh
./scripts/ci/check-runbooks.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p resume-cli --all-targets --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
./scripts/ci/guard-public-repo.sh
```

Output summary:

- The battery-mode and external-drive-disconnect focused tests passed after
  safe redacted simulation cases were added.
- The diagnostics, release-readiness, and runbook checks passed, proving the new
  hooks are advertised while real hardware drills remain blocked in the
  stable-release gate and documented as safe local probes only.
- Full fault-injection, diagnostics, fmt, focused clippy, full local
  verification, and public repo guard passed.

Scope note:

- S169 is safe synthetic fault-drill coverage only. It does not prove actual
  battery-mode transition behavior, real external-drive disconnect recovery,
  platform hardware behavior, destructive ENOSPC, service-level chaos drills,
  real-corpus performance, or stable release readiness.

### S168

Design target:

- Close part of the P6 observability gap from the acceptance docs: local status
  and diagnostics should expose query P50/P95/P99 rather than relying only on
  external benchmark reports.
- Preserve privacy: runtime query telemetry must not store query text, filter
  values, paths, filenames, snippets, or resume text.
- Keep the claim bounded: this is local telemetry for successful searches, not
  proof of the real-corpus `<200ms` hybrid P95 target.

Observed RED:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p meta-store --test s3_sqlite query_observations_are_aggregated_without_query_text --locked -- --exact
```

Output summary:

- The focused meta-store test failed before implementation because
  `MetaStore::record_query_observation` and
  `StoreStatusSummary::query_latency` did not exist.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p meta-store --test s3_sqlite query_observations_are_aggregated_without_query_text --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s9_import_search import_fixtures_builds_searchable_index_and_reopens_snapshot --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_authenticates_filters_and_redacts_results --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p meta-store --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s4_cli --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s13_diagnostics --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s9_import_search --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-daemon --test s20_ipc --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-daemon --test s48_search_ipc --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p meta-store -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
./scripts/ci/guard-public-repo.sh
```

Output summary:

- The meta-store focused test passed after V17 migration and query telemetry
  aggregation were added.
- The CLI focused test passed, proving successful local searches update
  telemetry and status, doctor, and redacted diagnostics report P50/P95/P99
  without the raw query or local paths.
- The daemon focused test passed, proving full-text search-over-IPC updates
  telemetry and daemon status JSON reports redacted aggregate latency.
- Full meta-store, S4 status, S9 import/search, S13 diagnostics, daemon S20
  status IPC, daemon S48 search IPC, fmt, focused clippy, full local
  verification, and public repo guard passed.

Scope note:

- S168 is local synthetic telemetry. It does not prove production query latency,
  real-corpus benchmark targets, semantic/vector quality, cross-platform
  performance, installer/service validation, signing, notarization, model/OCR
  licensing, or stable release readiness.

### S167

Design target:

- Close the P1 functional gap for full-text updates that previously rebuilt
  from metadata for every import, OCR text write, and soft-delete.
- Keep the privacy boundary: do not use Tantivy term-deletes as the durable
  published representation because stale stored text can remain in old segments;
  synthesize the next snapshot document set and publish a new encrypted snapshot
  instead.
- Preserve the old full metadata rebuild behavior when the active snapshot is
  unreadable, so index corruption does not block import recovery.

Observed RED:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext --test s8_fulltext incremental_snapshot_inherits_replaces_and_excludes_documents --locked -- --exact
```

Output summary:

- The focused full-text test failed before implementation with unresolved import
  `index_fulltext::publish_incremental_snapshot`.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext --test s8_fulltext incremental_snapshot_inherits_replaces_and_excludes_documents --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s9_import_search import_rebuilds_from_metadata_when_active_snapshot_is_unreadable --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p import-pipeline --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s9_import_search --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s14_delete_search --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s15_ocr_handoff --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p index-fulltext -p import-pipeline -p resume-cli --all-targets --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
./scripts/ci/guard-public-repo.sh
```

Output summary:

- The incremental full-text test passed, proving unchanged active docs are
  inherited, replacement doc_ids override older versions, and deleted doc_ids do
  not remain searchable.
- The corrupt-active regression passed, proving import falls back to metadata
  rebuild when active snapshot synthesis cannot read the current snapshot.
- Full `index-fulltext`, `import-pipeline`, S9 import/search, S14 delete/search,
  S15 OCR handoff, fmt, focused clippy, full local verification, and public
  repo guard passed.

Scope note:

- S167 is a synthetic local incremental full-text update slice. It does not
  prove million-scale incremental update latency, real-corpus behavior,
  cross-platform watcher soak, installer/service lifecycle, signing,
  notarization, OCR/model licensing, or stable release readiness.

### S166

Debug target:

- Fix the S165 pushed PR #9 hosted Windows Platform CI failure without changing
  production full-text fallback behavior.
- Identify whether the failure came from the S165 release workflow guard, a
  production full-text regression, or a hosted Windows test harness gap.
- Keep the fix synthetic-only and avoid committing paths, private data,
  diagnostics, model caches, or resume text.

Observed RED:

```bash
gh run view 26976094592 --repo FrankQDWang/resume-ir --job 79603359755 --log-failed
```

Output summary:

- Hosted Windows Platform CI failed in
  `s8_fulltext::active_snapshot_corruption_falls_back_to_last_good_snapshot`.
- The failing line directly overwrote `fulltext.snapshot.enc` to simulate
  active snapshot corruption and received Windows `os error 33`: another process
  had locked a portion of the file.
- The same run had already built the workspace; dependency, license, runbook,
  public guard, Rust Workspace, and macOS Platform CI were green.

Root cause:

- The test's corruption setup used bare `fs::write` immediately after
  `publish_snapshot`, bypassing the transient snapshot filesystem retry logic
  that production snapshot publication already uses for Windows file locks.
- The fallback behavior under test was not the failure point; the failure was
  the test's synthetic corruption write racing Windows file lock release.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext --test s8_fulltext active_snapshot_corruption_falls_back_to_last_good_snapshot --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p index-fulltext --test s8_fulltext --locked -- -D warnings
```

Output summary:

- The hosted-failing exact test passed locally.
- Full `index-fulltext` passed, including 8 unit tests, 13 integration tests,
  and doc-tests.
- Focused clippy and fmt passed.

Scope note:

- S166 adds a bounded retry only to the synthetic test corruption write. It does
  not change production full-text snapshot fallback behavior, prove the hosted
  Windows rerun until the branch is pushed, or clear any product release
  blockers.

### S165

Design target:

- Make the hosted Release dry-run workflow explicitly run the fail-closed
  release-readiness guard, instead of relying only on indirect `verify-local`
  coverage.
- Keep stable release blocked; this is a release-chain safety gate, not release
  approval.
- Keep all release-readiness evidence aggregate and redacted, without local
  data-dir paths, resume text, diagnostics, tokens, model cache paths, or private
  corpus details.

Observed RED:

```bash
./scripts/ci/check-release-readiness.sh
./scripts/ci/check-workflows.sh
```

Output summary:

- Both focused guards failed because `.github/workflows/release.yml` did not
  contain explicit `./scripts/ci/check-release-readiness.sh` wiring.

Implementation checks:

```bash
./scripts/ci/check-release-readiness.sh
./scripts/ci/check-workflows.sh
sh -n scripts/ci/check-release-readiness.sh scripts/ci/check-workflows.sh scripts/ci/verify-local.sh
git diff --check
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/release.yml"); puts "release workflow yaml ok"'
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Release-readiness and workflow guards passed after the Release dry-run job got
  an explicit `Confirm stable release remains blocked` step that runs
  `./scripts/ci/check-release-readiness.sh`.
- Shell syntax, workflow YAML parse, and diff checks passed.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license, runbook, workflow, release-readiness, release artifact, release
  SBOM, macOS package, Windows package skip-on-non-Windows, and public repo
  guards all passed.

Scope note:

- S165 strengthens Release workflow gating only. It does not create a stable
  release, sign or notarize artifacts, upload a GitHub Release, validate
  installer/service lifecycle, provide licensed OCR/model distribution evidence,
  or prove private 100k/1M benchmark targets.

### S164

Design target:

- Wire the fail-closed `release-readiness --json` command into local CI so
  stable release blockers cannot silently regress to test-only coverage.
- Keep stable release blocked unless all current release evidence exists.
- Keep output aggregate and redacted: no local data-dir paths, resume text,
  diagnostics, tokens, model cache paths, or private corpus details.

Observed RED:

```bash
./scripts/ci/check-release-readiness.sh
```

Output summary:

- The focused shell check failed with exit 127 because
  `scripts/ci/check-release-readiness.sh` did not exist.

Implementation checks:

```bash
./scripts/ci/check-release-readiness.sh
./scripts/ci/check-workflows.sh
./scripts/ci/check-runbooks.sh
sh -n scripts/ci/check-release-readiness.sh scripts/ci/check-workflows.sh scripts/ci/check-runbooks.sh scripts/ci/verify-local.sh
git diff --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The new release-readiness guard passed by confirming
  `resume-cli release-readiness --json` exits nonzero, emits
  `release-readiness.v1`, reports `stable_release: "blocked"`, reports
  `local_dry_run_artifacts: "evidence_only"`, includes all eight blocked
  release criteria, writes the expected stderr blocker message, and does not
  print the synthetic private-looking data-dir path or local runtime markers.
- Workflow and runbook guards passed, confirming `verify-local` now runs the
  release-readiness guard and the release blockers runbook documents the
  explicit blocked command.
- Shell syntax and diff checks passed.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license, runbook, workflow, release-readiness, release artifact, release
  SBOM, macOS package, Windows package skip-on-non-Windows, and public repo
  guards all passed.

Scope note:

- S164 adds CI enforcement for the existing release-readiness blocker gate only.
  It does not clear signing certificates, notarization, real-corpus benchmark
  evidence, licensed OCR/model distribution, installer/service lifecycle proof,
  or cross-platform validation blockers.

### S163

Design target:

- Stabilize hosted Ubuntu `s11_embedder` after the S162 push made the Rust
  Workspace job fail in the slow local embedding command timeout test.
- Treat the failure as a process-spawning test harness concurrency problem:
  multiple local command tests in the same binary can overlap process-group
  cleanup and slow synthetic command timing.
- Keep the fix test-only and synthetic-only, without changing production
  embedding runtime behavior or exposing local paths, text, diagnostics, model
  caches, or private data.

Observed RED:

```bash
gh run view 26974398706 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- Hosted Ubuntu `rust workspace` failed in
  `local_command_embedder_times_out_and_keeps_input_file_private`.
- The assertion expected `EmbeddingError::Timeout`, but the hosted run returned
  `EmbeddingError::EngineFailed`.
- Security, macOS Platform CI, and Windows Platform CI passed on the same
  pushed S162 branch tip.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p embedder --test s11_embedder local_command_embedder_times_out_and_keeps_input_file_private --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p embedder --test s11_embedder --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
git diff --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p embedder --test s11_embedder --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- The hosted-failing exact embedder timeout test passed locally.
- Full `s11_embedder` passed with 7 tests after implementation.
- Fmt, focused clippy, diff check, full local verification, and public-repo
  guard passed. Full local verification included workspace tests/doc-tests,
  license/runbook/workflow checks, release artifact/SBOM checks, macOS package
  check, and the public repository guard; Windows package check was skipped on
  non-Windows by the guard script.

Scope note:

- S163 serializes Unix local embedding command process tests only. It does not
  change production embedding runtime behavior, prove hosted Rust Workspace CI
  passes until PR #9 reruns, or clear licensed model, large-corpus performance,
  installer/service, signing, notarization, or release blockers.

### S162

Design target:

- Make the fail-closed release-readiness gate machine-readable for CI, release
  automation, and future local evidence collectors.
- Preserve the text mode and nonzero exit behavior; JSON mode must still mean
  stable release is blocked, not complete.
- Keep the output aggregate and redacted: no local data-dir paths, resume text,
  diagnostics, tokens, model cache paths, or private corpus details.

Observed RED:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s161_release_readiness --locked
```

Output summary:

- The existing text release-readiness test still passed.
- The new JSON release-readiness test failed because stdout was empty for
  `release-readiness --json`, so the JSON parser hit EOF.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s161_release_readiness --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
git diff --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p resume-cli --test s161_release_readiness --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused `s161_release_readiness` passed with 2 tests after implementation.
- Fmt, focused clippy, diff check, full local verification, and public-repo
  guard passed. Full local verification included workspace tests/doc-tests,
  license/runbook/workflow checks, release artifact/SBOM checks, macOS package
  check, and the public repository guard; Windows package check was skipped on
  non-Windows by the guard script.

Scope note:

- S162 adds machine-readable blocked release evidence only. It does not clear
  signing/notarization, real 100k/1M private benchmark, licensed OCR/model
  distribution, platform installer/service lifecycle, or cross-platform release
  validation blockers.

### S161

Design target:

- Add a local CLI release-readiness gate that makes stable-release blockers
  explicit instead of letting dry-run artifacts or green PR checks imply launch
  readiness.
- Fail closed until signing certificates, notarization, installer lifecycle,
  real 100k/1M benchmark evidence, OCR/model license/distribution decisions,
  and cross-platform validation have current evidence.
- Keep output aggregate and redacted: no local data-dir paths, resume text,
  diagnostics, tokens, model cache paths, or private corpus details.

Observed RED:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s161_release_readiness --locked
```

Output summary:

- The focused test failed because `resume-cli` had no `release-readiness`
  command and stdout did not contain the release-readiness report.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s161_release_readiness --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
git diff --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p resume-cli --test s161_release_readiness --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused `s161_release_readiness` passed with 1 test.
- Fmt, focused clippy, diff check, full local verification, and public-repo
  guard passed. Full local verification included workspace tests/doc-tests,
  license/runbook/workflow checks, release artifact/SBOM checks, macOS package
  check, and the public repository guard; Windows package check was skipped on
  non-Windows by the guard script.

Scope note:

- S161 adds a fail-closed stable-release readiness gate only. It does not clear
  signing/notarization, real 100k/1M private benchmark, licensed OCR/model
  distribution, platform installer/service lifecycle, or cross-platform release
  validation blockers.

### S160

Design target:

- Stabilize hosted Windows `s9_import_search` after S159 cleared Ubuntu Rust
  Workspace but Windows Platform CI moved the same redacted
  `search index update failed` class into local-discovery import.
- Avoid further production retry broadening without a new concrete filesystem
  error class; serialize only this Windows-heavy CLI import/search test binary
  so full-text snapshot rebuilds do not run concurrently inside it.
- Keep macOS/Linux test concurrency unchanged and keep the change synthetic-only
  without exposing paths, resume text, diagnostics, or local data.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s9_import_search --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
git diff --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p resume-cli --test s9_import_search --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
./scripts/ci/guard-public-repo.sh
```

Output summary:

- Full `s9_import_search` passed all 25 tests under the default local test
  harness.
- Fmt, focused clippy, diff check, full local verification, and public-repo
  guard passed. Full local verification included workspace tests/doc-tests,
  license/runbook/workflow checks, release artifact/SBOM checks, macOS package
  check, and the public repository guard; Windows package check was skipped on
  non-Windows by the guard script.

Scope note:

- S160 serializes the Windows side of the `s9_import_search` test binary only.
  It does not change production import/search or full-text behavior, prove
  hosted Windows CI passes until PR #9 reruns, or clear large-corpus
  performance, OCR/model quality, installer/service, signing, notarization, or
  release blockers.

### S159

Design target:

- Stabilize hosted Ubuntu `s12_ocr_client` after S158 made Platform CI green but
  Rust Workspace failed in the slow local OCR command timeout test with
  `EngineFailed` instead of `Timeout`.
- Avoid changing OCR runtime error classification for externally terminated
  commands; serialize the Unix process-spawning tests inside the OCR client test
  binary so timeout/cancel/process-group cleanup cases do not run concurrently.
- Keep the change synthetic-only and local-test-only, without exposing paths,
  resume text, diagnostics, or local data.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p ocr-client --test s12_ocr_client local_command_worker_times_out_and_does_not_report_late_output --locked -- --exact
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p ocr-client --test s12_ocr_client --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
git diff --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p ocr-client --test s12_ocr_client --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
./scripts/ci/guard-public-repo.sh
```

Output summary:

- Focused slow-command timeout test passed.
- Full `s12_ocr_client` passed all 17 tests under the default test harness.
- Fmt, focused clippy, diff check, full local verification, and public-repo
  guard passed. Full local verification included workspace tests/doc-tests,
  license/runbook/workflow checks, release artifact/SBOM checks, macOS package
  check, and the public repository guard; Windows package check was skipped on
  non-Windows by the guard script.

Scope note:

- S159 serializes OCR client Unix process-spawning tests only. It does not
  change production OCR process cleanup behavior, prove hosted Rust Workspace
  CI passes until PR #9 reruns, or clear large-corpus performance, OCR/model
  quality, installer/service, signing, notarization, or release blockers.

### S158

Design target:

- Stabilize hosted Windows `s14_delete_search` after S156/S157 moved the same
  redacted `search index update failed` across three independent tests in the
  same integration-test binary.
- Avoid further broadening production retry behavior without a concrete new
  error class; instead serialize this Windows-sensitive test harness section so
  multiple heavy CLI subprocesses do not rebuild/purge full-text snapshots at
  the same time inside one test binary.
- Keep the change synthetic-only and local-test-only, without exposing paths,
  resume text, diagnostics, or local data.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s14_delete_search --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
git diff --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p resume-cli --test s14_delete_search --locked -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Full `s14_delete_search` passed all 7 tests under the default test harness.
- Fmt, focused clippy, diff check, and full local verification passed. Full
  local verification included workspace tests/doc-tests, license/runbook/
  workflow checks, release artifact/SBOM checks, macOS package check, and the
  public repository guard; Windows package check was skipped on non-Windows by
  the guard script.

Scope note:

- S158 serializes a Windows-sensitive integration test binary only. It does not
  change production full-text behavior, prove all hosted Windows jobs pass until
  PR #9 reruns, or clear large-corpus performance, OCR/model quality,
  installer/service, signing, notarization, or release blockers.

### S157

Design target:

- Stabilize hosted Windows multi-root reimport after S156 moved the failure from
  the delete command test to the multi-root delete/reimport test.
- Treat Windows `ERROR_DIR_NOT_EMPTY` / `os error 145` as transient for snapshot
  filesystem cleanup, because recursive directory removal can partially remove
  a staging tree while Tantivy file handles are still settling.
- Keep the retry bounded and local-only, without printing snapshot paths,
  resume text, diagnostics, or local data.

Observed RED:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext transient_snapshot_fs_operation_retries_windows_directory_not_empty --locked
```

Output summary:

- The focused retry test failed because `ErrorKind::DirectoryNotEmpty` /
  `os error 145` was not treated as a transient snapshot filesystem cleanup
  error.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext transient_snapshot_fs_operation_retries_windows_directory_not_empty --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s14_delete_search multi_root_reimport_marks_missing_files_deleted_per_root --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s14_delete_search --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p index-fulltext -p resume-cli --all-targets --locked -- -D warnings
git diff --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused RED/GREEN tests passed after implementation.
- Full `index-fulltext` passed: 8 unit tests, 13 integration tests, and
  doc-tests.
- Full `s14_delete_search` passed all 7 tests, including the hosted-failing
  multi-root reimport case.
- Fmt, focused clippy, diff check, and full local verification passed. Full
  local verification included workspace tests/doc-tests, license/runbook/
  workflow checks, release artifact/SBOM checks, macOS package check, and the
  public repository guard; Windows package check was skipped on non-Windows by
  the guard script.

Scope note:

- S157 hardens transient Windows full-text directory cleanup only. It does not
  prove all hosted Windows jobs pass until PR #9 reruns, and it does not clear
  large-corpus performance, OCR/model quality, installer/service, signing,
  notarization, or release blockers.

### S156

Design target:

- Stabilize `resume-cli delete` full-text rebuilds on hosted Windows after PR #9
  failed with `search index update failed` in the delete/search suite.
- Treat Windows share/lock violations as transient during full-text index open
  and snapshot publish/cleanup, while keeping retry bounded and local-only.
- Do not print snapshot paths, resume text, diagnostics, or local data.

Observed RED:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext index_open_retries_transient_windows_share_violation --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext transient_snapshot_fs_operation_retries_extended_windows_lock_release --locked
```

Output summary:

- `index_open_retries_transient_windows_share_violation` failed because the
  full-text index open retry path did not treat a Windows `os error 32` share
  violation as transient.
- `transient_snapshot_fs_operation_retries_extended_windows_lock_release` failed
  because the existing snapshot filesystem retry exhausted before a longer
  Windows `os error 33` lock-release sequence.

Implementation checks:

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext index_open_retries_transient_windows_share_violation --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext transient_snapshot_fs_operation_retries_extended_windows_lock_release --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test s14_delete_search delete_soft_tombstones_document_and_removes_it_from_default_search --locked
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy -p index-fulltext -p resume-cli --all-targets --locked -- -D warnings
git diff --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused RED/GREEN tests passed after implementation.
- Full `index-fulltext` passed: 7 unit tests, 13 integration tests, and
  doc-tests.
- The hosted-failing delete regression passed locally.
- Fmt, focused clippy, diff check, and full local verification passed. Full
  local verification included workspace tests/doc-tests, license/runbook/
  workflow checks, release artifact/SBOM checks, macOS package check, and the
  public repository guard; Windows package check was skipped on non-Windows by
  the guard script.

Scope note:

- S156 hardens transient Windows full-text file-lock handling only. It does not
  prove all hosted Windows jobs pass until PR #9 reruns, and it does not clear
  large-corpus performance, OCR/model quality, installer/service, signing,
  notarization, or release blockers.

### S155

Design target:

- Add a fail-closed release evidence gate for local private real-corpus query
  benchmark reports without uploading real resumes, queries, filenames, paths,
  sample IDs, diagnostics, or local data.
- Preserve existing synthetic smoke behavior: synthetic reports still require
  explicit `--allow-synthetic` and cannot prove 100k/1M performance.
- Require strict aggregate-only `private-real-corpus` JSON with local/redacted
  boundary markers, false raw-data/path/query booleans, sha256 corpus/query
  digests, fixed target claim, and typed numeric/string fields.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner benchmark_gate_requires_private_real_corpus_metadata_for_release_evidence --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner resume_benchmark_gate_accepts_private_real_corpus_release_report --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner benchmark_gate_rejects_private_real_report_without_boundary_even_without_release_flag --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner benchmark_gate_rejects_private_real_report_with_extra_payload_field --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner benchmark_gate_rejects_private_real_report_with_payload_in_allowed_fields --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner benchmark_gate_rejects_private_real_report_with_duplicate_payload_keys --locked -- --exact
```

Output summary:

- The first focused tests failed because the release-gate API and CLI flags did
  not exist.
- Review-fix regressions then failed because private real-corpus validation was
  conditional on the release flag, accepted extra top-level payload fields, and
  accepted string payloads inside allowed aggregate fields.
- A final review-fix regression failed because duplicate JSON object keys could
  hide an earlier private payload behind a later valid aggregate value.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner benchmark_gate_rejects_private_real_report_with_payload_in_allowed_fields --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner benchmark_gate_rejects_private_real_report_with_duplicate_payload_keys --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner benchmark_gate_requires_private_real_corpus_metadata_for_release_evidence --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner resume_benchmark_gate_accepts_private_real_corpus_release_report --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
```

Output summary:

- Focused private real-corpus boundary and CLI release-gate tests passed after
  implementation.
- Full `benchmark-runner` tests passed: 9 CLI tests, 23 runner tests, and
  doc-tests.
- Focused clippy, fmt, diff check, and runbook policy passed.

Scope note:

- S155 validates only the redacted local aggregate report format used as release
  performance evidence. It does not run a real 100k/1M benchmark, clear the
  production P95 performance blocker, choose/distribute models, validate
  installers/services, sign/notarize artifacts, or upload a GitHub Release.

### S154

Design target:

- Stabilize encrypted full-text snapshot publication under hosted Windows
  transient file locks observed as `os error 33`.
- Keep the fix scoped to retrying transient local filesystem operations in the
  snapshot publish path; do not print snapshot paths, resume text, encrypted
  payloads, or local data directories.
- Preserve existing published-snapshot encryption, active snapshot fallback,
  and local witness behavior.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext transient_snapshot_fs_operation_retries_windows_lock_violation --locked
```

Output summary:

- The focused regression failed as intended because a simulated Windows
  `locked a portion of the file (os error 33)` operation was not retried.
- Hosted Windows PR #9 also repeatedly failed on the previous commit in
  `published_snapshot_encrypts_payload_at_rest` with `os error 33` from
  `publish_snapshot`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext transient_snapshot_fs_operation_retries_windows_lock_violation --locked
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_can_budget_documents_after_full_private_scan_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p import-pipeline -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN lock-violation test: exit 0 after implementation; 1 test
  passed.
- Full `index-fulltext` tests: exit 0; 5 unit tests, 13 integration tests,
  and doc-tests passed.
- Focused witness OCR budget regression: exit 0; 1 test passed.
- Full `s9_import_search` suite: exit 0; 25 tests passed locally.
- `cargo fmt --check`, focused clippy, `git diff --check`, public repo guard,
  and full local verification passed.

Scope note:

- S154 treats Windows full-text snapshot publish file-lock retries as a platform
  stability fix. It does not claim release readiness, real-data validation,
  service/installer validation, signing, notarization, OCR/model quality, or
  production-scale performance.

### S153

Design target:

- Extend deleted-document purge proof to current durable embedding job specs,
  not only vector snapshot documents and generic ingest job rows.
- Keep purge output aggregate and redacted: counts only, no model command
  paths, resume text, source paths, or local data paths.
- Preserve the existing local best-effort / not-forensic-erase boundary.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact
```

Output summary:

- The focused test failed as intended because purge output did not contain
  `embedding job specs purged: 1`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets --locked -- -D warnings
git diff --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN test: exit 0 after implementation; 1 test passed.
- Full `s14_delete_search` suite: exit 0; 7 tests passed.
- Full `meta-store` tests: exit 0; 46 integration tests, 1 identity test, and
  doc-tests passed.
- `cargo fmt --check`, focused clippy, `git diff --check`, public repo guard,
  and full local verification passed.

Scope note:

- S153 makes embedding job-spec cleanup visible and tested for the current
  `embedding_job_spec` queue metadata surface. It does not claim forensic
  erasure, deletion of user source files, real model availability, semantic
  quality, production-scale performance, release readiness, signing,
  notarization, or platform installer/service validation.

### S152

Design target:

- Extend deleted-document purge proof to the current OCR word-box/bbox evidence
  surface, not only plain OCR cache text.
- Keep purge output aggregate and redacted: counts only, no OCR text, word-box
  text, file paths, or local data paths.
- Preserve the existing local best-effort / not-forensic-erase boundary.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact
```

Output summary:

- The first edited test run failed to compile because the test used the wrong
  `succeeded_with_word_boxes` argument order; after fixing the test fixture, the
  focused test failed as intended because purge output did not contain
  `ocr word boxes purged: 1`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused GREEN test: exit 0 after implementation; 1 test passed.
- Full `s14_delete_search` suite: exit 0; 7 tests passed.
- Full `meta-store` tests: exit 0; 46 integration tests, 1 identity test, and
  doc-tests passed.
- `cargo fmt --check`, focused clippy, `git diff --check`, public repo guard,
  and full local verification passed.

Scope note:

- S152 makes OCR word-box purge visible and tested for the current
  `ocr_page_cache.word_boxes_json` surface. It does not claim forensic erasure,
  deletion of user source files, future unrelated PII surface coverage,
  OCR/model quality, production-scale performance, release readiness, signing,
  notarization, or platform installer/service validation.

### S151

Design target:

- Make the OCR page-cache privacy boundary explicit in doctor and redacted
  diagnostics.
- Prove the default OCR cache path stores OCR cache text in the SQLCipher
  metadata artifact rather than exposing cached OCR payload as plaintext in
  `metadata.sqlite3`.
- Keep this scoped to default local storage; do not claim OCR quality,
  distribution, full-library OCR, future bbox purge, or forensic erase.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_uses_sqlcipher_metadata_by_default_without_key_or_path_leak --locked -- --exact
```

Output summary:

- The focused test failed because doctor output did not contain
  `ocr cache encryption: sqlcipher`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_uses_sqlcipher_metadata_by_default_without_key_or_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_executes_local_command_persists_cache_and_indexes_searchable_text --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p embedder --test s11_embedder local_command_embedder_terminates_descendants_that_keep_output_pipes_open --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p embedder --test s11_embedder --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_ipc_connect_failure_does_not_fallback_to_sqlite --locked -- --exact --nocapture
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused diagnostics GREEN test: exit 0 after implementation; 1 test passed.
- Focused OCR worker cache-write proof: exit 0; 1 test passed and proved raw
  default `metadata.sqlite3` did not expose the synthetic OCR token or
  engine-profile marker.
- Full `s13_diagnostics` suite: exit 0; 14 tests passed.
- Full `s15_ocr_handoff` suite: exit 0; 13 tests passed.
- An initial full local verification run exposed a timing-sensitive
  `embedder` descendant-cleanup failure; exact and full `s11_embedder` reruns
  both passed, so no production change was made there.
- A later full local verification run exposed a real `s20_status_ipc`
  closed-port race under concurrent fake daemons; after the test helper was
  made deterministic, exact status-IPC connect-failure and full status-IPC
  suites passed.
- `cargo fmt --check`, focused clippy, `git diff --check`, public repo guard,
  and full local verification passed.

Scope note:

- S151 proves the current default OCR page cache inherits SQLCipher-at-rest
  protection from `MetaStore::open_data_dir` and exposes that state in redacted
  diagnostics, and it hardens a status-IPC closed-port regression test found
  during verification. It does not introduce a separate OCR cache artifact,
  prove forensic erasure of old disk pages, complete future bbox/PII purge
  coverage, distribute OCR engines or language packs, or clear remaining
  quality, large-corpus, model, packaging, signing, notarization, or real
  cross-platform blockers.

### S150

Design target:

- Encrypt published full-text snapshot artifacts at rest so
  `search-index/snapshots/<name>` no longer contains plaintext Tantivy files or
  stored resume text.
- Preserve current local behavior: publish, active open, fallback recovery,
  CLI search, daemon search IPC, diagnostics, import/search, and delete/purge
  flows must continue to work.
- Because the product is not shipped, do not keep plaintext published snapshot
  compatibility as a ready state; only the separate legacy root layout remains
  detectable for rebuild compatibility.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext published_snapshot_encrypts_payload_at_rest --locked -- --exact
```

Output summary:

- The focused test failed because `fulltext.snapshot.enc` did not exist under
  the published snapshot directory.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext published_snapshot_encrypts_payload_at_rest --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s8_search_cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused encrypted full-text snapshot test: exit 0 after implementation; 1
  test passed.
- Full `index-fulltext` tests: exit 0; 3 unit tests, 13 integration tests, and
  doc-tests passed.
- Related CLI/daemon import, search, diagnostics, delete, and search-IPC suites:
  exit 0.
- `cargo fmt --check`, focused clippy, `git diff --check`, public repo guard,
  and full local verification passed.

Scope note:

- S150 encrypts only published full-text snapshot artifacts at rest. It does
  not encrypt transient private decrypt directories while Tantivy is actively in
  use, does not add large-corpus latency evidence for decrypt-and-open, does not
  independently prove OCR-cache encryption beyond the default SQLCipher metadata
  path, and does not clear model, benchmark, installer, signing, notarization,
  or real cross-platform validation blockers.

### S149

Design target:

- Encrypt the persistent vector snapshot artifact at rest so `vector.snapshot`
  no longer stores vector IDs, document IDs, model IDs, or vector float payloads
  as plaintext.
- Preserve existing local behavior: reopen, inspection, HNSW ANN search,
  model-scoped semantic search, daemon embedding workers, and redacted
  diagnostics must continue to work.
- Because the product is not shipped, remove plaintext legacy snapshot support
  instead of keeping a compatibility path.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector persistent_vector_index_encrypts_snapshot_payload_at_rest --locked -- --exact
```

Output summary:

- The focused test failed because `vector.snapshot` still began with the old
  plaintext `resume-ir-vector-index-v2` header.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector persistent_vector_index_encrypts_snapshot_payload_at_rest --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_persistent_vector_snapshot_without_path_or_values --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-vector -p resume-daemon -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused encrypted vector snapshot test: exit 0 after implementation; 1 test
  passed.
- Full `index-vector` tests: exit 0; 10 integration tests and doc-tests passed.
- CLI embedding, vector diagnostics, daemon embedding worker, and daemon
  embedding job tests: exit 0.
- `cargo fmt --check`, focused clippy, `git diff --check`, public repo guard,
  and full local verification passed.

Scope note:

- S149 encrypts only the persistent vector snapshot artifact. Full-text
  snapshots, OCR-cache artifacts, real licensed embedding model
  selection/distribution, real semantic quality datasets, large-corpus ANN
  performance evidence, platform installer proof, signing, and notarization
  remain incomplete or BLOCKED.

### S148

Design target:

- Make the default `MetaStore::open_data_dir` path handle an existing plaintext
  `metadata.sqlite3` by migrating it to SQLCipher instead of failing after
  generating the local metadata key.
- Preserve existing metadata rows during migration and remove plaintext from the
  default metadata DB path after success.
- Keep migration errors redacted: no raw SQL, local paths, DB filenames, or
  synthetic payload text in user-facing error strings.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store open_data_dir_migrates_existing_plaintext_metadata_store_to_sqlcipher --locked -- --exact
```

Output summary:

- The focused test failed because `MetaStore::open_data_dir` returned
  `MetaStoreError { kind: Storage }` when a plaintext default metadata DB was
  already present.
- The first full local verification attempt exposed
  `daemon_serves_status_while_import_worker_processes_late_queued_task`
  failing under SQLCipher because repeated new IPC status connections could hit
  a transient encrypted WAL read error while the worker was polling/writing.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store open_data_dir_migrates_existing_plaintext_metadata_store_to_sqlcipher --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_serves_status_while_import_worker_processes_late_queued_task --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-daemon -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused plaintext metadata migration test: exit 0 after implementation; 1
  test passed.
- Full `meta-store` tests: exit 0; 46 integration tests and doc-tests passed.
- Full `resume-cli` tests: exit 0; all CLI tests passed.
- Focused daemon IPC regression and full `s20_ipc` suite: exit 0; the exact
  late-queued-task test passed and the full suite passed 18 tests.
- `cargo fmt --check`, focused clippy over `meta-store`, `resume-daemon`, and
  `resume-cli`, `git diff --check`, public repo guard, and full local
  verification passed.

Scope note:

- S148 migrates only the default metadata SQLite store from plaintext to
  SQLCipher. It does not encrypt full-text/vector/OCR-cache artifacts, prove
  forensic erasure, prove every mid-migration crash recovery window, run on real
  user stores, or complete the full product goal.

### S147

Design target:

- Add local metadata SQLCipher key rotation without weakening the S145 encrypted
  default data-dir path or the S146 backup/restore path.
- Use real SQLCipher rekeying: after rotation, the previous key must fail to
  open the metadata database and the new key must reopen the same schema.
- Keep CLI output, doctor output, errors, and Debug output free of key material,
  local data paths, key file paths, raw SQL, and metadata payloads.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s147_metadata_key_rotation_cli --locked
```

Output summary:

- The focused CLI test failed because `resume-cli privacy rotate-metadata-key`
  did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s147_metadata_key_rotation_cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s146_metadata_key_cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --locked
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused metadata key rotation CLI test: exit 0 after implementation; 1 test
  passed.
- Existing metadata key backup/restore CLI regression test: exit 0; 1 test
  passed.
- Full `meta-store` tests: exit 0; 45 integration tests and doc-tests passed.
- Full `resume-cli` tests: exit 0; all CLI tests passed.
- `cargo fmt --check`, focused clippy, `git diff --check`, public repo guard,
  and full local verification passed.

Scope note:

- S147 rotates only the local metadata SQLCipher key. It does not automatically
  sync backups, prove crash recovery for every mid-rotation failure window,
  encrypt full-text/vector/OCR-cache artifacts, prove old plaintext-store
  migration, prove forensic erasure, or complete the full product goal.

### S146

Design target:

- Add a local passphrase-protected backup/restore path for the metadata
  SQLCipher key created by the default data-dir metadata store.
- Keep the backup file, CLI output, errors, and Debug output free of key
  material, passphrases, local data paths, backup/passphrase file paths, raw SQL,
  and metadata payloads.
- Prove a copied encrypted metadata database can be reopened after restoring the
  key, while wrong passphrases and duplicate restores fail safely.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s146_metadata_key_cli --locked
```

Output summary:

- The focused CLI test failed because `resume-cli privacy` did not expose
  metadata key backup/restore subcommands.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s146_metadata_key_cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s146_metadata_key_cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s142_privacy_key_cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused metadata key CLI test: exit 0; 1 test passed.
- Focused metadata key CLI test with `--locked`: exit 0; 1 test passed.
- Existing contact key CLI regression test: exit 0; 1 test passed.
- Full `meta-store` tests: exit 0; 45 integration tests and doc-tests passed.
- Full `resume-cli` tests: exit 0; all CLI tests passed.
- `cargo fmt --check`, focused clippy, `git diff --check`, public repo guard,
  and full local verification passed.

Scope note:

- S146 backs up and restores only the local metadata SQLCipher key. It does not
  rotate metadata keys, automatically sync backups, encrypt full-text/vector/
  OCR-cache artifacts, prove old plaintext-store migration, prove forensic
  erasure, or complete the full product goal.

### S145

Design target:

- Move the default CLI and daemon metadata data-dir path from plaintext SQLite
  to SQLCipher-backed storage.
- Generate and reuse a local metadata SQLCipher key without printing paths,
  key material, raw SQL, or document content.
- Keep metadata-key availability independent from contact-hash-key diagnostics,
  so a broken contact key can still be reported by `doctor`.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli doctor_uses_sqlcipher_metadata_by_default_without_key_or_path_leak --locked -- --exact
```

Output summary:

- The focused test failed because `doctor` still reported `metadata encryption:
  plaintext` instead of `sqlcipher`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli doctor_uses_sqlcipher_metadata_by_default_without_key_or_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli doctor_uses_sqlcipher_metadata_by_default_without_key_or_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused default metadata encryption test: exit 0 after Cargo updated the
  lockfile for the new `meta-store` direct `getrandom` dependency.
- Focused default metadata encryption test with `--locked`: exit 0.
- Full diagnostics suite: exit 0; 14 tests passed.
- Full `resume-cli` tests: exit 0; all CLI tests passed.
- Full `resume-daemon` tests: exit 0; all daemon tests passed.
- Full `meta-store` tests: exit 0; 45 integration tests and doc-tests passed.
- `cargo fmt --check`, focused clippy, `git diff --check`, public repo guard,
  and full local verification passed.

Scope note:

- S145 encrypts default metadata persistence only. It does not back up or rotate
  metadata SQLCipher keys, encrypt full-text/vector/OCR-cache artifacts, prove
  forensic erasure, validate old plaintext-store migration, or complete the
  full product goal.

### S144

Design target:

- Add a real SQLCipher-backed metadata store open path rather than only
  diagnostic plaintext visibility.
- Prove with a synthetic database that the encrypted metadata file is not a
  readable plaintext SQLite file, wrong keys fail, and the correct key survives
  reopen and can read persisted rows.
- Keep errors redacted: no SQL statements, local paths, synthetic document path
  fields, or database filenames in `Display`/`Debug`.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store encrypted_metadata_store_requires_key_and_survives_reopen_without_plaintext_header --locked -- --exact
```

Output summary:

- The focused test failed because `MetaStore::open_encrypted` did not exist and
  `MetadataEncryptionState::SqlCipher` was not defined.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store encrypted_metadata_store_requires_key_and_survives_reopen_without_plaintext_header -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store encrypted_metadata_store_requires_key_and_survives_reopen_without_plaintext_header --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused encrypted metadata-store test: exit 0; 1 test passed after Cargo
  updated the lockfile with `openssl-src` and `openssl-sys`.
- Focused encrypted metadata-store test with `--locked`: exit 0; 1 test passed.
- Full `meta-store` tests: exit 0; 45 integration tests and doc-tests passed.
- Full diagnostics CLI suite: exit 0; 13 tests passed.
- `cargo fmt --check`, focused clippy, `git diff --check`, public repo guard,
  and full local verification passed.

Scope note:

- S144 proves an actual SQLCipher metadata-store open path. It does not migrate
  default CLI/daemon data directories to encrypted metadata, generate/store the
  metadata key, migrate existing plaintext databases, rotate metadata keys, or
  prove forensic erasure.

### S143

Design target:

- Replace the S142 plaintext contact-key backup envelope before shipment with a
  passphrase-protected local backup file.
- Require backup and restore callers to provide passphrase bytes; CLI callers
  must use a local `--passphrase-file` instead of passing the passphrase as a
  command-line argument.
- Keep backup files, stdout/stderr, and Debug output free of key material,
  passphrase material, contacts, local data paths, and backup/passphrase file
  paths.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p privacy contact_hash_key_backup_and_restore_round_trip_without_leaking_key_or_contacts --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s142_privacy_key_cli --locked
```

Output summary:

- The focused privacy test failed first because `backup_contact_hash_key` and
  `restore_contact_hash_key` accepted only data/backup paths, not passphrase
  bytes.
- The focused CLI test failed first because `resume-cli privacy
  backup-contact-key` rejected `--passphrase-file`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p privacy contact_hash_key_backup_and_restore_round_trip_without_leaking_key_or_contacts -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p privacy contact_hash_key_restore_with_wrong_passphrase_refuses_without_creating_key --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s142_privacy_key_cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p privacy --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p privacy -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused encrypted backup/restore round-trip test: exit 0; 1 test passed.
- Focused wrong-passphrase restore test: exit 0; 1 test passed.
- Focused privacy CLI test: exit 0; 1 test passed.
- Full `privacy` tests: exit 0; 7 integration tests and doc-tests passed.
- `cargo fmt --check`, focused clippy, `git diff --check`, public repo guard,
  and full local verification passed.

Scope note:

- S143 protects contact hash key backup files only. It does not rotate contact
  keys, encrypt SQLite metadata, prove forensic erasure, or clear the
  SQLCipher/encrypted local storage blocker.

### S142

Design target:

- Add a local-only backup/recovery path for the contact HMAC key so a restored
  data directory can keep deterministic contact hashes across reinstall or data
  migration.
- Keep all command, Debug, and test output redacted: no local data paths,
  backup paths, key material, contact values, resume text, or diagnostics.
- Refuse restore into a data directory that already has a contact hash key so a
  recovery operation cannot silently break existing contact-hash identity.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p privacy contact_hash_key_backup_and_restore_round_trip_without_leaking_key_or_contacts --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s142_privacy_key_cli --locked
```

Output summary:

- The focused privacy test first failed because
  `backup_contact_hash_key` and `restore_contact_hash_key` did not exist.
- The focused CLI test first failed because `resume-cli privacy` did not have
  backup/restore subcommands.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p privacy contact_hash_key_backup_and_restore_round_trip_without_leaking_key_or_contacts --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p privacy contact_hash_key_restore_refuses_to_overwrite_existing_key --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s142_privacy_key_cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p privacy --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p privacy -p resume-cli --all-targets --locked -- -D warnings
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused contact-key backup/restore round-trip test: exit 0; 1 test passed.
- Focused existing-key restore refusal test: exit 0; 1 test passed.
- Focused privacy CLI test: exit 0; 1 test passed.
- Full `privacy` tests: exit 0; 6 integration tests and doc-tests passed.
- `cargo fmt --check`, `git diff --check`, focused clippy, public repo guard,
  and full local verification passed.

Scope note:

- S142 completes contact hash key backup/recovery only. It does not rotate
  contact keys, encrypt SQLite metadata, passphrase-protect backup files, prove
  forensic erasure, or clear the SQLCipher/encrypted local storage blocker.

### S141

Hosted RED:

```bash
gh pr checks 9
gh run view 26953047980 --job 79522910616 --log
```

Output summary:

- PR #9 `rust workspace` failed on Ubuntu.
- Failure: `local_pdf_render_command_returns_page_bytes_without_payload_debug_leaks`
  returned `OcrError { kind: EngineFailed }`.

Linux reproduction RED:

```bash
docker run --rm -v "$PWD":/work -w /work rust:1.96-bookworm bash -lc 'export PATH=/usr/local/cargo/bin:$PATH CARGO_TARGET_DIR=/tmp/resume-ir-target; cargo test -p ocr-client --test s12_ocr_client --locked -- --nocapture'
```

Output summary:

- The full `s12_ocr_client` test file reproduced the same class of flake in a
  Linux container: timeout/descendant tests sometimes returned `EngineFailed`
  instead of `Timeout`.
- The focused `local_pdf_render_command_returns_page_bytes_without_payload_debug_leaks`
  test passed when run alone, pointing to parallel command-process interaction
  rather than renderer semantics.

Root cause:

- Normal child-exit paths immediately signaled the old child process group before
  first checking whether stdout/stderr readers had naturally drained.
- Under Linux parallel test execution, the old PGID can be reused quickly by
  another fixture process group; the cleanup signal can therefore kill an
  unrelated in-flight OCR command and turn success or timeout expectations into
  `EngineFailed`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked -- --nocapture
/Users/frankqdwang/.cargo/bin/cargo fmt
/Users/frankqdwang/.cargo/bin/cargo fmt --check
docker run --rm -v "$PWD":/work -w /work rust:1.96-bookworm bash -lc 'export PATH=/usr/local/cargo/bin:$PATH CARGO_TARGET_DIR=/tmp/resume-ir-target; for i in 1 2 3 4 5 6 7 8 9 10; do echo RUN=$i; cargo test -p ocr-client --test s12_ocr_client --locked -- --nocapture || exit 1; done'
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- macOS `s12_ocr_client`: exit 0; 17 tests passed.
- `cargo fmt --check`: exit 0.
- Linux container `s12_ocr_client` loop: exit 0; 10 consecutive full-file runs
  passed, 17 tests per run.
- Full `ocr-client` tests: exit 0; 17 integration tests and doc-tests passed.
- Focused `ocr-client` clippy, `git diff --check`, and public repo guard:
  exit 0.
- Full local verification: exit 0; workspace tests, doc-tests, license check,
  runbook check, workflow check, release artifact check, release SBOM check,
  macOS package check, and public repo guard passed.

Scope note:

- S141 fixes OCR command process cleanup robustness and hosted CI flake only. It
  does not change OCR quality, language-pack packaging, renderer selection, or
  metadata encryption.

### S140

Design target:

- Make the current plaintext metadata-storage state visible in doctor and
  redacted diagnostics.
- Avoid implying that the local SQLite metadata/task store is encrypted before
  SQLCipher or equivalent application-level encryption is implemented.
- Keep the diagnostic redacted: no local data directory paths, keys, raw resume
  text, or private diagnostics.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_reports_no_index_without_path_or_fake_benchmark --locked -- --exact
```

Output summary:

- The focused doctor test failed because output did not contain
  `metadata encryption: plaintext`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_reports_no_index_without_path_or_fake_benchmark --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store metadata_encryption_state_reports_plaintext_until_sqlcipher_is_enabled --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused doctor metadata-encryption diagnostic test: exit 0; 1 test passed.
- Focused redacted export metadata-encryption diagnostic test: exit 0; 1 test
  passed.
- Focused meta-store encryption-state test: exit 0; 1 test passed.
- Full `meta-store` tests: exit 0; 44 tests passed.
- Full diagnostics suite: exit 0; 13 tests passed.
- `cargo fmt --check`, focused clippy, and `git diff --check`: exit 0.
- Public repo guard: exit 0; `public repo guard passed`.
- Full local verification: exit 0; workspace tests, doc-tests, license check,
  runbook check, workflow check, release artifact check, release SBOM check,
  macOS package check, and public repo guard passed.

Scope note:

- S140 completes diagnostic visibility for the current plaintext metadata-store
  state only. It does not implement SQLCipher, encrypt SQLite, rotate or back
  up encryption keys, prove forensic erasure, or clear the encrypted local
  storage blocker.

### S139

Design target:

- Surface missing OCR language-pack blockers in redacted operational status,
  not only as a generic retryable OCR job.
- Count current retryable OCR jobs that have a linked `LanguageUnavailable`
  OCR page-cache failure.
- Keep status, doctor, diagnostics, daemon status IPC, and CLI status-over-IPC
  free of requested language names, runtime paths, private document paths, local
  language-list dumps, or OCR engine stderr.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_blocks_missing_tesseract_language_before_invoking_engine_without_leaks --locked -- --exact
```

Output summary:

- The focused test failed because `status` did not contain
  `ocr language unavailable: 1` after the OCR worker recorded a
  `LanguageUnavailable` retryable cache failure.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_blocks_missing_tesseract_language_before_invoking_engine_without_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_can_read_redacted_daemon_status_over_loopback_ipc --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_serves_redacted_status_over_loopback_ipc --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store status_summary_aggregates_documents_jobs_imports_and_index_state --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
```

Output summary:

- Focused missing-language worker visibility test: exit 0; 1 test passed.
- Focused CLI status-over-IPC test: exit 0; 1 test passed.
- Focused daemon status IPC test: exit 0; 1 test passed.
- Focused meta-store status summary test: exit 0; 1 test passed.
- Full `meta-store` tests: exit 0; 43 tests passed.
- Full CLI OCR handoff suite: exit 0; 13 tests passed.
- Full CLI status IPC suite: exit 0; 6 tests passed.
- Full daemon status IPC suite: exit 0; 18 tests passed.
- `cargo fmt --check`, focused clippy, and `git diff --check`: exit 0.

Scope note:

- S139 completes redacted operational visibility for missing requested OCR
  language packs. It does not install or distribute language packs, prove
  non-English OCR quality, complete full-library OCR, validate installed
  Windows/macOS OCR runtime behavior, or clear OCR quality and packaging
  blockers.

### S138

Design target:

- Block OCR workers before renderer or OCR invocation when the configured
  Tesseract runtime is missing any requested language pack, including combined
  requests such as `eng+chi_sim`.
- Persist a retryable OCR page-cache failure with `LanguageUnavailable` so the
  job can be retried after local runtime remediation.
- Keep CLI, daemon, cache, and status output redacted: no command paths,
  requested language names, local language-list dumps, private document paths,
  or OCR engine stderr.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_blocks_missing_tesseract_language_before_invoking_engine_without_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_blocks_missing_tesseract_language_before_engine_without_leaks --locked -- --exact
```

Output summary:

- The focused CLI test first failed because stderr did not contain the new
  language-unavailable worker block message.
- The focused daemon test first failed because the OCR page-cache failure was
  still recorded as `EngineFailed` instead of `LanguageUnavailable`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_blocks_missing_tesseract_language_before_invoking_engine_without_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_blocks_missing_tesseract_language_before_engine_without_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
```

Output summary:

- `cargo fmt --check`: exit 0.
- Focused CLI missing-language worker test: exit 0; 1 test passed.
- Focused daemon missing-language worker test: exit 0; 1 test passed.
- `ocr-client` tests: exit 0; 17 tests passed.
- Focused clippy for `ocr-client`, `resume-cli`, and `resume-daemon`: exit 0.
- Full CLI OCR handoff suite: exit 0; 13 tests passed.
- Full daemon OCR worker suite: exit 0; 9 tests passed.

Scope note:

- S138 completes a redacted worker preflight/failure-classification path for
  missing requested Tesseract language packs. It does not distribute language
  packs, prove non-English OCR accuracy, complete full-library OCR, validate
  installed Windows/macOS OCR runtime behavior, or clear OCR quality and release
  packaging blockers.

### S137

Design target:

- Support production-style Tesseract combined language requests such as
  `eng+chi_sim` in doctor and redacted diagnostics.
- Treat the combined request as available only when every requested component is
  present, without dumping unrelated local language-pack names.
- Verify that the local machine can now report the combined English/Simplified
  Chinese OCR language profile as available using an Apache-2.0 language-pack
  dependency.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_check_combined_ocr_languages_without_language_dump --locked -- --exact
```

Output summary:

- The focused test failed because `doctor --ocr-lang eng+chi_sim` did not report
  `ocr language eng+chi_sim: available` when the fake local Tesseract runtime
  exposed separate `eng` and `chi_sim` language packs.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_check_combined_ocr_languages_without_language_dump --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings
brew info --json=v2 tesseract-lang
HOMEBREW_NO_AUTO_UPDATE=1 brew install tesseract-lang
cargo run --quiet -p resume-cli -- doctor --ocr-lang eng+chi_sim
tesseract --list-langs
brew list --versions tesseract-lang
```

Output summary:

- Focused combined-language diagnostics test: exit 0; 1 test passed.
- `cargo fmt --check`: exit 0 after formatting.
- Full diagnostics suite: exit 0; 13 tests passed.
- Focused CLI clippy: exit 0.
- Homebrew metadata reported `tesseract-lang` license `Apache-2.0`; local
  install completed for `tesseract-lang 4.1.0`.
- Local Tesseract language listing included both `eng` and `chi_sim`.
- Local `doctor --ocr-lang eng+chi_sim` reported OCR renderer, OCR engine, and
  combined OCR language availability without printing binary paths or dumping
  the full language list.

Scope note:

- S137 completes combined-language availability diagnostics and local
  `eng+chi_sim` runtime availability only. It does not distribute language packs
  in installers, prove OCR accuracy on Chinese resumes, complete full-library
  OCR, validate Windows/macOS installed runtime behavior, or clear broader OCR
  quality and release-readiness blockers.

### S136

Design target:

- Confirm the latest PR #9 branch tip is green after the hosted Release dry-run
  evidence update.
- Rerun the explicit-root PDF/Word witness against the user-authorized local
  sample root without committing or uploading paths, filenames, raw content,
  sample counts, or diagnostics.
- Use local OCR only as a bounded chain witness, not as a claim of full private
  corpus OCR completion or OCR quality.

Implementation checks:

```bash
gh pr checks 9
cargo run --quiet -p resume-cli -- witness --root <authorized-local-sample-root> --max-files 10000 --probe-search --probe-fields --run-ocr --ocr-tesseract-command /opt/homebrew/bin/tesseract --ocr-pdftoppm-command /opt/homebrew/bin/pdftoppm --ocr-lang eng --ocr-max-documents 2 --ocr-max-pages-per-document 1
```

Output summary:

- PR #9 checks passed on commit `22e1adc`: dependency tree, license policy,
  public repository guard, runbook policy, Rust workspace, hosted macOS Platform
  CI, and hosted Windows Platform CI. Sourcery review remained skipped.
- The private witness ran with `source root: <redacted>`, explicit scan profile,
  and PDF/DOCX/DOC support only.
- The witness completed import, the redacted field probe, and the redacted
  search probe, without printing private search queries, field values,
  filenames, or paths.
- The bounded OCR witness used local `tesseract` and `pdftoppm`, exercised the
  configured OCR document budget under the local English-only OCR language
  configuration, and left the remaining OCR queue budgeted.
- The witness reported `private witness data: removed`.

Scope note:

- S136 is a local-only private sample witness refresh and hosted PR-check status
  record. It does not prove full private corpus OCR completion, OCR quality,
  non-English OCR quality, production recall/precision, large-corpus latency/
  throughput, packaging/signing/installers, Windows/Linux real sample behavior,
  or production model/ANN readiness.

### S135

Design target:

- Prove the latest Release workflow wiring on hosted GitHub runners after the
  WiX pin and macOS DMG retry fixes were pushed.
- Confirm the Ubuntu manifest/SBOM dry-run, hosted macOS pkg/dmg dry-run, and
  hosted Windows MSI dry-run all complete in one workflow run.
- Record only public workflow metadata and artifact names/sizes, without
  downloading artifacts or exposing local/private runtime data.

Implementation checks:

```bash
gh pr checks 9 --watch --interval 10
gh workflow run Release --ref codex/fault-injection-diagnostics -f version=v0.0.0
gh run watch 26945622774 --exit-status --interval 10
gh run view 26945622774 --json status,conclusion,workflowName,event,headBranch,headSha,url,jobs
gh api repos/FrankQDWang/resume-ir/actions/runs/26945622774/artifacts --jq '.artifacts[] | {name: .name, expired: .expired, size_in_bytes: .size_in_bytes, created_at: .created_at, expires_at: .expires_at}'
```

Output summary:

- PR #9 checks passed on commit `13f35a7`: dependency tree, license policy,
  public repository guard, runbook policy, Rust workspace, hosted macOS Platform
  CI, and hosted Windows Platform CI. Sourcery review remained skipped.
- Release workflow run `26945622774`: conclusion `success`, event
  `workflow_dispatch`, branch `codex/fault-injection-diagnostics`, head SHA
  `13f35a70d525486dc3ee5a72f21024a66b18718e`.
- `release dry run` job `79497737161`: exit 0; completed in 1m26s; workspace
  verification, release binary build, artifact manifest creation, SBOM creation,
  public artifact boundary check, artifact upload, and release gate all passed.
- `macOS package dry run` job `79497737062`: exit 0; completed in 2m42s;
  release binary build, unsigned macOS package dry-run creation, DMG boundary
  verification through the retry helper, artifact upload, and release gate all
  passed.
- `Windows package dry run` job `79497737003`: exit 0; completed in 5m24s;
  WiX `6.0.2` installation, release binary build, unsigned MSI dry-run
  creation, Windows package boundary check, artifact upload, and release gate
  all passed.
- Artifact metadata listed three non-expired artifacts: `release-dry-run`,
  `macos-package-dry-run`, and `windows-package-dry-run`.

Scope note:

- S135 proves hosted dry-run execution and workflow artifact publication only.
  It does not sign or notarize artifacts, create/upload a GitHub Release,
  validate install/upgrade/uninstall/rollback behavior, prove Gatekeeper
  behavior, install/register/start/stop a Windows service, or complete
  production release readiness.

### S134

Design target:

- Preserve `hdiutil verify` DMG checksum validation.
- Add bounded retry for hosted macOS transient disk-image availability after DMG
  creation.
- Reuse the same verification helper in Release workflow and local macOS package
  guard.

Observed hosted RED:

```bash
gh run watch 26944923353 --exit-status --interval 10
gh run view 26944923353 --job 79495400267 --log
```

Output summary:

- Release run `26944923353` passed `release dry run` in 1m41s.
- `Windows package dry run` passed in 5m56s after installing WiX `6.0.2`,
  building release binaries, creating the unsigned MSI dry-run, checking the
  Windows package boundary, uploading `windows-package-dry-run`, and reporting
  release gates.
- `macOS package dry run` failed in 3m18s at `Check macOS package boundary`
  because `hdiutil verify` reported the generated DMG as temporarily
  unavailable immediately after creation.

Implementation checks:

```bash
./scripts/ci/check-workflows.sh
./scripts/ci/check-macos-package.sh
ruby -e 'require "yaml"; Dir[".github/workflows/*.yml"].sort.each { |f| YAML.load_file(f); puts f }'
sh -n scripts/release/verify-macos-dmg.sh scripts/ci/check-macos-package.sh scripts/ci/check-workflows.sh
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Workflow guard: exit 0; Release workflow now calls
  `scripts/release/verify-macos-dmg.sh`, and that helper still contains
  `hdiutil verify`.
- macOS package guard: exit 0; generated a synthetic unsigned pkg/dmg dry-run,
  verified the DMG through the retry helper, validated the manifest, and kept
  invalid-version and missing-binary rejection checks.
- Workflow YAML parse: exit 0 for all tracked workflow files.
- Shell syntax checks: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, release SBOM check, macOS package check through the retry
  helper, Windows package wiring check, and public repository guard passed.

Scope note:

- S134 is a hosted macOS DMG verification retry fix and records that
  `26944923353` already proved the Windows MSI dry-run path after the WiX pin.
  It does not prove the combined hosted Release workflow until this fix is
  pushed and Release is rerun, and it does not sign artifacts, create/upload a
  GitHub Release, validate install/upgrade/uninstall/rollback behavior, install/
  register/start/stop a Windows service, or complete release readiness.

### S133

Design target:

- Do not accept WiX `7.0.0` OSMF EULA or fee terms in CI.
- Keep the hosted unsigned MSI dry-run path, but pin the WiX .NET tool to a
  version that can run unattended for this public dry-run workflow.
- Strengthen the workflow guard so future changes must keep an explicit WiX
  version pin.

Observed hosted RED:

```bash
gh run watch 26944149485 --exit-status --interval 10
gh run view 26944149485 --job 79492784081 --log
```

Output summary:

- Release run `26944149485` passed `release dry run` in 1m25s and
  `macOS package dry run` in 2m58s.
- `Windows package dry run` failed in 6m0s at `Create unsigned Windows MSI dry
  run`.
- The Windows job built release binaries successfully, then `wix build` failed
  with `WIX7015` because WiX Toolset v7 required OSMF EULA acceptance.

Implementation checks:

```bash
./scripts/ci/check-workflows.sh
./scripts/ci/check-runbooks.sh
./scripts/ci/check-windows-package.sh
ruby -e 'require "yaml"; Dir[".github/workflows/*.yml"].sort.each { |f| YAML.load_file(f); puts f }'
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Workflow guard: exit 0; Release workflow now requires
  `dotnet tool install --global wix --version 6.0.2`.
- Runbook guard: exit 0.
- Windows package guard: exit 0 on this macOS host by validating wiring, then
  explicitly skipping actual MSI creation on non-Windows.
- Workflow YAML parse: exit 0 for all tracked workflow files.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, release SBOM check, macOS package check, Windows package
  wiring check, and public repository guard passed.

Scope note:

- S133 is a toolchain pin and hosted Release failure response only. It does not
  prove the hosted Windows MSI build until the branch is pushed and Release is
  rerun, and it does not sign artifacts, create/upload a GitHub Release, validate
  install/upgrade/uninstall/rollback behavior, install/register/start/stop a
  Windows service, or complete release readiness.

### S132

Design target:

- Preserve the daemon IPC behavior and assertions while giving the hosted
  Windows runner enough polling budget for the late-queued import worker to
  complete.
- Keep the daemon `--max-requests` drain path covered so the child process still
  exits normally after the test.

Observed hosted RED:

```bash
gh pr checks 9 --watch
```

Output summary:

- PR #9 check run for commit `cd26a04` failed only in hosted Windows Platform CI.
- The failing command was `cargo test --workspace --locked`, with the concrete
  failing test `crates\daemon\tests\s20_ipc.rs:967:5`
  `daemon_serves_status_while_import_worker_processes_late_queued_task`.
- Failure message: `daemon status did not report searchable document count 2`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_serves_status_while_import_worker_processes_late_queued_task --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc --locked
git diff --check
./scripts/ci/guard-public-repo.sh
PATH=/Users/frankqdwang/.cargo/bin:$PATH ./scripts/ci/verify-local.sh
```

Output summary:

- Focused exact test: exit 0; 1 passed.
- Full daemon IPC suite: exit 0; 18 passed.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, release SBOM check, macOS package check, Windows package
  wiring check, and public repository guard passed.

Scope note:

- S132 is a hosted Windows CI wait-budget fix for an existing daemon IPC test
  exposed after S131 was pushed. It does not change production daemon behavior,
  prove Windows MSI creation, sign artifacts, create/upload a GitHub Release,
  validate install/upgrade/uninstall/rollback behavior, install/register/start/
  stop a Windows service, or complete release readiness.

### S131

Design target:

- Add a Windows-only unsigned MSI dry-run script for already-built release
  binaries using the WiX .NET tool.
- Wire local non-Windows guard checks into `verify-local` without pretending
  this macOS host can build an MSI.
- Add a hosted `windows-latest` Release workflow job that installs WiX, builds
  release binaries, creates the MSI dry-run, checks redacted artifact
  boundaries, and uploads `windows-package-dry-run`.

Observed RED:

```bash
./scripts/ci/check-workflows.sh
```

Output summary:

- The workflow guard failed because `scripts/ci/verify-local.sh` did not include
  `./scripts/ci/check-windows-package.sh`, and the Release workflow had no
  Windows package dry-run wiring.

Implementation checks:

```bash
./scripts/ci/check-workflows.sh
./scripts/ci/check-windows-package.sh
./scripts/ci/check-runbooks.sh
ruby -e 'require "yaml"; Dir[".github/workflows/*.yml"].sort.each { |f| YAML.load_file(f); puts f }'
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- Workflow guard: exit 0; Release workflow now requires
  `create-windows-package.ps1`, `windows-package.json`,
  `windows-package-dry-run`, `windows-latest`, WiX installation, and the Windows
  MSI artifact name.
- Windows package guard: exit 0 on this macOS host by validating script,
  workflow, guard, and runbook wiring, then explicitly skipping actual MSI
  creation on non-Windows.
- Runbook guard: exit 0.
- Workflow YAML parse: exit 0 for all tracked workflow files.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, release SBOM check, macOS package check, Windows package
  wiring check, and public repository guard passed.

Scope note:

- S131 is Windows MSI dry-run wiring and local guard evidence only. It does not
  prove the hosted Windows MSI build until the branch is pushed and Release is
  rerun, and it does not sign artifacts, create/upload a GitHub Release, validate
  install/upgrade/uninstall/rollback behavior, install/register/start/stop a
  Windows service, or complete release readiness.

### S130

Design target:

- Prove the S129 Release workflow wiring on hosted GitHub runners after pushing
  the branch.
- Confirm the existing Ubuntu release dry-run and the new hosted macOS package
  dry-run both complete on the same commit.
- Record only public workflow metadata and artifact names, without downloading
  artifacts or exposing local/private runtime data.

Implementation checks:

```bash
gh pr checks 9 --watch
gh workflow run Release --ref codex/fault-injection-diagnostics -f version=v0.0.0
gh run watch 26942549866 --exit-status
gh run view 26942549866 --json status,conclusion,workflowName,event,headBranch,headSha,url,jobs
gh api repos/FrankQDWang/resume-ir/actions/runs/26942549866/artifacts
gh run view 26942549866 --job 79487339237 --log
gh run view 26942549866 --job 79487339378 --log
```

Output summary:

- PR #9 hosted checks passed for commit `a7dc1c0`: dependency tree, license
  policy, public repository guard, runbook policy, Rust workspace, hosted macOS
  Platform CI, and hosted Windows Platform CI. Sourcery review remained
  skipped.
- Release workflow run `26942549866` completed with conclusion `success` on
  commit `a7dc1c0f19305d58672e1c8bc67e13f4c39e06c8`.
- Release job results: `macOS package dry run` job `79487339237` passed in
  3m1s; `release dry run` job `79487339378` passed in 1m30s.
- Artifact listing showed non-expired `macos-package-dry-run` and
  `release-dry-run` artifacts.
- Log scan across both Release jobs found no `Node.js 20 actions are
  deprecated` warning and no `actions/checkout@v4` or
  `actions/upload-artifact@v4` references.
- The hosted macOS package boundary step confirmed `hdiutil` checksum
  verification for the generated dmg and the manifest statuses
  `unsigned_dry_run`, `unsigned`, and `not_requested`.

Scope note:

- S130 proves hosted dry-run execution and artifact publication only. It does
  not sign or notarize artifacts, create/upload a GitHub Release, validate
  install/upgrade/uninstall/rollback behavior, prove Gatekeeper behavior, build
  Windows MSI, or complete production release readiness.

### S129

Design target:

- Add a hosted macOS Release workflow dry-run job that builds the workspace
  release binaries and runs the existing unsigned pkg/dmg package dry-run.
- Verify the generated dmg on the hosted macOS runner with `hdiutil`, keep the
  artifact boundary redacted, and upload only dry-run package outputs.
- Preserve explicit release gates for signing, notarization, installer
  lifecycle validation, Windows MSI, and GitHub Release upload.

Observed RED:

```bash
./scripts/ci/check-workflows.sh
```

Output summary:

- The workflow guard failed because `.github/workflows/release.yml` did not
  include `scripts/release/create-macos-package.sh` or the hosted macOS package
  dry-run evidence path.

Implementation checks:

```bash
./scripts/ci/check-workflows.sh
ruby -e 'require "yaml"; Dir[".github/workflows/*.yml"].sort.each { |f| YAML.load_file(f); puts f }'
./scripts/ci/check-release-artifacts.sh
./scripts/ci/check-release-sbom.sh
./scripts/ci/check-macos-package.sh
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- Workflow guard: exit 0; Release workflow now requires the macOS package
  dry-run script, `macos-package.json`, `macos-package-dry-run`,
  `macos-latest`, `hdiutil verify`, and current checkout/upload action majors.
- Workflow YAML parse: exit 0 for all tracked workflow files.
- Release artifact guard: exit 0.
- Release SBOM guard: exit 0.
- macOS package guard: exit 0; generated and validated synthetic unsigned
  pkg/dmg dry-run artifacts and redacted package metadata.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, release SBOM check, macOS package check, and public repository
  guard passed.

Scope note:

- S129 wires hosted macOS package dry-run execution into Release workflow only.
  It does not prove the hosted run until the branch is pushed and rerun, and it
  does not sign or notarize artifacts, create/upload a GitHub Release, validate
  install/upgrade/uninstall/rollback behavior, prove Gatekeeper behavior, build
  Windows MSI, or complete release readiness.

### S128

Design target:

- Add local macOS-only unsigned pkg/dmg dry-run packaging for already-built
  release binaries.
- Generate a redacted `macos-package.json` manifest with artifact filenames,
  byte counts, hashes, unsigned status, and still-blocked release steps only.
- Wire a guard into `verify-local` so the macOS dry-run does not regress, while
  skipping the guard on non-macOS rather than pretending Ubuntu can build macOS
  installers.

Observed RED:

```bash
./scripts/ci/check-macos-package.sh
```

Output summary:

- The focused macOS package guard failed because
  `scripts/ci/check-macos-package.sh` did not exist.

Implementation checks:

```bash
./scripts/ci/check-macos-package.sh
./scripts/ci/check-workflows.sh
./scripts/ci/check-release-artifacts.sh
./scripts/ci/check-release-sbom.sh
sh -n scripts/release/create-macos-package.sh scripts/ci/check-macos-package.sh scripts/ci/check-workflows.sh scripts/ci/verify-local.sh
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- macOS package guard: exit 0; generated unsigned synthetic pkg/dmg dry-run
  artifacts; expanded the pkg; verified the dmg checksum; verified the redacted
  `macos-package.json`; rejected invalid version input; rejected missing
  required binaries; and confirmed the guard remains wired into `verify-local`.
- Workflow guard: exit 0; `verify-local` now requires the macOS package guard.
- Release artifact guard: exit 0.
- Release SBOM guard: exit 0.
- Shell syntax checks: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, release SBOM check, macOS package check, and public repository
  guard passed.

Scope note:

- S128 is unsigned local macOS package dry-run evidence only. It does not sign
  or notarize artifacts, create/upload a GitHub Release, validate install/
  upgrade/uninstall/rollback behavior, prove Gatekeeper behavior, build Windows
  MSI, or complete release readiness.

### S127

Design target:

- Use the user-authorized local resume sample directory for a private local-only
  PDF/Word witness.
- Ignore other sample formats through the existing witness supported-extension
  filter.
- Verify import, field extraction, search probing, and bounded OCR with local
  OCR tools, while keeping paths, filenames, raw text, sample counts, and
  diagnostics out of git and remote services.

Private local-only checks:

```bash
./target/debug/resume-cli witness --root <private-local-root> --probe-search --probe-fields
./target/debug/resume-cli witness --root <private-local-root> --max-files <bounded> --probe-search --probe-fields --run-ocr --ocr-max-documents <bounded> --ocr-tesseract-command <local-tesseract> --ocr-pdftoppm-command <local-pdftoppm>
```

Output summary:

- Import/search/field witness: exit 0; source root was redacted; only
  PDF/DOCX/DOC inputs were selected by the witness path; import completed;
  field probe completed; search probe completed; private witness data was
  removed.
- Bounded OCR witness: exit 0; local `tesseract` and `pdftoppm` were used; OCR
  status completed for the configured document budget; remaining OCR work stayed
  budgeted instead of being reported as full-library completion; no OCR failures
  were reported for the budgeted run; private witness data was removed.

Scope note:

- S127 is private local-only witness evidence. No real resume data, filenames,
  paths, sample counts, raw text, or diagnostics were committed or uploaded. It
  does not prove full-library OCR completion, OCR quality, non-English OCR
  quality, large-corpus latency/throughput, Windows/Linux private sample
  behavior, or release/installer readiness.

### S126

Design target:

- Re-run the hosted Release workflow after S125 was pushed so the updated
  checkout/artifact action versions are actually exercised remotely.
- Verify the dry-run release artifact exists without downloading it.
- Verify the release job log no longer contains the Node.js 20 action
  deprecation warning or v4 checkout/artifact upload references.
- Wait for PR #9 hosted checks for the pushed commit.

Hosted release checks:

```bash
gh workflow run Release --ref codex/fault-injection-diagnostics -f version=v0.0.0
gh run watch 26940230718 --exit-status
gh run view 26940230718
gh api repos/FrankQDWang/resume-ir/actions/runs/26940230718/artifacts --jq '.artifacts[] | {name, expired}'
gh run view 26940230718 --json conclusion,status,workflowName,event,headBranch,headSha,url,jobs
if gh run view 26940230718 --job 79479637142 --log | rg -n 'Node\.js 20 actions are deprecated|actions/checkout@v4|actions/upload-artifact@v4'; then
  printf '%s\n' 'node20 action warning found'
  exit 1
else
  printf '%s\n' 'no Node.js 20 action warning or v4 action reference found in release dry-run job log'
fi
```

Output summary:

- Release workflow run `26940230718`: conclusion `success`, event
  `workflow_dispatch`, branch `codex/fault-injection-diagnostics`, head SHA
  `ea043fcd9b9cbb1edc89b50333bcd577c6a3910f`.
- Release job `79479637142`: exit 0; completed in 1m31s; checkout, workspace
  verify, release binary build, artifact manifest generation, SBOM generation,
  public artifact boundary check, dry-run artifact upload, and release gate all
  passed.
- Artifact metadata: `release-dry-run`, `expired: false`; artifact was not
  downloaded.
- Specific log scan: exit 0; no `Node.js 20 actions are deprecated`,
  `actions/checkout@v4`, or `actions/upload-artifact@v4` text was found in the
  release dry-run job log.
- Broader warning scan observed a separate Node `[DEP0040]` `punycode`
  deprecation warning in the `Cache Rust` and `Post Cache Rust` steps from
  `Swatinem/rust-cache@v2`; this is not the Node 20 action-major warning fixed
  in S125 and remains a separate follow-up risk.

Hosted PR checks:

```bash
gh pr checks 9 --watch
```

Output summary:

- PR #9 hosted checks for commit `ea043fc` passed: dependency tree, license
  policy, runbook policy, macOS Platform CI, Rust workspace, public repository
  guard, and Windows Platform CI.
- Sourcery review reported `skipping`.

Scope note:

- S126 proves only the hosted release dry-run path and current PR checks for
  the pushed workflow compatibility commit. It does not build installers, sign
  or notarize artifacts, create/upload a GitHub Release, validate installer
  lifecycle behavior, prove release readiness, or resolve the separate cache
  action `punycode` warning.

### S125

Design target:

- Remove the GitHub Actions Node.js 20 deprecation warning surfaced by the
  hosted release dry-run workflow.
- Pin tracked workflows to current Node 24-compatible checkout and artifact
  upload action majors.
- Extend the workflow policy guard so future workflow edits cannot reintroduce
  the deprecated action majors.

Observed hosted warning:

```bash
gh workflow run Release --ref codex/fault-injection-diagnostics -f version=v0.0.0
gh run watch 26939532282 --exit-status
```

Output summary:

- Hosted Release workflow run `26939532282` passed on the feature branch and
  uploaded the dry-run release manifest/SBOM artifact.
- GitHub also emitted a deprecation annotation for Node.js 20 actions affecting
  `actions/checkout@v4` and `actions/upload-artifact@v4`.

Version check:

```bash
gh release list -R actions/checkout --limit 5
gh release list -R actions/upload-artifact --limit 5
```

Output summary:

- Official GitHub release listings showed `actions/checkout` latest `v6.0.3`
  and `actions/upload-artifact` latest `v7.0.1`.

Implementation checks:

```bash
rg -n 'actions/checkout@|actions/upload-artifact@' .github/workflows
./scripts/ci/check-workflows.sh
./scripts/ci/check-release-sbom.sh
./scripts/ci/check-release-artifacts.sh
ruby -e 'require "yaml"; ARGV.each { |file| YAML.load_file(file); puts "yaml ok: #{file}" }' .github/workflows/*.yml
cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- Workflow references: exit 0; tracked workflows now use
  `actions/checkout@v6`, with artifact uploads using
  `actions/upload-artifact@v7`.
- Workflow guard: exit 0; required workflow action versions are enforced and
  deprecated `actions/checkout@v4` plus `actions/upload-artifact@v4` are
  rejected in the guarded workflow set.
- Release SBOM guard: exit 0.
- Release artifact guard: exit 0.
- Workflow YAML parse: exit 0 for every tracked workflow.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, release SBOM check, and public repository guard passed.

Scope note:

- S125 removes the tracked workflow action runtime deprecation risk. It does
  not build installers, sign or notarize artifacts, create a GitHub Release,
  validate installer lifecycle behavior, or complete release readiness. The
  updated hosted release dry-run still needs to run after this branch is pushed.

### S124

Design target:

- Generate a redacted release dry-run SBOM from locked Cargo metadata as a
  standard SPDX 2.3 JSON document.
- Omit local metadata paths, source paths, license-file paths, target
  directories, runtime data, diagnostics, model caches, and resume data.
- Wire the SBOM guard into local verification and the manual release dry-run
  workflow while keeping packaging, signing, notarization, and GitHub Release
  upload explicitly gated.

Observed RED:

```bash
./scripts/ci/check-release-sbom.sh
```

Output summary:

- The focused release SBOM guard failed because
  `scripts/ci/check-release-sbom.sh` did not exist.

Implementation checks:

```bash
./scripts/ci/check-release-sbom.sh
./scripts/ci/check-release-artifacts.sh
./scripts/ci/check-workflows.sh
./scripts/ci/check-runbooks.sh
sh -n scripts/release/create-sbom.sh scripts/ci/check-release-sbom.sh scripts/release/create-artifact-manifest.sh scripts/ci/check-release-artifacts.sh scripts/ci/check-workflows.sh scripts/ci/verify-local.sh
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- Release SBOM guard: exit 0; generated `release-sbom.json`, verified SPDX
  2.3 metadata, workspace package PURLs, registry/workspace source-kind
  annotations, invalid-version rejection, workflow wiring, local verify wiring,
  and absence of temp paths, repo-local paths, target paths, runtime-data
  markers, manifest paths, source paths, and license-file paths.
- Release artifact guard: exit 0; the dry-run artifact manifest still records
  binary names, byte counts, and sha256 hashes without temp paths.
- Workflow guard: exit 0; release workflow now generates and uploads both
  `release-artifacts.json` and `release-sbom.json`.
- Runbook guard: exit 0.
- Shell syntax checks: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, release SBOM check, and public repository guard passed.

Scope note:

- S124 adds release dry-run SBOM evidence only. It does not build installers,
  sign or notarize artifacts, create a GitHub Release, upload release binaries,
  validate installer lifecycle behavior, or complete release readiness.

### S123

Design target:

- Let `resume-cli witness` prove the persisted field-extraction path on private
  PDF/Word samples without printing field values.
- Keep the probe aggregate-only: status, document count, mention count, and
  per-field-type counts.
- Read field probe evidence from metadata-only `entity_type` count aggregation
  without selecting raw or normalized field values.
- Never print or commit private field values, filenames, paths, raw text,
  private queries, diagnostics, or temporary witness data.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_probe_fields_reports_aggregate_counts_without_values_or_paths --locked -- --exact
```

Output summary:

- The focused witness test failed because `resume-cli witness` did not accept
  `--probe-fields` and returned the witness usage string.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_probe_fields_reports_aggregate_counts_without_values_or_paths --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- Focused witness field-probe test: exit 0; it confirmed the probe completes
  with non-zero field mentions and does not print the private root, canonical
  private root, data dir, private filenames, fixture filenames, or extracted
  field values.
- Full import/search witness suite: exit 0; 25 tests passed.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Private local-only field witness: exit 0; redacted aggregate status showed
  completed import, completed field probe, and private data removal; the probe
  used metadata-only field-type aggregation, temporary stdout/stderr logs were
  removed, and no private output was committed.
- Private local-only bounded OCR/field witness: exit 0; redacted aggregate
  status showed completed import, completed OCR, completed field probe, and
  private data removal; temporary stdout/stderr logs were removed and no
  private output was committed.
- Marker scan: no private sample root, path marker, token marker, or temporary
  witness-log marker was present in tracked progress/code changes.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, and public repository guard passed.

Scope note:

- S123 proves only a redacted local witness field-extraction probe and bounded
  local OCR/field witness behavior. It does not prove field extraction quality,
  real labeled field F1, full-library OCR completion, real ranking quality,
  large-corpus performance, Windows/Linux private sample behavior, or release
  readiness.

### S122

Design target:

- Let `resume-cli witness` prove a local import-to-search loop on private
  PDF/Word samples without requiring the user to supply a query.
- Generate the search probe query only inside the temporary private witness
  data directory, never print the query, matched filenames, snippets, paths, or
  raw resume text, and remove temporary private witness data after the run.
- Keep the probe aggregate-only: status plus hit count.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_probe_search_runs_private_query_without_leaking_query_or_paths --locked -- --exact
```

Output summary:

- The focused witness test failed because `resume-cli witness` did not accept
  `--probe-search` and returned the witness usage string.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_probe_search_runs_private_query_without_leaking_query_or_paths --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
gh pr checks 9 --watch
```

Output summary:

- Focused witness search-probe test: exit 0; it confirmed the probe completes
  with non-zero hits and does not print the private root, canonical private
  root, data dir, private filenames, fixture filenames, or internal query.
- Full import/search witness suite: exit 0; 24 tests passed.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, and public repository guard passed.
- Private local-only import/search witness: exit 0; redacted aggregate status
  showed completed import, completed search probe, and private data removal;
  temporary stdout/stderr logs were removed and no private output was committed.
- Private local-only bounded OCR/search witness: exit 0; redacted aggregate
  status showed completed import, completed OCR, completed search probe, and
  private data removal; temporary stdout/stderr logs were removed and no private
  output was committed.
- Hosted PR checks: final run passed macOS Platform CI, Windows Platform CI,
  Rust workspace, dependency tree, license policy, runbook policy, and public
  repository guard.

Scope note:

- S122 proves only a redacted local witness search probe and bounded local
  OCR/search witness behavior. It does not prove full-library OCR completion,
  real ranking quality, large-corpus performance, production embedding model
  readiness, Windows/Linux private sample behavior, or release readiness.

### S121

Design target:

- Generate a release dry-run manifest for already-built binaries with artifact
  names, byte counts, and sha256 hashes only.
- Keep packaging status explicitly blocked until installer packaging, signing,
  notarization, SBOM, and release upload are separately approved and proven.
- Wire the manifest check into local verification and the release workflow
  without recording local build paths or runtime data.

Observed RED:

```bash
sh scripts/ci/check-release-artifacts.sh
```

Output summary:

- The focused release artifact guard failed because
  `scripts/release/create-artifact-manifest.sh` did not exist.

Implementation checks:

```bash
sh scripts/ci/check-release-artifacts.sh
sh scripts/ci/check-workflows.sh
sh scripts/ci/check-runbooks.sh
git diff --check
./scripts/ci/verify-local.sh
```

Output summary:

- Release artifact guard: exit 0; generated a synthetic
  `release-artifacts.json`, rejected an invalid version, rejected a missing
  release binary, verified workflow artifact upload wiring, and verified the
  manifest did not contain the synthetic temp path.
- Workflow guard: exit 0.
- Runbook guard: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, and public repository guard passed.

Scope note:

- S121 is release dry-run evidence only. It does not create install packages,
  sign or notarize artifacts, create an SBOM, create a GitHub Release, upload
  release binaries, or validate installer/service lifecycle behavior.

### S120

Design target:

- Let users check the configured Tesseract OCR language from local diagnostics
  without dumping the full local `--list-langs` output.
- Keep `doctor` and `export-diagnostics --redact` output path-redacted and
  free of unrelated language-pack names.
- Preserve the default English check when no OCR language is requested.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_check_requested_ocr_language_without_language_dump --locked -- --exact
```

Output summary:

- The focused diagnostics test failed because `doctor --ocr-lang chi_sim`
  returned non-zero; diagnostics did not parse a requested OCR language and
  only checked `eng`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_check_requested_ocr_language_without_language_dump --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
gh pr checks 9 --watch
```

Output summary:

- Focused requested-language diagnostics test: exit 0; 1 test passed.
- Full diagnostics suite: exit 0; 12 tests passed.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.
- Hosted PR checks: final run passed macOS Platform CI, Windows Platform CI,
  Rust workspace, dependency tree, license policy, runbook policy, and public
  repository guard.

Scope note:

- S120 improves local OCR runtime diagnostics only. It does not package OCR
  engines or language packs and does not prove non-English OCR quality.

### S119

Design target:

- Keep `service status` portable under non-macOS binary clippy while preserving
  macOS launchctl parser coverage in tests.
- Compile `running` and `loaded` service runtime states only for macOS or test
  builds, where they are meaningful and exercised.
- Keep non-macOS service status behavior at `runtime: unknown`.

Observed RED:

```bash
gh run view 26935300792 --job 79463500481 --log
```

Output summary:

- Hosted Rust Workspace for `c56e966` failed during Ubuntu clippy.
- Clippy flagged a needless `return` in the non-macOS `service status` branch.
- Clippy also flagged `Running`, `Loaded`, and the launchctl parser as dead code
  in non-macOS binary builds.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli launchctl_status --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
gh pr checks 9 --watch
```

Output summary:

- Focused CLI clippy: exit 0.
- Launchctl parser tests: exit 0; 4 tests passed.
- Service lifecycle integration tests: exit 0; 4 tests passed.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.
- Hosted PR checks: final run passed macOS Platform CI, Windows Platform CI,
  Rust workspace, dependency tree, license policy, runbook policy, and public
  repository guard.

Scope note:

- A local Linux-target clippy reproduction was attempted, but the macOS host did
  not have `x86_64-linux-gnu-gcc` for `zstd-sys`; hosted Ubuntu CI is the
  cross-platform witness for this slice.

### S118

Design target:

- Keep the macOS LaunchAgent service command surface portable in tests and
  hosted Windows builds.
- On non-macOS platforms, `service status` must remain redacted and successful
  for installed plist fixtures, reporting `runtime: unknown` instead of trying
  macOS-only `/usr/bin/id` or `/bin/launchctl`.
- Preserve the S117 macOS runtime query behavior on macOS.

Observed RED:

```bash
gh run view 26934977875 --job 79462477830 --log
```

Output summary:

- Hosted Windows Platform CI for `288a4c9` failed in
  `service_status_and_uninstall_are_redacted_and_preserve_user_data`.
- The status command returned non-zero because the runtime query attempted the
  macOS launchctl-domain path before handling the non-macOS platform.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli launchctl_status --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
gh pr checks 9 --watch
```

Output summary:

- Service lifecycle integration tests: exit 0; 4 tests passed and status now
  asserts that a redacted `runtime:` line is present.
- Launchctl parser tests: exit 0; 4 tests passed.
- Focused clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.
- Hosted PR checks for `c56e966`: Windows Platform CI was addressed, but Rust
  Workspace failed on Ubuntu clippy and is handled in S119.

Scope note:

- S118 is a portability fix for the macOS LaunchAgent command surface. It does
  not implement Windows services/MSI or prove Windows service lifecycle.

### S117

Design target:

- Make `resume-cli service status` report runtime state, not only plist
  presence, while preserving redacted CLI output.
- Query `launchctl print` for installed macOS LaunchAgents and map results to
  `running`, `loaded`, `not_loaded`, or `unknown` without printing launchctl
  diagnostics, local paths, logs, or data directories.
- Prove a local-only temporary LaunchAgent can install, start, serve status
  through authenticated IPC auto-discovery, stop, and uninstall without reading
  or persisting real resume data.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli tests::launchctl_status_success_with_running_state_reports_running --locked -- --exact
```

Output summary:

- The test failed before implementation because
  `service_runtime_state_from_launchctl_result` and `ServiceRuntimeState` did
  not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli launchctl_status --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked
/Users/frankqdwang/.cargo/bin/cargo build -p resume-cli -p resume-daemon --locked
target/debug/resume-cli --data-dir "$data_dir" service install --launch-agent-dir "$launch_dir" --label "$label" --daemon-binary "$PWD/target/debug/resume-daemon"
target/debug/resume-cli --data-dir "$data_dir" service status --launch-agent-dir "$launch_dir" --label "$label"
target/debug/resume-cli --data-dir "$data_dir" service start --launch-agent-dir "$launch_dir" --label "$label"
target/debug/resume-cli --data-dir "$data_dir" status --ipc auto
target/debug/resume-cli --data-dir "$data_dir" service stop --launch-agent-dir "$launch_dir" --label "$label"
target/debug/resume-cli --data-dir "$data_dir" service uninstall --launch-agent-dir "$launch_dir" --label "$label"
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- Launchctl status parser tests: exit 0; 4 tests passed for running, loaded,
  not-loaded, and unknown states.
- Service lifecycle integration tests: exit 0; install/status/uninstall and
  dry-run start/stop output remained redacted.
- Local macOS LaunchAgent witness: exit 0; install reported configured,
  pre-start status reported `runtime: not_loaded`, start reported started,
  post-start status reported `runtime: running`, `status --ipc auto` returned a
  redacted empty-store daemon status, stop reported stopped, post-stop status
  reported `runtime: not_loaded`, uninstall reported user data preserved, and
  temporary local data was removed.
- Focused clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.

Scope note:

- S117 proves a local temporary macOS LaunchAgent start/stop/status path on the
  current host only. It does not prove signed macOS pkg/dmg packaging,
  notarization, upgrade/rollback, Windows service/MSI install/uninstall, or
  release workflow execution.

### S116

Design target:

- Make full-text snapshot read-open robust to transient Windows directory/file
  handle release delays observed immediately after snapshot inspection.
- Keep retry bounded and specific to `FullTextIndex::open`, without changing
  full-text write, publish, or fallback semantics.
- Treat persistent access denial as a real error after retry exhaustion.

Observed RED:

```bash
gh run view 26934219893 --job 79460099591 --log
```

Output summary:

- Hosted Windows Platform CI for `f15ce1e` failed in
  `published_snapshot_becomes_active_without_reading_staging_orphans`.
- The failing call was `FullTextIndex::open_active(&index_root).unwrap()`,
  with Tantivy reporting `Access is denied. (os error 5)` after
  `inspect_snapshot_root` had just validated the active snapshot.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext tests::index_open_retries_transient_windows_access_denied --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext published_snapshot_becomes_active_without_reading_staging_orphans --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
gh pr checks 9 --watch
```

Output summary:

- Retry unit test: exit 0; the synthetic Tantivy access-denied diagnostic was
  retried and succeeded on the third attempt.
- Hosted-failing full-text snapshot test: exit 0 locally.
- `cargo test -p index-fulltext --locked`: exit 0; 3 unit tests, 12 integration
  tests, and doc-tests passed.
- Focused clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.
- Hosted PR checks: final run passed macOS Platform CI, Windows Platform CI,
  Rust workspace, dependency tree, license policy, runbook policy, and public
  repository guard.

Scope note:

- S116 covers transient local Windows read-open access denial around Tantivy
  snapshot directories. It does not prove network filesystem behavior,
  installer/service behavior, or production large-corpus full-text latency.

### S115

Design target:

- Prevent stale CLI/daemon vector-index writers from losing each other's
  updates when multiple `PersistentVectorIndex` instances open the same local
  `vector-index` root.
- Use a stable sidecar lock file rather than locking the replaceable snapshot
  file, so Windows snapshot rename semantics stay isolated from locking.
- While holding the writer lock, reload the latest durable vector snapshot,
  apply the current mutation, atomically rewrite the snapshot, and refresh the
  current instance's HNSW ANN state before returning.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector persistent_vector_index_merges_writes_from_stale_concurrent_openers --locked -- --exact
```

Output summary:

- The test failed before implementation with final `vector_count` equal to 1
  instead of 2, proving that a stale second opener overwrote the first opener's
  vector update.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-vector --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-licenses.sh
./scripts/ci/verify-local.sh
```

Output summary:

- `cargo test -p index-vector --locked`: exit 0; 10 vector index tests passed,
  including stale concurrent opener merge, stale opener tombstone preservation,
  local ANN refresh after merge, model-scoped ANN search, and stale-node
  prevention after upsert/tombstone.
- Focused clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/check-licenses.sh`: exit 0; license check passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.

Sub-agent orchestration:

- Subagent guidance was used under the Codex host-approved sub-agent tool as a
  read-only sidecar audit. The sub-agent confirmed the lost-update scenario,
  recommended a stable sidecar lock plus lock-held reload/merge/write, and did
  not edit files.

Scope note:

- S115 covers cooperative local file locking for vector snapshot mutations. It
  does not prove behavior on network filesystems, serialized HNSW graph
  persistence, production model quality, or real large-corpus vector latency.

### S114

Design target:

- Move the persistent vector query path beyond linear scan by adding a
  permissive-license HNSW ANN backend inside `index-vector`.
- Preserve the existing durable vector snapshot format and model-scoped query
  isolation; rebuild the in-process ANN graph from persisted vectors on open,
  upsert, deletion, and purge.
- Report the ANN backend through local status, doctor, and redacted diagnostics
  without emitting vector values, local paths, model command paths, or resume
  text.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector persistent_vector_index_uses_hnsw_ann_backend_after_reopen_and_keeps_model_scope --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_persistent_vector_snapshot_without_path_or_values --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker embed_worker_runs_local_command_and_persists_vector_snapshot_without_hiding_search_results --locked -- --exact
```

Output summary:

- The index-vector test failed before implementation because
  `VectorSearchBackend` and `VectorSnapshot::search_backend()` were unresolved.
- The CLI diagnostics and embed-worker exact tests failed before diagnostics
  implementation because vector status still reported
  `available (vector snapshot)` instead of the ANN backend.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_persistent_vector_snapshot_without_path_or_values --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker embed_worker_runs_local_command_and_persists_vector_snapshot_without_hiding_search_results --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-licenses.sh
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-vector -p resume-cli --all-targets --locked -- -D warnings
./scripts/ci/verify-local.sh
```

Output summary:

- `cargo test -p index-vector --locked`: exit 0; 8 vector index tests passed,
  including persistent HNSW ANN backend reporting after reopen, model-scoped
  ANN search, and HNSW rebuild after upsert/tombstone so stale nodes are not
  returned.
- Focused CLI diagnostics and embed-worker tests: exit 0; status, doctor, and
  redacted diagnostics now report `hnsw_ann`/`available (hnsw ann vector
  snapshot)` without local paths or vector values.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/check-licenses.sh`: exit 0; license check passed for the new
  `hnsw_rs` dependency set.
- Focused clippy: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.

Scope note:

- S114 adds an HNSW ANN query backend for the persisted vector index but does
  not prove a production embedding model, real semantic quality, large-corpus
  ANN recall/latency, cross-platform hosted execution for the new dependency,
  or signed/release packaging.

### S113

Local-only private sample validation:

```bash
target/debug/resume-cli --data-dir <temporary-unused-data-dir> witness --root <authorized-private-sample-root> --max-files 10000
target/debug/resume-cli --data-dir <temporary-unused-data-dir> witness --root <authorized-private-sample-root> --max-files 10000 --run-ocr --ocr-max-documents 5 --ocr-tesseract-command <local-tesseract> --ocr-pdftoppm-command <local-pdftoppm>
```

Output summary:

- Import-only witness: exit 0; redacted aggregate status showed completed
  import and private data removal.
- Bounded OCR witness: exit 0; redacted aggregate status showed completed
  import, completed bounded OCR, expected OCR budget behavior, and private data
  removal.
- No real resume paths, filenames, counts, extracted text, OCR output, tokens,
  diagnostics packages, or model caches were committed or uploaded.

Scope note:

- S113 validates the current local-only import/OCR witness behavior on the
  authorized private sample root. It does not prove full-library OCR completion,
  quality, performance, installer/service behavior, or cross-platform real-data
  behavior.

### S112

Design target:

- Make hosted macOS and Windows workspace build/test validation run on pull
  requests rather than only on manual or scheduled workflows.
- Extend the workflow policy guard so the platform matrix and core build/test
  commands cannot be silently removed.
- Keep the scope to build/test validation only; packaging, signing,
  notarization, MSI/pkg/dmg install flows, and service lifecycle proof remain
  separate release blockers.

Observed RED:

```bash
./scripts/ci/check-workflows.sh
```

Output summary:

- The workflow guard failed because `.github/workflows/ci-platform.yml` was
  missing required text: `pull_request`.

Implementation checks:

```bash
./scripts/ci/check-workflows.sh
ruby -e 'require "yaml"; ARGV.each { |file| YAML.load_file(file); puts "yaml ok: #{file}" }' .github/workflows/ci-platform.yml .github/workflows/pr.yml .github/workflows/bench-nightly.yml
git diff --check
./scripts/ci/verify-local.sh
```

Output summary:

- Workflow guard: exit 0 after Platform CI required `pull_request`,
  `macos-latest`, `windows-latest`, `cargo build --workspace --locked`, and
  `cargo test --workspace --locked`.
- Workflow YAML parse: exit 0 for Platform CI, PR, and nightly workflow files.
- `git diff --check`: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, workflow check, and
  public repo guard.

Hosted CI follow-up:

- The first pushed Platform CI run passed `macos-latest` and failed
  `windows-latest`.
- Windows failed compiling `crates/cli/tests/s9_import_search.rs` because two
  witness OCR tests called `write_fixture_executable` while that helper was
  gated behind `#[cfg(unix)]`.
- The fix keeps witness OCR command execution covered on Windows by writing
  `.cmd` fixture commands under `#[cfg(windows)]` instead of skipping the
  tests.
- Local focused verification after the fix:
  `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked`
  passed with 23 tests.
- Local Windows target checking was attempted with
  `/Users/frankqdwang/.cargo/bin/cargo check -p resume-cli --test s9_import_search --target x86_64-pc-windows-gnu --locked`,
  but this macOS host lacks `x86_64-w64-mingw32-gcc`; hosted Windows CI remains
  the authoritative validation for the Windows path.
- The next hosted Platform CI run compiled through the witness fix, then failed
  macOS in `resume-daemon --test s4_daemon` because two daemon integration tests
  exceeded the test harness's 8 second child-process wait on the hosted runner.
  The daemon was still making progress; the fix raises the test harness wait
  budget to 45 seconds while leaving the product `--max-worker-ticks` settings
  unchanged.
- That hosted run failed Windows in `benchmark-runner --test s17_benchmark_cli`
  because OCR and embedding benchmark fixtures still generated Unix shell
  scripts. The fix adds Windows `.cmd` fixtures for the same local command
  protocols in both CLI-level and runner-level benchmark tests.
- Local focused verification after the second fix:
  `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli --locked`,
  `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner --locked`,
  and
  `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked`
  passed.
- The next hosted Windows Platform CI run reached `resume-cli --test
  s14_delete_search` and failed three reimport deletion assertions. Root cause:
  deletion propagation compared stored slash-normalized document paths like
  `d:/...` against native Windows `PathBuf` roots with `Path::starts_with`,
  so missing files were not consistently recognized under the import root on
  Windows.
- The fix now normalizes import roots with `fs_crawler::normalize_path` and
  compares normalized path boundaries for root/skipped-subtree checks.
- Local focused verification after the Windows path fix:
  `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked` and
  `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked`
  passed.
- Red/green verification: with the old native `Path::starts_with` comparison
  temporarily restored, `/Users/frankqdwang/.cargo/bin/cargo test -p
  import-pipeline deletion_candidate_matches_windows_normalized_paths --locked`
  failed on the Windows-style normalized path assertion; after restoring the
  fix, the same focused regression, `s14_delete_search`, `cargo fmt --check`,
  public guard/marker scans, and `./scripts/ci/verify-local.sh` passed.
- The next hosted Windows Platform CI run passed the deletion assertions but
  failed three `s14_delete_search` cases during initial CLI import with the
  redacted error `resume-cli: search index update failed`; macOS, Rust
  workspace, and all policy checks passed in the same run.
- Root cause: full-text snapshot publishing validated by opening a reader on the
  staging directory, then immediately renamed that same staging directory. That
  is fragile on Windows where recently opened index files can remain locked
  briefly after handles are dropped.
- The fix now publishes the staging snapshot to the immutable snapshots
  directory before validation, validates the published snapshot before moving
  the active pointer, removes a failed published snapshot best-effort, and
  retries transient publish locks.
- Local focused verification after the full-text publish fix:
  `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked` and
  `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked`
  passed.
- The next hosted Windows Platform CI run passed `s14_delete_search` and then
  failed witness tests in `s9_import_search` after successful import/OCR
  summaries because private witness temp data cleanup reported `cleanup_failed`.
- Root cause: the witness command attempted to delete the temporary private data
  root while metadata/index handles could still be open; Unix tolerated that,
  but Windows does not delete open files/directories.
- The fix now drops the witness metadata store before cleanup and retries
  temporary witness root deletion to absorb transient Windows handle release.
- Local focused verification after the witness cleanup fix:
  `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked`,
  `cargo fmt --check`, guard/marker scans, and `./scripts/ci/verify-local.sh`
  passed.
- Final hosted PR validation for run `26932238947` / `26932238946` /
  `26932238956`: `macos-latest` passed in 1m30s, `windows-latest` passed in
  4m14s, `rust workspace` passed in 1m18s, and dependency tree, license policy,
  runbook policy, and public repository guard all passed.

Scope note:

- S112 improves hosted cross-platform build/test coverage only. It does not
  prove platform installer behavior, service manager behavior, signing,
  notarization, upgrade, uninstall, rollback, real whole-machine scans, or
  complete release readiness.
- Full product is still not complete.

### S111

Design target:

- Wire the S110 vector-quality evaluator/gate into PR and nightly benchmark
  smoke workflows.
- Keep the workflow smoke local-only, synthetic-labeled, redacted, and explicit
  that it is not proof of production semantic quality.
- Extend the workflow policy guard so future edits cannot silently drop the
  vector smoke gate.

Observed RED:

```bash
./scripts/ci/check-workflows.sh
```

Output summary:

- The workflow guard failed because `.github/workflows/pr.yml` was missing the
  required `resume-benchmark --locked -- vector-quality` command.

Implementation checks:

```bash
./scripts/ci/check-workflows.sh
sh -n scripts/ci/check-workflows.sh
ruby -e 'require "yaml"; ARGV.each { |file| YAML.load_file(file); puts "yaml ok: #{file}" }' .github/workflows/pr.yml .github/workflows/bench-nightly.yml
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Workflow guard: exit 0 after PR and nightly workflows were wired to run
  `vector-quality` and `vector-gate`.
- Strict local vector smoke reproduction: exit 0; `vector quality gate passed`,
  and the generated report did not contain the temporary command path, raw
  queries, candidate text, candidate IDs, or vector values.
- Shell syntax check: exit 0.
- Workflow YAML parse: exit 0 for PR and nightly workflow files.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, workflow check, and
  public repo guard.

Scope note:

- S111 adds CI coverage for the benchmark gate path only. It does not provide
  real business relevance labels, choose or license a production embedding
  model, prove ANN behavior, prove 100k/1M semantic latency, or complete product
  readiness.
- Full product is still not complete.

### S110

Design target:

- Add a redacted labeled vector-quality evaluator and gate that use the existing
  local embedding command protocol.
- Score recall@k, MRR, NDCG@k, and zero-recall query count from JSONL samples.
- Keep reports free of raw queries, candidate text, sample IDs, candidate IDs,
  vectors, command paths, resume paths, and real filenames.
- Keep private PDF/Word witness validation local-only and bounded.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner vector_quality_report_scores_labeled_samples_without_text_id_path_or_vector_leakage --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_vector_quality_outputs_redacted_report_and_gate --locked -- --exact
```

Output summary:

- The runner exact failed because `VectorQualityConfig`,
  `VectorQualityGateConfig`, `run_vector_quality_jsonl`, and
  `evaluate_vector_quality_gate_json` did not exist.
- The CLI exact failed because `resume-benchmark` rejected the
  `vector-quality` command.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner vector_quality_report_scores_labeled_samples_without_text_id_path_or_vector_leakage --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_vector_quality_outputs_redacted_report_and_gate --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings
```

Output summary:

- Focused vector-quality runner exact: exit 0.
- Focused vector-quality CLI exact: exit 0.
- `benchmark-runner`: exit 0; full crate tests passed, including vector gate
  acceptance/rejection and redaction coverage.
- Focused benchmark-runner clippy: exit 0.
- A private local-only bounded PDF/Word witness against the user-authorized
  sample directory completed with redacted aggregate output and temporary
  private data removal.
- A private local-only bounded OCR witness completed with redacted processed and
  failed document counters, explicit OCR budget exhaustion reporting, and
  temporary private data removal.
- A private local-only Word-only witness completed with redacted aggregate
  output and temporary private data removal.
- No real resume path, filename, raw text, OCR text, command path, count, or
  diagnostic payload was committed or uploaded.

Scope note:

- S110 adds a quality gate surface and redaction boundary for labeled vector
  retrieval evaluation. It does not choose a licensed embedding model, ship a
  model pack, provide real business relevance labels, add ANN indexing, prove
  production semantic latency, or complete product readiness.
- Full product is still not complete.

### S109

Design target:

- Make `resume-cli witness --run-ocr --ocr-max-documents <n>` useful against a
  real private resume directory when one OCR document fails before the budget is
  exhausted.
- Count OCR document attempts as successful plus failed documents, continue
  through the configured document budget after per-document OCR failures, and
  report redacted aggregate `ocr documents failed` output.
- Preserve the existing `blocked` status when no OCR command is configured, and
  do not print real paths, filenames, OCR text, command paths, or diagnostics.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_budget_reports_failed_documents_without_stopping_or_leaking_paths --locked -- --exact
```

Output summary:

- The test failed because `resume-cli witness` still reported
  `witness ocr status: blocked` after the first OCR failure instead of
  completing the bounded witness and reporting a failed-document counter.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_budget_reports_failed_documents_without_stopping_or_leaking_paths --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused OCR witness resilience exact: exit 0.
- `s9_import_search`: exit 0; 23 tests passed.
- `cargo fmt --check`: exit 0.
- Focused CLI clippy: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, workflow check, and
  public repo guard.
- A private local-only full-directory PDF/Word witness using the
  user-authorized sample directory passed with redacted aggregate output and
  temporary private data removal.
- A private local-only bounded OCR witness using local renderer/OCR runtimes
  completed with redacted processed and failed document counters, explicit OCR
  budget exhaustion reporting, and temporary private data removal.
- No real resume path, filename, raw text, OCR text, count, command path, or
  diagnostic payload was committed or uploaded.

Scope note:

- S109 makes bounded real-library OCR witnessing more resilient. It does not
  prove OCR quality, full-library OCR completion, non-English OCR behavior,
  packaged runtime distribution, 100k/1M performance, Windows/Linux behavior,
  or complete product readiness.
- Full product is still not complete.

### S108

Design target:

- Wire the synthetic OCR throughput benchmark/gate from S107 into PR and nightly
  benchmark smoke workflows.
- Add a workflow policy guard so required query and OCR benchmark smoke gates
  cannot silently disappear from workflows or local verification.
- Keep workflow artifacts redacted; no real resume paths, raw resume text,
  diagnostics, or local data are uploaded.

Observed RED:

```bash
sh scripts/ci/check-workflows.sh
```

Output summary:

- The new workflow policy guard failed because `.github/workflows/pr.yml` did
  not include `resume-benchmark --locked -- ocr-throughput`.

Implementation checks:

```bash
sh scripts/ci/check-workflows.sh
tmpdir=$(mktemp -d); trap 'rm -rf "$tmpdir"' EXIT; printf '%s\n' '#!/usr/bin/env sh' 'printf "resume-ir-ocr-v1\nconfidence=0.97\ntext:\nSynthetic OCR smoke page %s\n" "$RESUME_IR_OCR_PAGE_NO"' > "$tmpdir/ocr-fixture.sh"; chmod 700 "$tmpdir/ocr-fixture.sh"; /Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- ocr-throughput --command "$tmpdir/ocr-fixture.sh" --pages 3 --page-timeout-ms 5000 --json > "$tmpdir/ocr-benchmark-smoke.json"; /Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- ocr-gate --report "$tmpdir/ocr-benchmark-smoke.json" --allow-synthetic --min-pages 3 --max-p95-ms 5000 --min-pages-per-second 0.001; if rg -n 'Synthetic OCR smoke|resume-ir-ocr-v1|RESUME_IR_OCR|/tmp/' "$tmpdir/ocr-benchmark-smoke.json"; then exit 1; fi
sh -n scripts/ci/check-workflows.sh scripts/ci/verify-local.sh scripts/ci/check-runbooks.sh scripts/ci/guard-public-repo.sh
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/verify-local.sh
```

Output summary:

- Workflow policy guard: exit 0.
- Synthetic local OCR benchmark smoke and gate: exit 0; redacted report did not
  include synthetic OCR text, OCR protocol text, OCR environment names, or temp
  paths.
- Shell syntax checks: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, workflow check, and
  public repo guard.

Scope note:

- S108 adds workflow enforcement for synthetic OCR smoke only. It does not
  prove real scanned-resume OCR quality, full-library OCR completion,
  non-English language behavior, packaged OCR runtime distribution, 100k/1M
  corpus performance, or Windows/Linux validation.
- Full product is still not complete.

### S107

Design target:

- Add `resume-benchmark ocr-throughput` so the benchmark runner can measure
  synthetic OCR page throughput through the existing local OCR command protocol
  or Tesseract adapter without touching real resumes.
- Add `resume-benchmark ocr-gate` so synthetic OCR reports require explicit
  `--allow-synthetic` before they can pass a gate.
- Keep reports redacted: no raw OCR text, page bytes, command paths, resume
  paths, sample IDs, or private data.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner synthetic_ocr_throughput_reports_page_latency_without_payload_or_path_leakage --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_ocr_throughput_outputs_redacted_report_and_gate --locked -- --exact
```

Output summary:

- The library test failed because OCR throughput API symbols did not exist.
- The CLI test failed because `resume-benchmark` rejected `ocr-throughput` as
  unsupported usage.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner synthetic_ocr_throughput_reports_page_latency_without_payload_or_path_leakage -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_ocr_throughput_outputs_redacted_report_and_gate -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused OCR throughput library exact: exit 0.
- Focused OCR throughput CLI exact: exit 0.
- `benchmark-runner`: exit 0; 19 integration tests plus doc-tests passed.
- Focused benchmark-runner clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S107 proves only a synthetic OCR throughput report/gate path that exercises
  existing local OCR clients without leaking payloads or paths.
- It does not prove real scanned-resume OCR quality, full-library OCR
  completion, non-English language behavior, packaged OCR runtime distribution,
  100k/1M corpus performance, or Windows/Linux validation.
- Full product is still not complete.

### S106

Design target:

- Add `resume-cli witness --root-preset local-discovery` so the local witness
  command can exercise the same root-preset discovery path users need when they
  do not know where resumes are stored.
- Use the existing discovery profile skip rules for system/cache/dependency
  directories and keep output redacted.
- Continue anonymizing selected PDF/Word inputs into a temporary witness data
  directory and remove private witness data before returning.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_local_discovery_preset_uses_discovery_profile_without_path_leak --locked -- --exact
```

Output summary:

- The test failed because `resume-cli witness` rejected
  `--root-preset local-discovery` as unsupported usage.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_local_discovery_preset_uses_discovery_profile_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused local-discovery witness exact: exit 0.
- `s9_import_search`: exit 0; 22 tests passed.
- `fs-crawler`: exit 0; 11 tests passed plus doc-tests.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.
- A private local-only local-discovery witness using the user-authorized sample
  directory override passed with redacted aggregate output and temporary
  private data removal. No real resume path, filename, raw text, or diagnostic
  payload was committed or uploaded.

Scope note:

- S106 makes local-discovery witnessing possible without pretending to prove a
  full default whole-machine scan, Windows drive behavior, full-library OCR,
  OCR quality, or large-corpus performance.
- Full product is still not complete.

### S105

Design target:

- Add `resume-cli witness --run-ocr --ocr-max-documents <n>` so a private
  local root can be scanned/imported at its full witness file budget while OCR
  execution is independently bounded.
- Preserve the existing real OCR worker path for each processed document and
  output only aggregate redacted counters.
- Report whether the OCR document budget was exhausted; do not imply full OCR
  completion when queued OCR work remains.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_can_budget_documents_after_full_private_scan_without_path_leak --locked -- --exact
```

Output summary:

- The test failed because `resume-cli witness` rejected
  `--ocr-max-documents` as unsupported usage.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_can_budget_documents_after_full_private_scan_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused OCR witness-budget exact: exit 0.
- `s9_import_search`: exit 0; 21 tests passed.
- `s15_ocr_handoff`: exit 0; 12 tests passed.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.
- A private local-only full-directory witness using the user-authorized sample
  directory and local OCR runtimes passed with a bounded OCR document budget,
  redacted aggregate output, explicit OCR budget exhaustion reporting, and
  temporary private data removal. No real resume path, filename, raw text, or
  diagnostic payload was committed or uploaded.

Scope note:

- S105 makes full-root local OCR witnessing practical without pretending to
  complete full-library OCR. It does not prove OCR quality, throughput,
  non-English behavior, packaged runtime distribution, Windows/Linux behavior,
  or large-corpus performance.
- Full product is still not complete.

### S104

Design target:

- Add a safe `resume-cli fault-simulate --case migration-failure` probe that
  creates a synthetic broken migration-state SQLite database under scratch,
  invokes the real `MetaStore::run_migrations()` path, and removes probe data.
- Report only redacted aggregate output: fault name, reproduced status,
  migration check state, recovery guidance, and `paths: <redacted>`.
- Do not touch the caller's data directory and do not print paths, schema SQL,
  table names, raw SQLite errors, or resume data.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_metadata_migration_failure_reproduces_without_path_or_schema_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths --locked -- --exact
```

Output summary:

- The fault simulation exact failed because `migration-failure` was rejected as
  unsupported usage.
- The diagnostics exact failed because the redacted diagnostics skeleton did not
  include `metadata_migration`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_metadata_migration_failure_reproduces_without_path_or_schema_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused migration-failure fault simulation exact: exit 0.
- Focused diagnostics exact: exit 0.
- `s71_fault_injection`: exit 0; 10 tests passed.
- `s13_diagnostics`: exit 0; 11 tests passed.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S104 adds a safe synthetic migration failure fault simulation and updates
  doctor/diagnostics hook listings. It does not run destructive migration
  rollback drills on real metadata, prove backup/restore operations, prove
  cross-platform filesystem fault behavior, or complete upgrade rehearsal.
- Full product is still not complete.

### S103

Design target:

- Add a non-contact soft-dedupe scorer for same-name profiles using school,
  company, and skill overlap as bounded evidence.
- Surface low-confidence suspected-duplicate hints in local CLI and daemon
  full-text search results without assigning `candidate_id` or folding those
  versions.
- Output only aggregate hint data: suspected version count, maximum confidence,
  and `folded=false`; do not output raw names, schools, companies, contacts,
  paths, or dedupe keys.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion soft_dedupe --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding search_marks_soft_duplicate_hints_without_low_confidence_folding --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_includes_redacted_soft_dedupe_hints --locked -- --exact
```

Output summary:

- The rank-fusion test failed because `DedupeProfile` and
  `soft_dedupe_score` did not exist.
- The local CLI test failed because search output had no
  `soft_dedupe: suspected_versions=...` hint line.
- The daemon IPC test failed because search result JSON had no `soft_dedupe`
  object.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion soft_dedupe --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding search_marks_soft_duplicate_hints_without_low_confidence_folding --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_includes_redacted_soft_dedupe_hints --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p rank-fusion -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- Focused rank-fusion RED/GREEN: exit 0 after implementation.
- Focused local CLI soft-dedupe RED/GREEN: exit 0 after implementation.
- Focused daemon IPC soft-dedupe RED/GREEN: exit 0 after implementation.
- `rank-fusion`: exit 0; 7 tests passed plus doc-tests.
- CLI candidate-folding suite: exit 0; 2 tests passed.
- Daemon search IPC suite: exit 0; 5 tests passed.
- Focused clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S103 adds bounded soft-dedupe scoring and redacted search hints. It does not
  strong-fold low-confidence matches, does not persist manual merge decisions,
  does not prove real dedupe precision/recall, and does not prove million-corpus
  latency.
- Full product is still not complete.

### S102

Design target:

- Add `resume-benchmark field-quality --dataset <jsonl> --json` for labeled
  field extraction quality evaluation.
- Add `resume-benchmark field-gate --report <path>` with configurable minimum
  sample count, precision, recall, and F1 thresholds.
- Output only aggregate metrics; do not output raw resume text, sample IDs,
  paths, expected values, predicted values, email addresses, or phone numbers.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality --locked
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_quality_outputs_redacted_report_and_gate --locked -- --exact
```

Output summary:

- The library test failed because `evaluate_field_quality_gate_json`,
  `run_field_quality_jsonl`, and `FieldQualityGateConfig` did not exist.
- The CLI exact failed because `resume-benchmark` rejected `field-quality` as
  unsupported usage.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality --locked
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_quality_outputs_redacted_report_and_gate --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings
./scripts/ci/check-licenses.sh
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_ipc_connect_failure_does_not_fallback_to_sqlite --locked -- --exact --nocapture
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment --locked
./scripts/ci/verify-local.sh
```

Output summary:

- Focused field-quality library tests: exit 0; 3 tests passed.
- Focused field-quality CLI exact: exit 0.
- `benchmark-runner`: exit 0; 13 tests passed plus doc-tests.
- Focused benchmark-runner clippy: exit 0.
- License guard: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- An initial full verify attempt hit an existing IPC connect-failure test
  failure; its exact rerun passed.
- A later full verify attempt exposed an existing flaky contact-hash key test
  assertion that rejected any random key containing the short digit fragment
  `415`; the assertion was hardened to check full synthetic contact strings,
  and `s21_import_candidate_assignment` then passed.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S102 adds the evaluator/gate needed to measure field precision/recall/F1. It
  does not provide real business labeled datasets, does not prove production
  field F1 targets, does not broaden dictionaries, and does not complete
  soft-dedupe scoring.
- Full product is still not complete.

### S101

Design target:

- Add `resume-daemon run --foreground --work-imports --watch-import-roots`.
- Watch latest completed import roots through a real local filesystem watcher,
  aggregate relevant create/modify/remove events, and requeue the affected root
  through the existing durable import task plus scan-scope path.
- Print only aggregate watcher counts; do not print source roots, event paths,
  filenames, or notify error details.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_watcher_requeues_completed_root_after_file_change_without_path_leak --locked -- --exact
```

Output summary:

- The test failed because `resume-daemon` rejected `--watch-import-roots` as
  unsupported usage.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_watcher_requeues_completed_root_after_file_change_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets --locked -- -D warnings
./scripts/ci/check-licenses.sh
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused import watcher exact: exit 0.
- `s4_daemon`: exit 0; 12 tests passed.
- Focused daemon clippy: exit 0.
- License guard: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S101 adds local OS watcher event-to-import-task integration for completed
  roots. It does not prove Windows watcher behavior, long-running watcher soak
  stability, large-corpus event storms, or incremental index-update-only writes.
- Full product is still not complete.

### S100

Design target:

- Add optional `resume-cli witness --run-ocr` support that reuses the existing
  OCR worker path inside the isolated witness data directory.
- Accept local OCR command/Tesseract and renderer/pdftoppm options without
  printing command paths, rendered bytes, OCR text, source paths, filenames, or
  diagnostics.
- Report `completed` aggregate OCR work when local OCR executes, or explicit
  `blocked` aggregate output when OCR is requested but no local OCR command is
  configured. Always remove private witness input/data directories.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_executes_local_command_without_output_or_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_without_command_reports_blocked_without_persisting_private_data --locked -- --exact
```

Output summary:

- Both tests failed because `resume-cli witness` rejected `--run-ocr` and OCR
  options as unsupported usage.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_executes_local_command_without_output_or_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_without_command_reports_blocked_without_persisting_private_data --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused witness OCR completed exact: exit 0.
- Focused witness OCR blocked exact: exit 0.
- `s9_import_search`: exit 0; 20 tests passed.
- `s15_ocr_handoff`: exit 0; 12 tests passed.
- `cargo fmt --check`: exit 0.
- Focused CLI clippy: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.
- Private bounded local-only OCR witness using the user-authorized sample
  directory without an OCR command: exit 0 with explicit redacted `blocked`
  output and no metadata persisted in the external data directory.
- Private bounded local-only OCR witness using local OCR runtime commands:
  exit 0 with redacted `completed` output and no metadata persisted in the
  external data directory. This run used a file budget and did not prove
  full-library OCR coverage or throughput.

Scope note:

- S100 adds witness-level OCR execution/blocked reporting. It does not package
  OCR runtimes, prove non-English OCR, prove full-library OCR, prove
  large-corpus OCR throughput, or validate Windows/Linux.
- Full product is still not complete.

### S99

Design target:

- Add `resume-cli witness --root <path> [--max-files <count>]` for
  user-authorized local-only PDF/Word validation.
- Select only PDF/DOCX/DOC inputs, copy them under anonymized temporary
  filenames, run the existing import/index path in an isolated temporary data
  directory, and remove the temporary private input and data directories before
  returning.
- Print only aggregate redacted output; do not print source paths, filenames,
  resume text, diagnostics, or user sample counts in committed artifacts.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_imports_only_pdf_and_word_samples_without_persisting_private_data --locked -- --exact
```

Output summary:

- The test failed because the CLI rejected `witness` as an unknown top-level
  command.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_imports_only_pdf_and_word_samples_without_persisting_private_data --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused witness exact: exit 0.
- `s9_import_search`: exit 0; 18 tests passed.
- `cargo fmt --check`: exit 0.
- Focused CLI clippy: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.
- Private local-only PDF/Word witness using the user-authorized sample directory:
  exit 0 with redacted output, no scan-budget exhaustion at the default witness
  budget, and no metadata persisted in the external data directory. No real
  resume path, filename, count, raw text, or diagnostic payload was committed or
  uploaded.

Scope note:

- S99 adds a privacy-preserving local witness command for PDF/Word validation.
  It does not prove production-scale performance, complete converter/OCR/model
  packaging, validate Windows/Linux, or replace the remaining full-library
  quality gates.
- Full product is still not complete.

### S98

Design target:

- Add a cross-platform polling background import-rescan mode for completed
  import roots.
- Preserve import task history by creating a new queued task from the latest
  completed root scan scope, only when the same root has no queued/running/
  retryable task.
- Keep worker output redacted; do not print root paths, data directories, or
  filenames.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_rescans_completed_root_without_path_leak --locked -- --exact
```

Output summary:

- The test failed because `resume-daemon` rejected
  `--rescan-completed-imports` as an unknown usage path.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_rescans_completed_root_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon -p meta-store --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused daemon import-rescan exact: exit 0.
- `s4_daemon`: exit 0; 11 tests passed.
- `s3_sqlite`: exit 0; 43 tests passed.
- Focused daemon/meta-store clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S98 implements polling background rescan for completed roots. It does not
  implement a native OS filesystem watcher, prove long-running full-library
  rescans, or replace full snapshot rebuilds with incremental index writes.
- Full product is still not complete.

### S97

Design target:

- Treat legacy Word `.doc` as Word input rather than permanently failing it
  before parsing.
- Use a local converter with private temp input/output files, fixed timeout,
  bounded output size, hidden stdout/stderr, and redacted debug surfaces.
- Keep synthetic tests as the committed proof; use real samples only as
  uncommitted local witness data.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p parser-doc --test s6_doc extracts_legacy_doc_text_with_local_converter_without_output_leakage --locked -- --exact
```

Output summary:

- The test failed because `DocParser::with_converter` was not implemented.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p parser-doc --test s6_doc extracts_legacy_doc_text_with_local_converter_without_output_leakage --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline tests::import_root_parses_legacy_doc_with_local_converter_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p parser-doc --locked
/Users/frankqdwang/.cargo/bin/cargo test -p parser-common --locked
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p parser-doc -p import-pipeline --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused legacy DOC parser exact: exit 0.
- Focused import-pipeline legacy DOC exact: exit 0.
- `parser-doc`: exit 0; 2 tests passed.
- `parser-common`: exit 0; 7 tests passed.
- `import-pipeline`: exit 0; 6 tests passed.
- Focused parser/import clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.
- Private local-only witness using anonymized temporary PDF/DOCX/DOC copies:
  DOCX imported as searchable, text-layer/scanned PDF routed to OCR as expected,
  most legacy DOC samples became searchable through the local converter, and one
  DOC sample remained a safe permanent failure. No real resume path, filename,
  count, raw text, or diagnostic payload was committed or uploaded.

Scope note:

- S97 adds legacy `.doc` support through a local converter path. It does not
  finish converter packaging/distribution, Windows/Linux converter proof, full
  OCR completion for scanned PDFs, large-corpus proof, or full-library
  validation.
- Full product is still not complete.

### S96

Design target:

- Report local OCR runtime availability in `resume-cli doctor` and
  `resume-cli export-diagnostics --redact` without leaking binary paths,
  command output, language dumps, or resume data.
- Check `pdftoppm`, Tesseract, and the `eng` Tesseract language pack through
  local-only process inspection. Tests use temporary synthetic executables on
  `PATH`, not real resumes or network calls.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_ocr_runtime_without_paths_or_language_dump --locked -- --exact
```

Output summary:

- The test failed because doctor output did not contain
  `ocr renderer pdftoppm: available`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_ocr_runtime_without_paths_or_language_dump --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_reports_non_executable_ocr_tools_as_missing_without_paths --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused OCR runtime diagnostics exact: exit 0.
- Focused non-executable OCR runtime exact: exit 0.
- `s13_diagnostics`: exit 0; 11 tests passed, including redacted OCR runtime
  availability and non-executable tool handling without path or language-list
  leakage.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S96 reports local OCR runtime availability only. It does not implement final
  OCR/renderer distribution policy, non-English language-pack install/selection
  policy, real scanned-resume witness runs, large-corpus OCR throughput proof,
  or Windows/macOS validation.
- Full product is still not complete.

### S95

Design target:

- Persist an enum-only OCR job failure reason for scanned PDFs blocked by the
  local page budget. Do not persist raw worker stderr, local paths, commands,
  resume text, or OCR payloads as failure diagnostics.
- Surface aggregate remediation through `resume-cli status`, daemon status IPC,
  `resume-cli doctor`, and `resume-cli export-diagnostics --redact`.
- Keep over-budget documents non-searchable, avoid renderer/OCR invocation, and
  preserve the S94 no-partial-cache/no-partial-index behavior.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact
```

Output summary:

- The test failed after adding status/doctor/diagnostics expectations because
  `resume-cli status` did not report `ocr page budget blocked: 1`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite ocr_job_failure_kind_persists_reports_and_clears_on_retry_claim --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_can_read_redacted_daemon_status_over_loopback_ipc --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli --locked
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused meta-store, CLI, daemon, and IPC checks passed after implementation.
- `s3_sqlite`: exit 0; 43 tests passed, including schema v16, persisted
  `OcrPageBudgetExceeded`, aggregate blocked count, and clearing stale failure
  kind when the job is reclaimed.
- `s15_ocr_handoff`: exit 0; 12 tests passed, including local
  status/doctor/redacted diagnostics reporting the page-budget block without
  path, command, marker, or OCR payload leakage.
- `s50_ocr_worker`: exit 0; 8 tests passed, including daemon page-budget
  failure-kind persistence.
- `s20_status_ipc`: exit 0; 6 tests passed, including daemon status IPC
  rendering of the aggregate blocked count and remediation text.
- `cargo fmt --check`: exit 0.
- Focused clippy: exit 0.
- `s13_diagnostics`: exit 0; 9 tests passed after the diagnostics output
  changes.
- `s4_cli`: exit 0; 6 tests passed after the status output changes.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker guard: exit 0 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S95 adds redacted visibility/remediation for over-budget OCR documents. It
  does not implement real scanned-resume witness runs, large-corpus OCR
  throughput proof, final OCR/renderer distribution policy, non-English
  language-pack policy, or Windows/macOS validation.
- Full product is still not complete.

### S94

Design target:

- Add OCR page-count backpressure for scanned PDFs so a single oversized document
  cannot trigger unbounded local rendering/OCR work.
- Apply the guard to both `resume-cli ocr-worker` and `resume-daemon run
  --work-ocr*`; expose `--max-pages-per-document` on the CLI worker and
  `--ocr-max-pages-per-document` on the daemon. The macOS service install path
  can pass the daemon budget into LaunchAgent ProgramArguments.
- When a document exceeds the limit, do not invoke renderer/OCR, do not write
  partial OCR cache entries, do not index partial text, and keep paths/payloads
  out of output.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact
```

Output summary:

- CLI test failed because the worker did not recognize
  `--max-pages-per-document`, returning usage instead of the backpressure error.
- Daemon test failed because `resume-daemon run` did not recognize
  `--ocr-max-pages-per-document`, returning usage instead of reporting one OCR
  worker failure.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- `s15_ocr_handoff`: exit 0; 12 tests passed, including CLI OCR
  backpressure before renderer/OCR invocation.
- `s50_ocr_worker`: exit 0; 8 tests passed, including daemon OCR
  backpressure before renderer/OCR invocation.
- `s66_service_lifecycle`: exit 0; 4 tests passed, including LaunchAgent
  ProgramArguments carrying `--ocr-max-pages-per-document` without stdout path
  leakage.
- `cargo fmt --check`: exit 0.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker guard: exit 0 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S94 prevents over-budget OCR execution and partial indexing. S95 later adds
  redacted user-facing remediation diagnostics. Large-corpus OCR throughput
  proof, real scanned-resume witness runs, OCR/renderer distribution policy,
  non-English language-pack policy, and Windows/macOS validation remain not
  complete or BLOCKED.
- Full product is still not complete.

### S93

Design target:

- Persist OCR word bounding boxes from local Tesseract TSV output into the local
  OCR page cache without putting OCR payloads or file paths into debug/user
  output.
- Keep the existing custom OCR command protocol compatible with empty word boxes;
  only concrete OCR engines that return boxes populate the metadata.
- Prove the path with synthetic fixtures only: OCR client parses word boxes,
  meta-store round-trips redacted word-box cache metadata, and CLI/daemon
  Tesseract worker paths write boxes into cache before search indexing.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client tesseract_worker_recognizes_synthetic_image_without_payload_debug_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite ocr_page_cache_persists_word_boxes_without_debug_payload_leak --locked -- --exact
```

Output summary:

- `ocr-client` failed with exit 101 because `OcrPage::word_boxes()` did not
  exist.
- `meta-store` failed with exit 101 because `OcrWordBox`,
  `OcrPageCacheEntry::succeeded_with_word_boxes`, and
  `OcrPageCacheEntry::word_boxes()` did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p meta-store -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
if rg -n "schema_version\\(\\)\\.unwrap\\(\\), 14|\\[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14\\]" crates/meta-store/tests/s3_sqlite.rs; then exit 1; else echo "no stale schema version expectations"; fi
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- `ocr-client` test suite: exit 0; 17 tests passed, including real Tesseract
  recognition of a synthetic image and word-box parsing for the `S92` token.
- `meta-store` SQLite suite: exit 0; 42 tests passed, including schema V15 and
  OCR word-box cache round-trip with redacted Debug output.
- `s15_ocr_handoff`: exit 0; 11 tests passed, including CLI Tesseract worker
  cache word-box persistence and search indexing.
- `s50_ocr_worker`: exit 0; 7 tests passed, including daemon Tesseract worker
  cache word-box persistence and search indexing.
- `cargo fmt --check`: exit 0 after formatting.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- Schema expectation guard: exit 0 with no stale schema-version expectations.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker guard: exit 0 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S93 stores OCR word boxes locally in OCR cache rows and keeps them out of
  Debug. It does not add bbox-aware retrieval/ranking UI, final OCR distribution,
  non-English language-pack policy, real scanned-resume witness proof,
  large-corpus OCR throughput, or Windows/macOS validation.
- Full product is still not complete.

### S92

Design target:

- Install and validate a concrete local OCR recognition engine for English
  synthetic OCR witness runs. Homebrew installed `tesseract 5.5.2`; local
  `brew info --json=v2 tesseract` reports license `Apache-2.0`, and the
  installation includes a local LICENSE file.
- Add a Tesseract OCR client that writes private temp image input, runs
  `tesseract <image> stdout --psm 6 -l <lang> tsv`, parses TSV word text plus
  average confidence, and redacts payloads/paths from debug and user-visible
  output.
- Wire `resume-cli ocr-worker --tesseract-command` and
  `resume-daemon run --ocr-tesseract-command`, mutually exclusive with the
  existing custom OCR command protocol.
- Prove worker-level cache/search integration with synthetic images rendered
  by a local fixture and recognized by real Tesseract. Use synthetic fixtures
  only; do not scan real resumes.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client tesseract_worker_recognizes_synthetic_image_without_payload_debug_leaks -- --exact
```

Output summary:

- Exit 101 before implementation because `TesseractOcrClient` and
  `TesseractOcrSpec` were unresolved imports.

Additional wiring RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_uses_tesseract_for_rendered_image_before_indexing --locked -- --exact
```

Output summary:

- Exit 101 after adding the daemon integration test because the daemon startup
  guard still required `ocr_command` and did not accept `ocr_tesseract_command`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- `ocr-client` test suite: exit 0; 17 tests passed, including real Tesseract
  recognition of a synthetic image rendered in memory by the test.
- `s15_ocr_handoff`: exit 0; 11 tests passed, including CLI worker handoff
  through a rendered synthetic image into real Tesseract, OCR page cache, and
  full-text search without token/path leakage.
- `s50_ocr_worker`: exit 0; 7 tests passed, including daemon one-shot worker
  handoff through a rendered synthetic image into real Tesseract, OCR page
  cache, and full-text search without token/path leakage.
- `cargo fmt --check`: exit 0 after formatting.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S92 proves a real local Tesseract recognition engine path for English
  synthetic OCR. It does not claim final distribution packaging, non-English
  language packs, OCR bounding-box persistence, real scanned-resume witness
  proof, large-corpus OCR throughput, or Windows/macOS validation.
- Full product is still not complete.

### S91

Design target:

- Add a concrete local Poppler `pdftoppm` PDF page renderer adapter that writes
  private temp PDF input and private temp PPM output, bounds captured output,
  observes timeout/cancellation, and keeps payloads/paths out of debug and
  user-visible output.
- Wire the renderer through `resume-cli ocr-worker --pdftoppm-command` and
  `resume-daemon run --ocr-pdftoppm-command`, with mutual exclusion against the
  existing generic render-command path.
- Prove the path with valid synthetic PDF bytes rendered to PPM before the OCR
  command receives the page input. Install `poppler-utils` in PR CI so hosted
  tests exercise the real renderer instead of skipping for a missing binary.
- Use synthetic fixtures only; do not claim Tesseract or real OCR recognition
  engine completion from this slice.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client pdftoppm_renderer_renders_valid_pdf_page_to_ppm_without_payload_debug_leaks --locked -- --exact
```

Output summary:

- Exit 101 before implementation because `PdftoppmPdfRenderer` and
  `PdftoppmRenderSpec` were unresolved imports.

Additional wiring RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_uses_pdftoppm_renderer_for_valid_pdf_before_ocr --locked -- --exact
```

Output summary:

- Exit 101 after adding the daemon integration test because `RunOptions` did
  not yet have `ocr_pdftoppm_command`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- `ocr-client` test suite: exit 0; 16 tests passed, including the Poppler
  `pdftoppm` renderer witness that produced a `P6` PPM page from a valid
  synthetic PDF.
- `s15_ocr_handoff`: exit 0; 10 tests passed, including CLI worker handoff
  from `pdftoppm` PPM bytes to OCR command/cache/search without token/path
  leakage.
- `s50_ocr_worker`: exit 0; 6 tests passed, including daemon one-shot worker
  handoff from `pdftoppm` PPM bytes to OCR command/cache/search without
  token/path leakage.
- `cargo fmt --check`: exit 0 after formatting.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S91 proves the local Poppler renderer adapter and CLI/daemon worker wiring
  on valid synthetic PDFs when `pdftoppm` is installed. It does not install,
  select, or license-review a real OCR recognition engine; local OCR text
  recognition remains through the existing command protocol and synthetic test
  commands.
- It does not persist OCR bounding boxes, prove behavior on real resumes, prove
  large-corpus OCR throughput, define final renderer/OCR distribution policy,
  or validate Windows/macOS behavior.
- Full product is still not complete.

### S90

Design target:

- Extend `resume-cli purge --deleted` so tombstoned-document cleanup also
  removes current ingest jobs and OCR page-cache entries associated with the
  purged documents.
- Keep cache deletion content-hash scoped, but preserve shared OCR cache entries
  when the same content hash is still referenced by a visible document.
- Print only aggregate counts for the new purge surfaces; do not print OCR text,
  local paths, data directories, fixture roots, or command payloads.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact
```

Output summary:

- Exit 101 after tightening the purge test because stdout did not contain
  `ingest jobs purged: 1`, exposing that current purge output and cleanup did
  not cover OCR job/cache retention surfaces.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- Target RED/GREEN test: exit 0 after implementation.
- `s14_delete_search`: exit 0; 7 tests passed, including tombstoned metadata,
  full-text snapshots/staging, vector records, OCR job, and OCR page-cache
  cleanup without private text/path leakage.
- `meta-store`: exit 0; 41 tests passed plus doc-tests.
- Focused clippy: exit 0.
- `cargo fmt --check`: exit 0 after formatting.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S90 covers current OCR page-cache rows and ingest jobs for purged documents.
  It does not claim encrypted storage, forensic erase, future OCR bbox/table
  purge coverage, real-resume witness proof, large-corpus proof, or cross-
  platform validation.
- Full product is still not complete.

### S89

Design target:

- Add a local PDF page-render command protocol for scanned PDFs while keeping
  command paths, input paths, and OCR payloads out of user-visible output.
- Detect scanned PDF page count, render and OCR each page, persist per-page OCR
  cache entries, aggregate page text in order, and index one searchable OCR
  version with the correct page count.
- Wire the path through both `resume-cli ocr-worker --render-command` and
  `resume-daemon run --ocr-render-command`.
- Use synthetic PDF fixtures only; do not claim a concrete Poppler/PDFium/
  Tesseract integration or real resume witness from this slice.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_processes_all_scanned_pdf_pages_before_indexing --locked -- --exact
```

Output summary:

- Exit 101 before implementation because the OCR worker processed the scanned
  PDF as a single page, so the test did not observe two per-page OCR cache
  writes or two rendered page handoffs before indexing.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_processes_all_scanned_pdf_pages_before_indexing --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked
/Users/frankqdwang/.cargo/bin/cargo test -p parser-pdf --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p import-pipeline -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- Target RED/GREEN test: exit 0 after implementation.
- `ocr-client` test suite: exit 0; 15 tests passed, including render command
  page-byte handoff without debug payload leakage.
- `s15_ocr_handoff`: exit 0; 9 tests passed, including CLI multi-page OCR
  fan-out, per-page cache writes, page-count persistence, and searchability.
- `s50_ocr_worker`: exit 0; 5 tests passed, including daemon multi-page render
  and OCR fan-out.
- `cargo fmt --check`: exit 0.
- `import-pipeline`: exit 0; 5 tests passed plus doc-tests.
- `parser-pdf`: exit 0; 7 tests passed plus doc-tests.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S89 adds a local command protocol and tested multi-page fan-out path. It does
  not install or license-review a concrete renderer/OCR engine, persist OCR
  bounding boxes, prove behavior on real resumes, prove large-corpus OCR
  throughput, complete OCR cache/job retention purge, or validate Windows/
  macOS behavior.
- Full product is still not complete.

### S88

Design target:

- Add an explicit `resume-cli purge --deleted` command for local tombstoned
  document cleanup.
- Remove matching vectors from the persistent vector snapshot, rebuild the
  active full-text snapshot from visible metadata, delete obsolete full-text
  snapshots and staging directories, purge deleted rows from SQLite metadata,
  refresh candidate counts, and run WAL checkpoint plus `VACUUM`.
- Keep command output path-free and clear that the scope is local best-effort,
  not forensic erase or encrypted-storage proof.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact
```

Output summary:

- Exit 101 before implementation because `resume-cli purge` was not recognized
  as a command.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p index-fulltext -p index-vector -p meta-store --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- Target RED/GREEN test: exit 0 after implementation.
- `cargo fmt --check`: exit 0.
- `s14_delete_search`: exit 0; 7 tests passed, including explicit purge of
  tombstoned metadata, old full-text snapshots, and vector records without data
  directory or fixture path leakage.
- `index-fulltext`: exit 0; 12 tests passed.
- `index-vector`: exit 0; 6 tests passed.
- `meta-store`: exit 0; 42 tests passed across unit/integration/doc-test
  targets.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S88 adds best-effort local purge for documents already tombstoned by the
  product: vector records are physically removed from the vector snapshot, a
  clean full-text snapshot is rebuilt from visible metadata, old full-text
  snapshots/staging directories are removed, and deleted metadata rows are
  purged with SQLite checkpoint/VACUUM.
- It does not delete original user files, claim forensic erasure, encrypt local
  storage, purge every possible future PII surface such as OCR cache or
  queued-job retention, prove behavior on real resumes, or validate Windows/
  macOS filesystem semantics.
- Full product is still not complete.

### S87

Design target:

- Make full-text search with structured filters constrain recall by persisted
  field metadata before the full-text TopDocs cutoff.
- Keep the final profile filter as a correctness guard after hydration.
- Avoid any real resume data; use synthetic `.txt` files in a temporary local
  directory only.
- Do not claim field F1, dictionary completeness, or million-scale performance
  from this slice.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_fields_before_fulltext_top_k_cutoff --locked -- --exact
```

Output summary:

- Exit 101 before implementation because `resume-cli search needle
  --skills-any rust --top-k 1` returned `results: 0` when five high-scoring
  decoy documents occupied the unfiltered full-text TopDocs window and the
  lower-scoring Rust candidate was filtered out before it could be considered.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters --locked
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p index-fulltext -p meta-store --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- The RED test now passes and returns the field-matching synthetic Rust
  candidate even when `--top-k 1`.
- `s10_search_filters`: exit 0; 2 tests passed.
- `index-fulltext`: exit 0; 12 tests passed.
- `meta-store`: exit 0; 41 tests passed.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S87 adds indexed metadata prefiltering for current degree, skill, and years
  filters before full-text TopDocs retrieval. It does not prove field F1,
  complete dictionaries, million-scale latency, ANN quality, encrypted
  metadata, physical purge, or cross-platform release validation.
- Full product is still not complete.

### S86

Design target:

- Add a local-only model package manifest validation command:
  `resume-cli model validate-manifest --manifest <path>`.
- Validate schema `resume-ir.model-manifest.v1`, `model_pack_id`, non-empty
  `models[]`, per-model id/type/format, embedding `dim`, local artifact
  checksum, and `license.reviewed: true`.
- Keep outputs redacted: no manifest path, model artifact path, model bytes, or
  complete digest should be printed.
- Record that this is governance evidence only; it does not select, download,
  distribute, or quality-evaluate a real OCR/embedding model.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker model_manifest_validate --locked
```

Output summary:

- Exit 101 before implementation because `model` was not a supported top-level
  CLI command.
- After aligning the test with the production model-pack schema, the same
  command failed again because the initial implementation accepted only a
  single-model manifest and rejected `model_pack_id` plus `models[]` as an
  invalid manifest.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
./scripts/ci/check-runbooks.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- `cargo fmt --check`: exit 0.
- `s39_embedding_worker`: exit 0; 9 tests passed, including valid reviewed
  model-pack manifest, unreviewed-license rejection, checksum-mismatch
  rejection, existing local embedding worker, semantic, and hybrid search paths.
- `verify-local.sh` initially failed twice in
  `foreground_import_scheduler_processes_task_enqueued_after_startup` because
  the test helper inserted a queued import task before writing its scan scope,
  allowing the running daemon to claim the task and mark it failed under
  parallel test timing; the helper now uses the existing atomic
  `insert_import_task_with_scan_scope` API.
- `s4_daemon`: exit 0 after the stability repair; 10 tests passed.
- `resume-cli` clippy: exit 0.
- `check-runbooks.sh`: exit 0; worker and release runbooks now require
  `resume-cli model validate-manifest`.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S86 adds local governance for model-pack checksum and license-review evidence.
  It does not choose/download/distribute a real model, prove semantic/vector
  quality, implement ANN, prove production model performance, or complete model
  release approval.
- Full product is still not complete.

### S85

Design target:

- Add a local-only model checksum fault simulation for controlled model
  artifacts:
  `resume-cli fault-simulate --case model-checksum --model-file <path> --expected-sha256 <hex>`.
- Compute the actual SHA-256 locally, report match/mismatch as a safe
  reproduced/not-reproduced probe, and expose the hook in doctor plus redacted
  diagnostics.
- Keep outputs redacted: no model path, model bytes, full digest, or local data
  directory should be printed.
- Do not select, license, download, package, distribute, or validate a real
  production embedding/OCR model in this slice.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_model_checksum --locked
```

Output summary:

- Exit 101 before implementation because `fault-simulate` usage did not include
  `model-checksum`, and the CLI rejected the new checksum probe arguments.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
./scripts/ci/check-runbooks.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- `cargo fmt --check`: exit 0.
- `s71_fault_injection`: exit 0; 9 tests passed, including checksum mismatch
  and checksum match probes against synthetic local bytes.
- `s13_diagnostics`: exit 0; 9 tests passed, including redacted diagnostics
  advertising `model_checksum`.
- `resume-cli` clippy: exit 0.
- `check-runbooks.sh`: exit 0; the fault-injection runbook documents
  `resume-cli fault-simulate --case model-checksum`.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S85 adds a local checksum fault probe for a caller-supplied, controlled model
  artifact. It does not select/license/download/distribute a real model, prove
  semantic/vector quality, prove OCR/embedding model performance, or complete
  model package governance.
- Full product is still not complete.

### S84

Design target:

- Add a real benchmark policy gate for existing benchmark JSON artifacts, so a
  benchmark smoke can fail on insufficient sample size, P95 latency regression,
  zero-result regressions, or unproven million-scale claims.
- Wire the gate into PR benchmark smoke and nightly benchmark smoke workflows.
- Keep synthetic smoke explicitly scoped: `--allow-synthetic` is required, and a
  passing synthetic gate must not be treated as 100k/1M real-corpus evidence.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
```

Output summary:

- Exit 101 before implementation because the new RED tests imported missing
  symbols `evaluate_benchmark_gate_json` and `BenchmarkGateConfig`.
- The CLI RED tests also required `resume-benchmark gate --report <path>` to
  exist and reject synthetic artifacts unless `--allow-synthetic` is supplied.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings
tmpdir=$(mktemp -d); /Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- synthetic-query --index-dir "$tmpdir/index" --documents 24 --queries 6 --top-k 5 --json > "$tmpdir/benchmark-smoke.json" && /Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- gate --report "$tmpdir/benchmark-smoke.json" --allow-synthetic --min-documents 24 --min-queries 6 --max-p95-ms 1000 --max-zero-result-queries 0; rc=$?; rm -rf "$tmpdir"; exit $rc
./scripts/ci/check-runbooks.sh
git diff --check
./scripts/ci/verify-local.sh
```

Output summary:

- `cargo fmt --check`: exit 0.
- `benchmark-runner` tests: exit 0; 9 integration tests passed, including gate
  rejection for synthetic-without-allowance, latency regression, and unproven
  million-scale claims.
- `benchmark-runner` clippy: exit 0.
- CLI smoke: exit 0; `resume-benchmark gate` printed `benchmark gate passed`
  against a generated redacted synthetic report.
- `check-runbooks.sh`: exit 0; the release blocker runbook now documents
  `resume-benchmark gate`.
- `git diff --check`: exit 0.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S84 adds synthetic benchmark policy gates and workflow wiring. It does not run
  or claim 100k/1M real-corpus benchmarks, semantic/vector recall gates, OCR
  throughput gates, representative hardware runs, Windows/macOS benchmark
  evidence, or production P95 target compliance.
- Full product is still not complete.

### S83

Design target:

- Close the P6 runbook gap with production runbooks for diagnostics redaction,
  fault injection, OCR/embedding workers, and release blockers.
- Enforce local-only privacy language and required operational commands with a
  CI guard so runbooks cannot silently disappear from local or hosted checks.
- Keep this slice synthetic-fixture only; do not read, scan, upload, or transmit
  real resumes.

Observed RED:

```bash
sh scripts/ci/check-runbooks.sh
```

Output summary:

- Exit 1 before runbooks existed with `missing required runbook:
  docs/runbooks/diagnostics-redaction.md`.
- After the files were created, the same guard exposed missing canonical command
  strings for `resume-cli export-diagnostics --redact` and
  `resume-cli fault-simulate --case disk-space-low`; those checks were kept in
  the guard and the runbooks were corrected.

Implementation checks:

```bash
./scripts/ci/check-runbooks.sh
sh -n scripts/ci/check-runbooks.sh scripts/ci/verify-local.sh scripts/ci/guard-public-repo.sh scripts/ci/check-licenses.sh
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- `check-runbooks.sh`: exit 0; required runbook files, Local-only/Do not upload/
  Synthetic fixtures privacy language, diagnostics, fault-simulation, worker,
  and release-blocker command strings were present.
- `sh -n`: exit 0 for the runbook, verify-local, public guard, and license
  scripts.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0; public repo guard passed.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S83 adds documentation and CI policy coverage only. It does not perform real
  resume scanning, package signing/notarization, Windows/macOS release
  validation, real 100k/1M corpus benchmarks, destructive service-manager
  failure drills, or actual disk-exhaustion drills.
- Full product is still not complete.

### S82

Design target:

- Add a local-only `resume-cli fault-simulate --case ocr-crash
  --ocr-command <path>` probe that runs a configured local OCR command against
  synthetic page bytes, treats an engine crash as reproduced, and redacts command
  output, paths, and payload bytes.
- Add CLI and daemon OCR worker crash-recovery evidence: a crashing OCR command
  must leave the scanned document `OcrRequired`, keep the ingest job
  `FailedRetryable`, write a retryable OCR cache failure, and avoid leaking OCR
  stdout/stderr, command paths, data paths, or fixture roots.
- Expose `ocr_crash` in doctor/export diagnostics without weakening the privacy
  boundary. Do not read real resumes or run a real OCR engine.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_ocr_crash_reproduces_engine_failure_without_payload_or_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths -- --exact
```

Output summary:

- `s71_fault_injection`: exit 101 before implementation because
  `fault-simulate --case ocr-crash` returned the usage error.
- `s13_diagnostics`: exit 101 before implementation because diagnostics did
  not include `"ocr_crash"`.
- The CLI and daemon worker retryable-failure tests passed against existing
  worker failure semantics after being added, proving the current worker paths
  already preserved retryability and redaction for crashing OCR commands.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/verify-local.sh
```

Output summary:

- `resume-cli --test s71_fault_injection`: exit 0; 7 tests passed, including
  `fault_simulate_ocr_crash_reproduces_engine_failure_without_payload_or_path_leak`.
- `resume-cli --test s13_diagnostics`: exit 0; 9 tests passed.
- `resume-cli --test s15_ocr_handoff`: exit 0; 8 tests passed, including
  retryable CLI OCR worker command-crash handling.
- `resume-daemon --test s50_ocr_worker`: exit 0; 4 tests passed, including
  retryable daemon OCR worker command-crash handling.
- `cargo fmt --check`: exit 0.
- `cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D
  warnings`: exit 0.
- `git diff --check`: exit 0.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, and public repository guard passed.

Scope note:

- S82 safely exercises OCR command crash behavior using a controlled local
  command and synthetic bytes. It does not install or license a real OCR engine,
  render real PDF pages, crash a production service manager, simulate actual disk
  exhaustion, or prove Windows/macOS behavior.
- Full product is still not complete.

### S81

Design target:

- Add a local-only `resume-cli fault-simulate --case daemon-kill
  --daemon-binary <path>` probe that starts a configured daemon binary against a
  synthetic data directory, waits for readiness, terminates the controlled
  process, runs a same-directory `--once` restart check, and redacts paths.
- Add actual `resume-daemon` kill/restart integration evidence using the real
  daemon binary and a synthetic data directory.
- Expose `daemon_kill` in doctor/export diagnostics without weakening the
  privacy boundary. Do not kill user services or read real resumes.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_daemon_kill_restarts_configured_daemon_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths -- --exact
```

Output summary:

- `s71_fault_injection`: exit 101 before implementation because
  `fault-simulate --case daemon-kill` returned the usage error.
- `s13_diagnostics`: exit 101 before implementation because diagnostics did
  not include `"daemon_kill"`.
- The real daemon kill/restart integration test was added as production
  evidence and passed against existing daemon behavior, so no daemon production
  code change was required for restart health.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s81_daemon_kill --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/verify-local.sh
```

Output summary:

- `resume-cli --test s71_fault_injection`: exit 0; 6 tests passed, including
  `fault_simulate_daemon_kill_restarts_configured_daemon_without_path_leak`.
- `resume-cli --test s13_diagnostics`: exit 0; 9 tests passed.
- `resume-daemon --test s81_daemon_kill`: exit 0; the real foreground daemon
  was killed and restarted with the same synthetic data directory without path
  leakage.
- `cargo fmt --check`: exit 0.
- `cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D
  warnings`: exit 0.
- `git diff --check`: exit 0.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, and public repository guard passed.

Scope note:

- S81 safely exercises process kill/restart for a controlled daemon binary and
  synthetic data directory. It does not kill a user-installed service, simulate
  actual disk exhaustion, crash OCR workers, validate service managers, or prove
  Windows/macOS behavior.
- Full product is still not complete.

### S80

Design target:

- Add a real local file-lock contention probe to `resume-cli fault-simulate`
  without leaking paths or leaving probe files behind.
- Expose the new `file_lock` hook in doctor/export diagnostics.
- Keep the probe synthetic and local-only; do not scan or upload user data.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_file_lock_reproduces_contention_without_path_leak --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths --locked
```

Output summary:

- `s71_fault_injection`: exit 101 before implementation because
  `fault-simulate --case file-lock` returned the usage error.
- `s13_diagnostics`: exit 101 before implementation because diagnostics did
  not include `"file_lock"`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
./scripts/ci/verify-local.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `resume-cli --test s71_fault_injection`: exit 0; 5 tests passed, including
  `fault_simulate_file_lock_reproduces_contention_without_path_leak`.
- `resume-cli --test s13_diagnostics`: exit 0; 9 tests passed.
- `cargo clippy -p resume-cli --all-targets --locked -- -D warnings`: exit 0.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, and public repository guard passed.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.

Scope note:

- S80 exercises advisory file-lock contention against a local synthetic probe
  file. It does not implement destructive ENOSPC, daemon-kill, OCR-crash,
  model-checksum, battery-mode, or external-drive-disconnect fault injection.
- Full product is still not complete.

### S79

Design target:

- Add local resource telemetry to doctor and redacted diagnostics without
  reading resume files or printing local paths.
- Report data-volume disk total/available bytes, current-process memory bytes,
  and CPU core count.
- Keep `export-diagnostics --redact` valid JSON with resource paths explicitly
  redacted.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_redacted_resource_telemetry --locked
```

Output summary:

- Exit 101 before implementation; the new test failed because stdout did not
  contain `resource telemetry: available`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
./scripts/ci/verify-local.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `resume-cli --test s13_diagnostics`: exit 0; 9 tests passed, including the
  new resource telemetry test that parses redacted JSON and checks numeric
  telemetry fields.
- `cargo clippy -p resume-cli --all-targets --locked -- -D warnings`: exit 0.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, and public repository guard passed.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- PR #9 hosted checks after push: dependency tree pass 26s, license policy
  pass 20s, public repository guard pass 5s, rust workspace pass 1m3s.

Scope note:

- S79 reports local resource numbers only; it does not run real-resume witness
  scans, does not prove 100k/1M corpus performance, and does not implement
  destructive ENOSPC or kill-daemon fault injection.
- Full product is still not complete.

### S78

Design target:

- Apply the same portable Unix process-group signaling syntax to the local
  command embedder that S77 applied to OCR.
- Add an embedder regression test for descendant processes that keep stdout
  pipes open after a timeout.

Observed RED:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 10
gh run view 26864432512 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- `rust workspace` failed in GitHub Actions after 1m57s.
- OCR tests passed in the hosted run, including
  `local_command_worker_terminates_descendants_that_keep_output_pipes_open`.
- The hosted run then failed with exit 143 while running
  `tests/s51_embedding_worker.rs`.
- The embedder had the same `/bin/kill <signal> -PGID` process-group signaling
  form as the pre-S77 OCR client, so S78 updates it to
  `/bin/kill <signal> -- -PGID`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p embedder --test s11_embedder --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker --locked
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
./scripts/ci/verify-local.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `embedder --test s11_embedder`: exit 0; 7 tests passed, including the new
  inherited-pipe descendant timeout regression test.
- `resume-daemon --test s51_embedding_worker`: exit 0; 2 tests passed.
- `ocr-client --test s12_ocr_client`: exit 0; 14 tests passed.
- First `verify-local.sh` attempt failed at `cargo fmt --check` after the new
  test; `/Users/frankqdwang/.cargo/bin/cargo fmt --all` was run.
- Second `verify-local.sh`: exit 0; metadata, fmt, clippy, workspace tests,
  license check, and public repository guard passed.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0; public repo guard passed.
- Obsolete-reference marker scan: exit 1 with no matches.

Hosted checks:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir
```

Output summary:

- `dependency tree`: pass in 19s.
- `license policy`: pass in 17s.
- `public repository guard`: pass in 3s.
- `rust workspace`: pass in 2m8s.

Scope note:

- S78 fixes local embedding command process cleanup only. It does not package
  or validate a real embedding model.

### S77

Design target:

- Make OCR command process-group termination portable across macOS and Linux.
- Keep timeout/cancel error paths joining stdout/stderr readers after the worker
  process group has actually been terminated, so timeout returns before
  descendants close inherited pipes naturally.

Observed RED:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 10
gh run view 26864213730 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- `rust workspace` failed in GitHub Actions after 1m38s.
- The failing test was
  `local_command_worker_terminates_descendants_that_keep_output_pipes_open`.
- The assertion message was `timeout returned only after descendant closed
  inherited pipes`.
- The failure indicates the S76 Unix cleanup still did not signal the Linux
  process group; S77 changes `/bin/kill` calls to pass `--` before the negative
  process-group id and removes the unreliable direct-child helper.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
./scripts/ci/verify-local.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `ocr-client --test s12_ocr_client`: exit 0; 14 tests passed, including the
  inherited-pipe descendant timeout case.
- `resume-daemon --test s50_ocr_worker`: exit 0; 3 tests passed.
- `verify-local.sh`: exit 0; metadata, fmt, clippy, workspace tests, license
  check, and public repository guard passed.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0; public repo guard passed.
- Obsolete-reference marker scan: exit 1 with no matches.

Pending remote check:

- PR #9 hosted GitHub Actions checks after push

Scope note:

- S77 fixes process-group signal syntax for local OCR command cleanup only. It
  does not package or validate a real OCR engine.

### S76

Design target:

- Restore OCR timeout/cancel error-path output-reader cleanup so the client
  does not leave detached reader threads or background process side effects.
- Before the OCR shell exits, terminate its direct child processes and then the
  process group, so descendants that inherited stdout/stderr pipes do not hang
  reader joins.

Observed RED:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 10
gh run view 26863872803 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- `rust workspace` failed in GitHub Actions after the S75 push.
- The failed run exited with code 143 while running
  `tests/s50_ocr_worker.rs`, after
  `daemon_ocr_worker_once_respects_pause_without_claiming_or_invoking_command`
  had completed.
- S75 passed local verification but was not stable enough for Linux hosted CI.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
./scripts/ci/verify-local.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `ocr-client --test s12_ocr_client`: exit 0; 14 tests passed, including the
  inherited-pipe descendant timeout case.
- `resume-daemon --test s50_ocr_worker`: exit 0; 3 tests passed.
- `verify-local.sh`: exit 0; metadata, fmt, clippy, workspace tests, license
  check, and public repository guard passed.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0; public repo guard passed.
- Obsolete-reference marker scan: exit 1 with no matches.

Pending remote check:

- PR #9 hosted GitHub Actions checks after push

Scope note:

- S76 fixes local OCR command timeout cleanup stability only. It does not
  package or validate a real OCR engine.

### S75

Design target:

- OCR timeout/cancel/error paths should not wait for stdout/stderr reader
  threads after the worker process has already been terminated.
- Descendant processes that inherited output pipes must not delay the caller's
  timeout result.

Observed RED:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 10
gh run view 26863687741 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- `rust workspace` failed in GitHub Actions after 1m36s.
- The failing test remained `local_command_worker_terminates_descendants_that_keep_output_pipes_open`.
- The S74 process-group kill change passed local verification but still did not
  prevent the error path from waiting for inherited output pipes on Linux CI.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
./scripts/ci/verify-local.sh
```

Output summary:

- `ocr-client --test s12_ocr_client`: exit 0; 14 tests passed.
- `verify-local.sh`: exit 0; metadata, fmt, clippy, workspace tests, license
  check, and public repository guard passed.

Scope note:

- S75 fixed OCR command timeout return behavior locally, but later hosted CI
  failed with exit 143 while running daemon OCR worker tests. S76 supersedes it.

### S74

Design target:

- PR #9 `rust workspace` should not hang until OCR command descendants close
  inherited stdout/stderr pipes after a timeout.
- OCR fixture permission checks should use Linux GNU `stat` first and fall
  back to macOS `stat`.

Observed RED:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 10
gh run view 26863536781 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- `rust workspace` failed in GitHub Actions after 1m40s.
- The failing test was `local_command_worker_terminates_descendants_that_keep_output_pipes_open`.
- The failure message showed timeout cleanup returned only after a descendant
  closed inherited pipes.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
./scripts/ci/verify-local.sh
```

Output summary:

- `ocr-client --test s12_ocr_client`: exit 0; 14 tests passed.
- `verify-local.sh`: exit 0; metadata, fmt, clippy, workspace tests, license
  check, and public repository guard passed.

Scope note:

- S74 fixes local OCR command timeout cleanup and a Linux/macOS fixture
  portability issue. It does not package or validate a real OCR engine.

### S73

Design target:

- PR #9 required GitHub Actions should pass on Linux, not only local macOS.
- The embedder permission test should inspect owner-only temp input file
  permissions using portable `stat` invocation order.

Observed RED:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 10
gh run view 26863418606 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- `rust workspace` failed in GitHub Actions after 1m40s.
- The failing test was `local_command_embedder_times_out_and_keeps_input_file_private`.
- On Linux, `stat -f '%Lp'` returned filesystem information plus `600`
  instead of failing, so the assertion compared a multi-line string to `600`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p embedder --test s11_embedder --locked
```

Scope note:

- S73 changes only the synthetic test fixture command. It does not alter the
  product embedder protocol or claim Linux installer/release readiness.

### S72

Design target:

- `verify-local.sh` must be stable enough to gate public PR work.
- Local embedding command temp input directories must not collide when multiple
  embedding tests or worker requests run concurrently in the same process.

Observed RED:

```bash
./scripts/ci/verify-local.sh
```

Output summary:

- Exit 101 before the fix; `local_command_embedder_runs_configured_binary_and_parses_structured_vectors`
  failed with `EmbeddingError::EngineFailed` during the workspace test phase.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p embedder --test s11_embedder --locked
./scripts/ci/verify-local.sh
```

Output summary:

- `embedder --test s11_embedder`: exit 0; 6 tests passed, including the new
  parallel local-command request regression.
- `verify-local.sh`: exit 0; metadata, fmt, clippy, workspace tests, license
  check, and public repository guard passed.

Scope note:

- S72 only fixes local temp-directory uniqueness for the command embedder. It
  does not add a licensed model, ANN index, semantic quality proof, or
  OS-enforced network isolation for external embedding commands.

### S71

Design target:

- S71 closes the P6 gap where doctor/export listed fault simulation hooks but
  the CLI had no executable local fault-simulation entrypoint.
- `resume-cli fault-simulate --case disk-space-low` now safely reproduces a
  low-space budget condition without filling the real disk, or writes and
  removes a bounded probe when the configured available budget is sufficient.
- `resume-cli fault-simulate --case permission-denied` now attempts a redacted
  local write probe and reports permission denial without printing paths.
- Doctor/export diagnostics now include the permission-denied probe hook.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
```

Output summary:

- Exit 101 before implementation; all four S71 tests failed because
  `resume-cli fault-simulate` did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
```

Output summary:

- `resume-cli --test s71_fault_injection`: exit 0; 4 tests passed, covering
  disk-space-low reproduction without probe writes, bounded probe write cleanup
  when the budget is sufficient, permission-denied reproduction, and usage
  errors without path leaks.
- `cargo fmt --all`: exit 0.
- `resume-cli --test s13_diagnostics`: exit 0; 8 tests passed.
- `cargo clippy -p resume-cli --all-targets --locked -- -D warnings`: exit 0.

Scope note:

- S71 is a safe local simulation/probe slice. It does not fill the actual disk,
  does not claim real ENOSPC coverage, does not implement advisory/mandatory
  file-lock behavior, and does not cover kill-daemon or OCR crash injection.
- Full product is still not complete.

### S68

Design target:

- S68 fixes the GitHub configuration script after a real public-repository
  configuration attempt exposed an invalid `gh repo edit` option for personal
  public repositories.
- The failed run happened after guard and push and before branch protection; no
  branch protection settings were applied by the failed command.

Checks and remote operations:

```bash
./scripts/ci/configure-github-repo.sh FrankQDWang resume-ir
sh -n scripts/ci/configure-github-repo.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- First `configure-github-repo.sh`: exit 1 after `public repo guard passed`,
  `Everything up-to-date`, and `branch 'main' set up to track 'origin/main'`;
  `gh repo edit` returned `HTTP 422` because the forking option is only valid
  for org-owned private repositories.
- Removed the invalid repo edit option.
- `sh -n scripts/ci/configure-github-repo.sh`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.

Scope note:

- Branch protection is rerun after this fix is committed and pushed.
- Full product is still not complete.

### S67

Design target:

- S67 unblocks the public GitHub setup now that `FrankQDWang` keyring auth is
  available outside the sandbox.
- The public repository was created only after the local public-repository guard
  passed; no local data directory, token file, diagnostic bundle, index, model
  cache, or real resume was committed or uploaded.
- The GitHub configuration script fallback remote now uses HTTPS, matching the
  selected Git protocol.

Checks and remote operations:

```bash
gh repo view FrankQDWang/resume-ir
gh repo create FrankQDWang/resume-ir --public --source=. --remote=origin --description "Local-first resume search engine" --disable-wiki
git remote -v
./scripts/ci/guard-public-repo.sh
git rev-parse main
git push -u origin main
sh -n scripts/ci/configure-github-repo.sh
git diff --check
```

Output summary:

- `gh repo view FrankQDWang/resume-ir`: exit 1 before creation; repository did
  not exist.
- `gh repo create ...`: exit 0 and returned
  `https://github.com/FrankQDWang/resume-ir`.
- `git remote -v`: origin is `https://github.com/FrankQDWang/resume-ir.git`.
- `./scripts/ci/guard-public-repo.sh`: exit 0.
- `git rev-parse main`: `cc009da12c7c5753bbf3e66642fccee7db2ebeae`.
- `git push -u origin main`: exit 0; new remote branch `main` was pushed and
  set as upstream.
- `sh -n scripts/ci/configure-github-repo.sh`: exit 0 after the HTTPS fallback
  fix.
- `git diff --check`: exit 0.

Scope note:

- S67 does not prove hosted GitHub Actions results, does not create a release,
  and does not package/sign/notarize the app. Branch protection is executed
  after this commit is pushed so that the progress/script-fix commit is not
  blocked by protection.
- Full product is still not complete.

### S66

Design target:

- S66 closes the local P5 gap where the daemon could run in foreground but the
  CLI had no service lifecycle entrypoint.
- The CLI now supports `resume-cli service install|uninstall|status|start|stop`.
  Install writes a macOS user LaunchAgent plist with `ProgramArguments` for
  `resume-daemon --data-dir <local> run --foreground --work-imports
  --work-index --ipc-listen 127.0.0.1:0`, preserves user data on uninstall, and
  keeps CLI stdout/stderr path-redacted.
- Optional OCR and embedding worker command flags can be included in the
  generated plist, but no concrete engine/model is bundled by this slice.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked
```

Output summary:

- Exit 101 before implementation; all four S66 tests failed because
  `resume-cli service` did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace --locked
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
gh auth status
```

Output summary:

- `resume-cli --test s66_service_lifecycle`: exit 0; 4 tests passed, covering
  LaunchAgent plist install, XML escaping, redacted install/status/uninstall
  output, user-data preservation, start/stop dry-run output, and invalid label
  rejection without path leaks.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `resume-cli --test s4_cli`: exit 0; 6 tests passed.
- `resume-cli --test s20_status_ipc`: exit 0; 6 tests passed.
- `resume-daemon --test s4_daemon`: exit 0; 10 tests passed.
- `resume-daemon --test s20_ipc`: exit 0; 18 tests passed.
- `cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: exit 0.
- `cargo test --workspace --locked`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- Sandboxed `gh auth status`: exit 1 with stale invalid credential. Escalated
  `gh auth status`: exit 0; `FrankQDWang` is logged in from keyring with
  `repo` and `workflow` scopes.

Scope note:

- S66 does not create a signed macOS pkg/dmg, does not notarize, does not build
  Windows MSI/service registration, does not run real `launchctl` start/stop
  against the user's login session, does not execute hosted GitHub Actions, and
  does not prove cross-platform install/upgrade/uninstall.
- Full product is still not complete.

### S65

Design target:

- S65 prepares the repository for public GitHub hosting without uploading real
  resumes, local data directories, daemon tokens, diagnostic bundles, logs,
  indexes, or model caches.
- The repository now has MIT licensing, CODEOWNERS, contribution and security
  policies, PR and issue templates, GitHub Actions workflow definitions,
  Dependabot configuration, AI coding harness instructions, local license
  checking, local public-repository guardrails, and a GitHub configuration
  script for repo creation, first push, and branch protection.
- Workspace crate metadata now uses `MIT` while keeping `publish = false`.

Implementation checks:

```bash
sh -n scripts/ci/guard-public-repo.sh scripts/ci/check-licenses.sh scripts/ci/verify-local.sh scripts/ci/configure-github-repo.sh
git diff --check
/Users/frankqdwang/.cargo/bin/cargo metadata --no-deps --locked --format-version 1
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- synthetic-query --documents 24 --queries 6 --top-k 5 --json
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace --locked
./scripts/ci/check-licenses.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
gh auth status
```

Output summary:

- Script syntax check: exit 0.
- `git diff --check`: exit 0.
- `cargo metadata --no-deps --locked --format-version 1`: exit 0 and shows
  workspace crates licensed as MIT.
- `cargo fmt --check`: exit 0.
- Synthetic benchmark smoke: exit 0 and emitted redacted synthetic JSON with
  no paths or raw resume text.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`:
  exit 0.
- `cargo test --workspace --locked`: exit 0.
- `./scripts/ci/check-licenses.sh`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `gh auth status`: exit 1; the active `FrankQDWang` token is invalid. Remote
  GitHub repository creation, initial push, PR creation, and branch protection
  configuration are therefore BLOCKED until re-authentication.

Scope note:

- S65 does not prove GitHub Actions execution on hosted runners, does not create
  the public remote repository, does not push a branch, and does not configure
  branch protection because the local GitHub CLI credential is invalid. It also
  does not implement service lifecycle, release packaging, signing,
  notarization, token rotation/revocation, real whole-machine witness runs, or
  Windows/macOS validation.

### S64

Design target:

- S64 closes the P0 gap where full-text index rebuild/repair existed only as a
  local CLI operation. The daemon now accepts `--work-index-once` to force a
  full-text snapshot rebuild from persisted local metadata and `--work-index`
  to repair non-ready snapshot roots inside the long-running worker loop.
- The full-text index worker is separate from embedding `UpdateIndex` jobs. It
  does not claim or repurpose embedding job queues.
- Because the product is not yet shipped, `--work-index` treats legacy
  root-layout indexes as unhealthy and rebuilds the published snapshot layout
  rather than preserving backward-compatible read behavior.
- Worker output reports only rebuild state and indexed document count. It does
  not print data directories, import roots, file paths, token material, raw
  resume text, or local query contents.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon foreground_once_index_worker_rebuilds_missing_full_text_snapshot_without_path_leak --test s4_daemon
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon foreground_index_worker_loop_repairs_missing_snapshot_once_per_health_change --test s4_daemon
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon foreground_index_worker_loop_rebuilds_legacy_root_snapshot_layout --test s4_daemon
```

Output summary:

- The one-shot worker test failed before implementation because
  `--work-index-once` was not parsed and daemon usage was returned.
- The loop worker test failed before implementation because `--work-index` was
  not parsed and daemon usage was returned.
- The legacy-root regression test failed before the predicate fix because the
  loop treated a `Ready` legacy root as healthy and skipped rebuild.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-daemon --test s4_daemon`: exit 0; 10 tests passed.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  exit 0.
- `cargo test --workspace`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.

Scope note:

- S64 does not implement queued incremental index jobs, snapshot GC/retention,
  vector or ANN index maintenance, singleton service lifecycle, CI, CODEOWNERS,
  token rotation/revocation, real whole-machine witness runs, Windows/macOS
  validation, or packaging/signing. Those remain incomplete or externally
  blocked.

### S63

Design target:

- S63 closes the P0 control-plane gap where import progress was visible only by
  polling status. The daemon now advertises an `import_progress` endpoint in
  its local endpoint manifest and serves authenticated newline-delimited JSON
  progress events over loopback IPC.
- The progress stream events reuse the same redacted import scan snapshot fields
  as status and never include requested roots, canonical roots, token material,
  raw resume text, or local data directory paths.
- `resume-cli status --watch-import --ipc auto` now discovers the daemon,
  validates the status endpoint, reads the local daemon token file, subscribes
  to the progress stream, and renders each progress event. Because the product
  is not yet shipped, explicit watch mode requires the real `/imports/progress`
  endpoint rather than accepting `/status` as a compatibility alias.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_streams_redacted_import_progress_over_loopback_ipc -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_watch_import_ipc_auto_streams_redacted_progress_without_local_store -- --exact
```

Output summary:

- The daemon test failed before implementation because `/imports/progress` did
  not return `200 OK`.
- The CLI test failed before implementation because `status --watch-import`
  was not parsed and did not connect to the fake daemon.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-cli --test s20_status_ipc`: exit 0; 6 tests passed.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 18 tests passed.
- `cargo test -p resume-cli`: exit 0.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S63 does not implement daemon index-maintenance workers, singleton service
  lifecycle, CI, CODEOWNERS, token rotation/revocation, real whole-machine
  witness runs, Windows/macOS validation, or packaging/signing. Those remain
  incomplete or externally blocked.

### S62

Design target:

- S62 closes the P0 user-control gap where import cancellation markers and
  running-task cooperative checks existed, but a user could not request import
  cancellation through the daemon command IPC control plane.
- The daemon endpoint manifest now advertises a redacted `import_cancel`
  endpoint. Authenticated `POST /imports/cancel` accepts a task id, validates
  the task state, records the cancellation marker, and returns a response that
  does not include root paths, token material, or raw store diagnostics.
- `resume-cli cancel import` keeps the existing local store path and now also
  supports explicit cancel IPC plus `--ipc auto` endpoint discovery with the
  shared local daemon token file.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_import_cancel_command_records_cancellation_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc cancel_import_ipc_submits_authenticated_request_without_touching_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc cancel_import_ipc_auto_discovers_endpoint_and_token_file -- --exact
```

Output summary:

- The daemon test failed before implementation because `/imports/cancel` did
  not return `202 Accepted`.
- The CLI tests failed before implementation because `cancel import` did not
  parse IPC options and did not connect to the fake daemon.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-cli --test s47_import_ipc`: exit 0; 10 tests passed.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 17 tests passed.
- `cargo test -p resume-cli`: exit 0.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S62 does not implement a dedicated progress stream, token rotation/
  revocation, singleton service lifecycle, real whole-machine witness runs,
  Windows/macOS validation, or release packaging/signing. Those remain
  incomplete or externally blocked.

### S61

Design target:

- S61 closes the local status visibility gap where import scan scopes were
  initialized and finalized but did not receive pipeline-owned progress updates
  while import work advanced.
- `import-pipeline` now updates an existing `ImportScanScope` after scan error
  persistence, periodically during per-file processing, after deletion
  propagation, during searchable document finalization, and after final index
  state update. The update path is a no-op when no scope exists, preserving older
  direct import callers.
- `resume-cli status` now prints latest import progress counters without root
  paths. Daemon `/status` now includes a `latest_import_scan` object with the
  same redacted counters, and CLI IPC status renders it.
- This is status-pollable live progress, not a dedicated push/SSE progress
  stream.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline tests::import_root_updates_existing_scan_scope_progress_without_daemon_postprocessing -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli status_reports_latest_import_scan_progress_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_can_read_redacted_daemon_status_over_loopback_ipc -- --exact
```

Output summary:

- `import-pipeline` failed before implementation because the existing scan
  scope still had `files_discovered = 0` after import completed without daemon
  post-processing.
- Local CLI status failed before implementation because it did not print latest
  import progress counters.
- IPC status rendering failed before implementation because CLI ignored
  daemon-provided latest import progress fields.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p import-pipeline`: exit 0; 5 unit tests passed.
- `cargo test -p resume-cli`: exit 0.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S61 does not implement a dedicated progress stream, cancel-over-IPC UX, token
  rotation/revocation, singleton service lifecycle, real whole-machine witness
  runs, or Windows/macOS validation. Those remain incomplete or externally
  blocked.

### S60

Design target:

- S60 closes the P0 control-plane gap where cancellation markers existed for
  queued/retryable import tasks but a task already marked `Running` could not be
  cancelled cooperatively.
- `MetaStore::cancel_import_task` now records cancellation markers for running
  import tasks. Cancelled running tasks are excluded from root de-duplication,
  worker recovery, queued/recoverable status counts, and worker claims through
  the existing marker checks.
- `fs-crawler` now exposes explicit scan control with cancellation checks during
  directory traversal and fingerprinting. Cancellation returns a redacted
  cancellation error instead of a path-bearing scan error.
- `import-pipeline` now checks the cancellation marker before scan, during scan,
  before per-file work, around expensive parse/index steps, before deletion
  propagation, before snapshot publish, and before final index-state updates.
  A cancelled import transitions out of `Running` to retryable failure while the
  marker keeps it out of retry/recovery queues.
- The daemon import worker now counts a cooperatively cancelled import as
  cancelled in its summary rather than generic failed work.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store running_import_task_cancellation_is_recorded_and_removed_from_recovery -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler scan_control_cancels_directory_walk_without_path_leakage -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline import_root_stops_running_task_when_cancellation_marker_exists -- --exact
```

Output summary:

- `meta-store` failed before implementation because running-task cancellation
  returned `InvalidTransition`.
- `fs-crawler` failed before implementation because `ScanControl`,
  `crawl_with_fs_options_and_control`, and cancellation error variants did not
  exist.
- `import-pipeline` failed before implementation because there was no
  `Cancelled` import error path; after the first implementation pass it also
  exposed a timestamp boundary where finish time could be earlier than the
  cancellation marker. The final test uses the full module path and passed with
  one executed test.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p fs-crawler`: exit 0; 10 tests passed.
- `cargo test -p meta-store`: exit 0; 41 integration tests plus identity passed.
- `cargo test -p import-pipeline`: exit 0; 4 unit tests passed.
- `cargo test -p resume-daemon --test s4_daemon`: exit 0; 7 tests passed.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 16 tests passed.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 17 tests passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S60 does not implement live import progress streaming, cancel-over-IPC UX,
  token rotation/revocation, singleton service lifecycle, real whole-machine
  witness runs, or Windows/macOS validation. Those remain incomplete or
  externally blocked.

### S59

Design target:

- S59 closes the P0 control-plane gap where users had to copy a printed
  daemon IPC URL and separately locate the local command token file.
- The daemon now writes a local `ipc.endpoints.json` manifest after a loopback
  IPC bind succeeds. The manifest includes only status/import/search/detail
  loopback URLs and schema version; it does not include the token, token path,
  data directory, query text, roots, or resume text.
- CLI `status`, `import`, `search`, and `detail` now accept `--ipc auto`.
  Status reads only the manifest. Command endpoints read the manifest and then
  use `data-dir/ipc.auth` locally for bearer-token authentication.
- Auto command endpoints perform an unauthenticated `/status` liveness probe
  before sending any bearer token or private request body, and the daemon removes
  the manifest on normal IPC shutdown. Manifest writes reject symlink/non-file
  destinations and publish through an owner-only temporary file.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_ipc_auto_discovers_endpoint_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc import_ipc_auto_discovers_endpoint_and_token_file -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_auto_discovers_endpoint_and_token_file -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_auto_rejects_stale_manifest_without_sending_token_or_query -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_ipc detail_ipc_auto_discovers_endpoint_and_token_file -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_serves_redacted_status_over_loopback_ipc -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_rejects_symlinked_ipc_endpoint_manifest_without_clobbering_target -- --exact
```

Output summary:

- Before implementation, the CLI focused tests failed because `--ipc auto` did
  not connect to the fake daemon.
- Before implementation, the daemon focused test failed because no endpoint
  discovery manifest existed after IPC bind.
- Reviewer-driven RED checks then failed because auto command endpoints did not
  probe status before sending command payloads, normal IPC shutdown did not
  remove the manifest, and symlinked manifests were not rejected.
- After implementation, the focused tests passed and proved auto-discovered
  status/import/search/detail IPC, stale-manifest rejection before token/query
  send, manifest cleanup, symlink rejection, and manifest redaction.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-cli --test s20_status_ipc`: exit 0; 5 tests passed.
- `cargo test -p resume-cli --test s47_import_ipc`: exit 0; 8 tests passed.
- `cargo test -p resume-cli --test s48_search_ipc`: exit 0; 7 tests passed.
- `cargo test -p resume-cli --test s49_detail_ipc`: exit 0; 4 tests passed.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 16 tests passed.
- `cargo test -p resume-cli`: exit 0.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S59 does not implement live progress streaming, cooperative cancellation of
  already-running import scans, token rotation/revocation, singleton service
  lifecycle enforcement, real whole-machine witness runs, or Windows/macOS
  validation. Those remain incomplete or externally blocked.

### S58

Design target:

- S58 closes the P2 gap where the domain and metadata model already supported
  `EntityType::Name` but rules never produced a name mention.
- Added high-confidence name extraction for explicit `Name:`/localized labels
  and conservative resume-heading candidates. The rule rejects section headers,
  contact lines, school/company lines, and known title lines to reduce false
  positives.
- Import now maps `FieldType::Name` to `EntityType::Name`, so extracted names
  are persisted with evidence, confidence, and extractor metadata through the
  existing entity mention path. The S57 synthetic TXT import test now asserts
  the persisted name mention.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_candidate_name_from_labeled_line_and_heading_with_evidence -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_txt_resume_builds_searchable_index_without_path_leakage -- --exact
```

Output summary:

- Before implementation, the focused extractor test failed to compile because
  `FieldType::Name` did not exist.
- Before implementation, the focused CLI import test failed because no
  persisted `EntityType::Name` mention existed for the synthetic TXT resume.
- After implementation, the focused tests prove labeled and heading name
  extraction, debug redaction of name text, and import-time persistence of the
  synthetic name mention.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p extractor-rules`: exit 0; 9 tests passed, covering name
  extraction, false-positive avoidance, existing contact/date/education/company/
  title/skill/certificate extraction, and debug redaction.
- `cargo test -p import-pipeline`: exit 0; 2 tests passed.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 17 tests passed,
  including TXT import/search plus persisted name mention.
- `cargo test -p resume-cli`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S58 does not implement broad name dictionaries, multilingual name
  normalization, name-based soft-dedupe scoring, labeled field F1 metrics,
  encrypted local storage, or physical purge. Those remain incomplete.

### S57

Design target:

- S57 closes the P1 gap where `.txt` files were discovered by the crawler but
  failed permanently in import because no text parser was connected.
- Added a production `parser-text` crate for UTF-8, UTF-8 BOM, and BOM-marked
  UTF-16 text with parser-level budget support. Parser debug/error formatting
  does not expose raw text bytes.
- Import now routes `FileExtension::Txt` through the parser, then uses the
  existing normalizer, extractor, candidate assignment, and full-text snapshot
  path. CLI import/search tests cover a synthetic TXT resume without leaking
  temporary root paths or contact values in search output.
- TXT import has a pre-read byte cap and treats blank text as a failed document
  instead of enqueueing OCR, because OCR is not a valid recovery path for a
  plaintext file.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_txt_resume_builds_searchable_index_without_path_leakage -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_blank_txt_resume_fails_without_queueing_ocr -- --exact
```

Output summary:

- Before implementation, the focused CLI test failed because import stdout did
  not contain `searchable documents: 1`; the file was discovered but not
  parsed into a searchable document.
- Before the blank-TXT fix, the focused CLI test failed because import stdout
  did not contain `ocr required documents: 0`; blank TXT was incorrectly routed
  to the OCR queue.
- After implementation, the focused test proves a synthetic TXT resume imports
  as searchable, search can find it, blank TXT does not enqueue OCR, and output
  redacts the temp root and contact value.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p parser-text
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p parser-text`: exit 0; 8 tests passed, covering UTF-8,
  UTF-16LE/BE with BOM, unsupported-extension rejection, invalid UTF-8/UTF-16
  redaction, and parser byte-budget enforcement.
- `cargo test -p import-pipeline`: exit 0; 2 tests passed, preserving existing
  discovery deletion behavior with the TXT parser dependency wired in.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 17 tests passed.
  Coverage includes the TXT import/search loop, blank TXT non-OCR behavior,
  and existing import/search regressions.
- `cargo test -p resume-cli`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S57 does not implement legacy `.doc` parsing, broad non-BOM encoding
  detection, file streaming beyond the current pre-read cap, watcher/background
  incremental import, production-grade PDF coverage, large-corpus proof, or
  incremental index updates. Those remain incomplete.

### S56

Design target:

- S56 adds a production control-plane cancellation path for import tasks that
  have not started running yet. The metadata store now has a V14
  `import_task_cancellation` table, a task-id cancellation API, status summary
  counts, and claim/pending queries that exclude cancelled tasks.
- `resume-cli cancel import --task-id <id>` records cancellation without
  printing roots or paths. Local status and daemon status IPC include
  `import tasks cancelled`.
- Daemon import workers do not need a separate skip branch because cancelled
  tasks are no longer claimable. Daemon import command IPC can enqueue a new
  task for a root whose previous queued task was cancelled.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store cancelled_import_tasks_are_not_claimed_or_reported_as_queued -- --exact
```

Output summary:

- Before implementation, the focused metadata test failed to compile because
  `cancel_import_task`, `is_import_task_cancelled`, and
  `import_tasks_cancelled` did not exist.
- After implementation, metadata tests prove queued and retryable cancelled
  import tasks are not returned by pending lookup, are not claimed by workers,
  and are not counted as queued/recoverable.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
git diff --check
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p meta-store`: exit 0; 40 tests passed, including V14
  migration, queued/retryable cancellation, claim exclusion, and status counts.
- `cargo test -p resume-cli --test s20_status_ipc`: exit 0; 4 tests passed,
  including cancelled-count rendering from daemon status IPC.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 15 tests passed,
  including task-id cancellation without running import or leaking paths.
- `cargo test -p resume-cli`: exit 0.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 15 tests passed,
  including cancelled-count daemon status IPC and requeue after cancellation.
- `cargo test -p resume-daemon --test s4_daemon`: exit 0; 7 tests passed,
  including worker skip of a cancelled queued task.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S56 does not implement cooperative cancellation for an import task already
  inside a running scanner/import pipeline. That still needs cancellation-token
  plumbing through crawler, per-file processing, parser/index phases, and
  partial-write semantics. Live import progress streaming also remains
  incomplete.

### S55

Design target:

- S55 closes the product gap where embedding workers only generated one vector
  per resume version. CLI and daemon workers now keep the document-level vector
  for compatibility and additionally expand sectionizer output into
  `version:section:n` local embedding inputs.
- Section vector identity is stored only in the vector id. Vector `doc_id`
  remains the document id, so existing semantic hit hydration, candidate
  folding, and hybrid RRF behavior stay unchanged.
- Daemon durable jobs remain per version/model/dimension. A single claimed
  version job writes the document vector plus any section vectors, then
  completes once.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker semantic_and_hybrid_search_can_rank_section_vectors_over_document_vectors -- --exact
```

Output summary:

- Before implementation, semantic top-1 returned the synthetic document whose
  document-level vector was closer, because no section vector existed for the
  actual section match.
- After implementation, semantic and hybrid top-1 return the synthetic
  section-match document while redacting the query and local temp paths.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
git diff --check
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-cli --test s39_embedding_worker`: exit 0; 6 tests
  passed. Coverage includes section vectors outranking a document vector in
  semantic and hybrid modes, model-scoped search isolation, and local command
  snapshot persistence without path leakage.
- `cargo test -p resume-daemon --test s51_embedding_worker`: exit 0; 2 tests
  passed, preserving one-shot and looped daemon embedding worker behavior.
- `cargo test -p resume-daemon --test s52_embedding_jobs`: exit 0; 4 tests
  passed. Coverage includes one version job writing multiple vectors and
  completing once, restart skip, and model-change re-embedding.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S55 does not choose or distribute a licensed model, add ANN/HNSW/FAISS, run
  semantic quality metrics, prove production-scale vector performance, enforce
  OS-level no-network sandboxing for configured commands, or validate
  Windows/macOS command behavior. Those remain incomplete or BLOCKED.

### S54

Design target:

- S54 fixes semantic/hybrid query isolation after embedding model changes. The
  persistent vector snapshot now writes v2 vector records with optional model id
  metadata while still reading existing v1 snapshots.
- `VectorIndex::knn_for_model` filters by explicit stored model id and falls
  back to the legacy vector-id prefix for old snapshots. Unscoped `knn` remains
  available for existing callers.
- CLI and daemon embedding workers now write model metadata with each vector,
  and CLI semantic search uses the requested model id when searching the vector
  snapshot. Hybrid search inherits the same protection through its semantic
  channel.
- During workspace verification, two daemon IPC long-poll tests exposed a
  request-budget race. Their test request budget now has headroom, with no
  production daemon behavior change.
- The workspace run also reproduced an existing CLI import-IPC closed-port
  race. That test now uses a deterministic local dropped-response fixture
  instead of relying on an unused port remaining unused.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector persistent_vector_index_filters_knn_by_model_scope_after_reopen -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker semantic_search_uses_only_vectors_for_requested_model -- --exact
```

Output summary:

- Before implementation, both tests failed to compile because
  `VectorDocument::new_for_model` and `knn_for_model` did not exist.
- After implementation, the vector-index test proves a reopened snapshot can
  exclude a higher-scoring old-model vector. The CLI test proves both semantic
  and hybrid mode return only the requested model's vector result even when an
  old-model vector would otherwise win top-1.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
git diff --check
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p index-vector`: exit 0; 6 tests passed. Coverage includes v2
  model-scoped snapshot persistence after reopen and legacy v1 model-prefix
  fallback.
- `cargo test -p resume-cli --test s39_embedding_worker`: exit 0; 5 tests
  passed. Coverage includes CLI semantic and hybrid model-scoped vector search.
- `cargo test -p resume-cli --test s47_import_ipc`: exit 0; 7 tests passed.
  Coverage includes deterministic import-IPC transport failure without local
  store fallback.
- `cargo test -p resume-daemon --test s51_embedding_worker`: exit 0; 2 tests
  passed, preserving daemon vector snapshot writes with the new format.
- `cargo test -p resume-daemon --test s52_embedding_jobs`: exit 0; 3 tests
  passed, preserving durable model/dimension-scoped embedding job behavior.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 14 tests passed after
  increasing long-poll test request budget headroom.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S54 does not choose or distribute a licensed model, add ANN/HNSW/FAISS, create
  section vectors, run semantic quality metrics, prove production-scale vector
  performance, enforce OS-level no-network sandboxing for configured commands,
  or validate Windows/macOS command behavior. Those remain incomplete or
  BLOCKED.

### S53

Design target:

- S53 fixes daemon embedding job invalidation when the configured model id or
  dimension changes. The metadata store now has a v13 `embedding_job_spec`
  table and scopes durable embedding jobs by `resume_version_id`, model id, and
  dimension.
- The daemon now enqueues and claims embedding jobs only for the active
  model/dimension pair, so completed jobs for one model no longer suppress
  embedding work for a different model or dimension.
- This slice still requires a user-provided local command, model id, and
  dimension. It does not choose, bundle, download, license, or claim a
  production embedding model.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store embedding_update_jobs_are_scoped_by_model_and_dimension -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs daemon_embedding_worker_once_reembeds_completed_jobs_for_new_model -- --exact
```

Output summary:

- Before implementation, the meta-store test failed to compile because
  `enqueue_embedding_job_for_resume_version` and `claim_next_embedding_job`
  did not accept model id or dimension.
- Before implementation, the daemon model-change test failed because the second
  run with a different model did not process the completed versions again.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p meta-store`: exit 0; 39 tests passed. Coverage includes v13
  migration, model/dimension-scoped embedding job idempotence, and
  model/dimension-filtered embedding-job claim.
- `cargo test -p resume-daemon --test s52_embedding_jobs`: exit 0; 3 tests
  passed. Coverage includes restart skip for the same model and re-embedding
  when the model id changes.
- `cargo test -p resume-daemon --test s51_embedding_worker`: exit 0; 2 tests
  passed, preserving daemon embedding worker behavior.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S53 does not choose or distribute a licensed model, add ANN/HNSW/FAISS, add
  model-scoped vector query isolation, create section vectors, run semantic
  quality metrics, prove production-scale vector performance, enforce OS-level
  no-network sandboxing for configured commands, or validate Windows/macOS
  command behavior. Those remain incomplete or BLOCKED.

### S52

Design target:

- S52 makes daemon embedding work durable at the resume-version level. The
  metadata store now persists idempotent `UpdateIndex` jobs with
  `resume_version_id`, exposes a dedicated embedding-job claim path that skips
  unrelated index jobs, and reports `embedding_queue_depth` from queued durable
  jobs instead of document lifecycle state.
- The daemon embedding worker now enqueues missing version jobs, claims durable
  embedding jobs, marks successful jobs completed, marks failed command/vector
  writes retryable, and skips completed version jobs after daemon restart.
- This slice still requires a user-provided local command, model id, and
  dimension. It does not select, bundle, download, license, or claim a
  production embedding model.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store embedding_update_jobs_are_durable_idempotent_and_claimable_by_resume_version -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs daemon_embedding_worker_once_skips_completed_jobs_after_restart -- --exact
```

Output summary:

- Before implementation, the meta-store test failed to compile because
  `MetaStore` had no version-level embedding job enqueue API.
- Before implementation, the daemon restart test failed because the second run
  still invoked the local embedding command and did not print
  `embedding worker processed: 0`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p meta-store`: exit 0; 38 tests passed. Coverage includes the
  new v12 migration, durable/idempotent version embedding jobs, dedicated
  embedding-job claim filtering, and status summary queue-depth aggregation from
  durable jobs.
- `cargo test -p resume-daemon --test s52_embedding_jobs`: exit 0; 2 tests
  passed. Coverage includes persisted completed embedding jobs and no repeated
  local embedding command invocation after daemon restart.
- `cargo test -p resume-daemon --test s51_embedding_worker`: exit 0; 2 tests
  passed, preserving daemon local embedding command execution and status IPC
  behavior.
- `cargo test -p resume-cli --test s39_embedding_worker`: exit 0; 4 tests
  passed, preserving CLI local embedding worker and semantic/hybrid search.
- `cargo test -p resume-daemon`: exit 0; daemon identity, IPC, import, OCR,
  S51, and S52 tests passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S52 does not choose or distribute a licensed model, add ANN/HNSW/FAISS,
  invalidate completed jobs when the embedding model/dimension changes, create
  section vectors, run semantic quality metrics, prove production-scale vector
  performance, enforce OS-level no-network sandboxing for configured commands,
  or validate Windows/macOS command behavior. Those remain incomplete or
  BLOCKED.

### S51

Design target:

- S51 moves local embedding execution into the daemon control plane. A daemon
  can now run `--work-embeddings-once` or a long-running `--work-embeddings`
  loop, execute an explicitly configured local embedding command, persist the
  vector snapshot, and keep serving status IPC while embedding work runs.
- This slice still requires a user-provided local command, model id, and
  dimension. It does not select, bundle, download, license, or claim a
  production embedding model.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker daemon_embedding_worker_once_runs_local_command_and_persists_vector_snapshot -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker daemon_embedding_worker_loop_serves_status_ipc_while_persisting_vectors -- --exact
```

Output summary:

- Before implementation, the one-shot daemon embedding test failed because
  `resume-daemon run` rejected `--work-embeddings-once` as usage.
- Before implementation, the loop daemon embedding test failed because the
  daemon rejected `--work-embeddings` and exited before printing an IPC
  endpoint.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-daemon --test s51_embedding_worker`: exit 0; 2 tests
  passed. Coverage includes one-shot daemon embedding command execution,
  vector snapshot persistence, no stdout leakage of paths or embedded text, and
  a worker loop serving status IPC while persisting vectors.
- `cargo test -p resume-cli --test s39_embedding_worker`: exit 0; 4 tests
  passed, preserving CLI local embedding worker and semantic/hybrid search
  behavior.
- `cargo test -p resume-daemon`: exit 0; daemon identity, status/search/detail
  IPC, import worker, OCR worker, and S51 embedding worker tests passed.
- Focused daemon clippy passed.

Scope note:

- S51 does not choose or distribute a licensed model, add ANN/HNSW/FAISS,
  persist durable per-version embedding job state, create section vectors, run
  semantic quality metrics, prove production-scale vector performance, enforce
  OS-level no-network sandboxing for configured commands, or validate
  Windows/macOS command behavior. Those remain incomplete or BLOCKED.

### S50

Design target:

- S50 moves OCR execution into the daemon control plane. A daemon can now run
  `--work-ocr-once` or a long-running `--work-ocr` loop, claim durable
  `OcrDocument` jobs, honor the persistent OCR pause flag, execute a configured
  local OCR command, persist the page cache, index successful OCR text, and
  keep serving status IPC while the OCR worker loop runs.
- The daemon summary output reports counts only. It does not print source
  document paths, data-dir paths, OCR command paths, or OCR text.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_executes_local_command_and_indexes_scanned_pdf -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_loop_serves_status_ipc_while_indexing_scanned_pdf -- --exact
```

Output summary:

- Before implementation, the one-shot daemon OCR test failed because
  `resume-daemon run` rejected `--work-ocr-once` as usage.
- Before implementation, the loop daemon OCR test failed because the daemon
  rejected `--work-ocr` and exited before printing an IPC endpoint.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-daemon --test s50_ocr_worker`: exit 0; 3 tests passed.
  Coverage includes one-shot daemon OCR command execution, cache persistence,
  searchable OCR text indexing, persistent pause preventing job claim/command
  invocation, no stdout leakage of paths or OCR text, and an OCR worker loop
  serving status IPC while processing a queued OCR job.
- `cargo test -p resume-cli --test s15_ocr_handoff`: exit 0; 7 OCR handoff
  tests passed, preserving CLI OCR worker command/cache/pause/resume behavior.
- `cargo test -p resume-daemon`: exit 0; daemon identity, status/search/detail
  IPC, import worker, combined IPC-worker, and S50 OCR worker tests passed.
- `cargo clippy -p resume-daemon --all-targets -- -D warnings`,
  `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt
  --check`, `git diff --check`, and `cargo test --workspace` all passed.
- The obsolete-reference marker scan returned no matches.

Scope note:

- S50 does not render real PDF pages for OCR, split multi-page scanned PDFs,
  persist OCR bounding boxes, choose/install/license an OCR engine, implement
  OCR backpressure, encrypt/purge OCR text, run a real scanned-resume witness,
  add daemon embedding execution, or validate Windows/macOS service and process
  behavior. Those remain incomplete or BLOCKED.

### S49

Design target:

- S49 adds local `resume-cli detail --doc-id <doc_id>` and authenticated
  loopback `resume-cli detail --ipc ... --ipc-token-file ...` against daemon
  `POST /details`.
- Detail output returns redacted structured fields and a short redacted snippet
  for the latest non-hidden resume version. It does not return source URI,
  normalized path, raw full text, tokens, or local private paths.
- CLI IPC mode validates the success protocol, checks the returned doc id
  against the request, validates enum-like strings before printing them, and
  does not fall back to opening the local store when IPC fails.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_cli detail_local_prints_redacted_fields_and_short_snippet_without_private_paths -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_ipc detail_ipc_submits_authenticated_request_and_renders_redacted_detail_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s49_detail_ipc daemon_detail_ipc_authenticates_and_returns_redacted_structured_detail -- --exact
```

Output summary:

- The local CLI red test failed before implementation because `resume-cli`
  did not recognize the `detail` command.
- The CLI IPC red test failed before implementation because it never connected
  to the fake daemon listener.
- The daemon red test failed before implementation because `/details` returned
  a non-200 response.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s49_detail_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p meta-store -p resume-cli -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo test -p resume-cli --test s49_detail_cli`: exit 0; 2 tests passed.
  Coverage includes local redacted details, latest non-hidden version
  selection, deleted-document hiding, contact/path redaction, and no full
  raw-text leakage.
- `cargo test -p resume-cli --test s49_detail_ipc`: exit 0; 3 tests passed.
  Coverage includes successful authenticated request rendering, no local-store
  fallback, HTTP error behavior, invalid token/non-loopback rejection, malformed
  success protocol rejection, response doc-id matching, enum validation, and no
  token/path/contact leakage.
- `cargo test -p resume-daemon --test s49_detail_ipc`: exit 0; 3 tests passed.
  Coverage includes bearer authentication, redacted structured details, latest
  version selection, deleted-document hiding, invalid JSON/doc-id rejection, and
  not-found responses without sensitive values.
- `cargo test -p index-fulltext`: exit 0; 12 tests passed, including common
  local path redaction.
- `cargo test -p meta-store`: exit 0; 37 tests passed, including latest
  visible resume-version selection.
- `cargo test -p resume-cli`, `cargo test -p resume-daemon`, `cargo fmt
  --check`, `git diff --check`, focused clippy, workspace clippy, and
  `cargo test --workspace` passed.
- The obsolete-reference marker scan was re-run and returned no matches.

Sub-agent review:

- Two read-only Codex sub-agents reviewed the S49 diff. Medium findings around
  path-like detail redaction, unstable version selection, and loose CLI IPC
  protocol validation were fixed before commit and covered by tests. A low
  duplication note remains accepted for this slice because the CLI and daemon
  are separate binaries and no shared protocol crate exists yet.

Scope note:

- S49 does not add daemon endpoint discovery UX, semantic/hybrid daemon search
  IPC, token rotation/revocation, progress streaming, singleton service
  lifecycle enforcement, daemon OCR/vector workers, real whole-machine witness
  scans, or macOS/Windows service validation. Those remain incomplete.

### S48

Design target:

- S48 adds an authenticated loopback daemon search command IPC endpoint and CLI
  `resume-cli search --ipc ... --ipc-token-file ...` mode. This lets a local
  caller query the daemon's persistent full-text index without opening the
  metadata database or index from the CLI process.
- The new endpoint is bearer-token protected with the existing daemon IPC token,
  rejects non-loopback CLI targets, returns static redacted errors, validates the
  response protocol on the CLI, and redacts contact values in file names and
  snippets before rendering or returning results.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_authenticates_filters_and_redacts_results -- --exact
```

Output summary:

- The CLI red test failed before implementation because `resume-cli search` did
  not recognize the IPC flags and never connected to the fake daemon listener.
- The daemon red test failed before implementation because the daemon did not
  have search IPC support and lacked the full-text search dependency.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc -- --test-threads=1
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p rank-fusion -p resume-cli -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo test -p resume-cli --test s48_search_ipc`: exit 0; 5 tests passed.
  Coverage includes successful authenticated request rendering, HTTP error
  no-fallback behavior, invalid success protocol rejection, invalid JSON,
  malformed response, non-loopback rejection, invalid token rejection, wrong
  path rejection, connect failure, local-store no-fallback, and query/token/path
  redaction.
- `cargo test -p resume-daemon --test s48_search_ipc`: exit 0; 4 tests passed.
  Coverage includes authenticated full-text search with degree/skill/years
  filters, result contact redaction, missing/wrong bearer token, invalid JSON,
  empty query, unsupported mode, malformed filters, not-ready index response,
  and no query/token/path leakage.
- `cargo test -p resume-daemon --test s20_ipc -- --test-threads=1`: exit 0; 14
  tests passed after confirming import/status IPC compatibility.
- `cargo test -p resume-cli`, `cargo test -p resume-daemon`,
  `cargo test -p rank-fusion`, `cargo fmt --check`, `git diff --check`, and the
  focused clippy command passed.
- Workspace clippy and `cargo test --workspace` passed after the final S48
  changes; all workspace tests and doc-tests completed with 0 failures.
- The obsolete-reference marker scan was re-run and returned no matches.

Sub-agent review:

- A read-only Codex sub-agent review found no high or medium security/privacy
  regressions. Its low-risk findings were addressed before commit by tightening
  CLI response schema validation and adding daemon/CLI negative tests for
  malformed protocol, invalid JSON, unsupported modes, malformed filters, wrong
  path, and not-ready index behavior.

Scope note:

- S48 does not add detail IPC endpoints, daemon endpoint discovery UX,
  semantic/hybrid daemon search IPC, token rotation/revocation, import/search
  progress streaming, singleton service lifecycle enforcement, daemon OCR/vector
  workers, real whole-machine witness scans, or macOS/Windows service
  validation. Those remain incomplete.

### S47

Design target:

- S47 wires the S46 authenticated daemon import command IPC into the CLI. A
  local caller can now run `resume-cli import --ipc ... --ipc-token-file ...`
  to submit explicit roots to the daemon without opening or writing the
  metadata store directly.
- IPC mode remains loopback-only, reads the bearer token from a caller-supplied
  local token file, sends only the import command payload to `/imports`, and
  keeps stdout/stderr free of token values and local paths.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc import_ipc_submits_authenticated_request_without_touching_local_store -- --exact
```

Output summary:

- Failed before implementation because `resume-cli import` did not recognize
  the IPC flags and never connected to the fake daemon listener.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc import_ipc_submits_authenticated_request_without_touching_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_import_command_preserves_local_discovery_preset_scope -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- The successful IPC import test passed: CLI sends `POST /imports`, includes
  the bearer token in the `Authorization` header, serializes roots/profile/file
  budget as JSON, renders the daemon `202 Accepted` response as queued import
  output, omits root path/token path/token content from stdout, and does not
  create the local `--data-dir`.
- `cargo test -p resume-cli --test s47_import_ipc`: exit 0; 7 tests passed.
  The suite covers success, local-discovery preset preservation, HTTP error,
  invalid JSON, connect failure, malformed response, non-loopback rejection,
  missing token file, invalid token content, no local-store fallback, and
  token/path redaction.
- The daemon focused preset test passed and verified that import command IPC
  persists local-discovery scan scope metadata with `Preset` root kind.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 14 tests passed.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed, including existing
  local import/search/status/OCR/embedding behavior.
- `cargo fmt --check`, `git diff --check`, workspace clippy, and
  `cargo test --workspace` passed.
- The obsolete-reference marker scan was re-run and returned no matches.

Sub-agent orchestration:

- A separate Codex sub-agent review was spawned after local verification to
  inspect the S47 diff for token/path leakage, endpoint parsing, no-fallback
  guarantees, and compatibility with existing import/status commands. Review
  findings were fixed before commit: IPC mode now preserves root preset
  semantics, validates the daemon token shape before composing headers, reads
  fake-daemon request bodies by `Content-Length`, and tests connect-failure,
  malformed-response, and invalid-JSON no-fallback paths.

Scope note:

- S47 does not add search/detail IPC endpoints, daemon endpoint discovery UX,
  token rotation/revocation, import cancellation/progress streaming, singleton
  service lifecycle enforcement, daemon OCR/vector workers, real whole-machine
  witness scans, or macOS/Windows service validation. Those remain incomplete.

### S0

```bash
git init
git add GOAL.md MANIFEST.md 01_system_design_系统设计 02_execution_plan_执行方案 docs
git commit -m "docs: commit initial design baseline"
```

Output summary:

- Initialized empty Git repository.
- Created root commit `43e3d1c` with 25 design and execution planning files.

```bash
git status --short
git log --oneline -3
```

Output summary:

- `git status --short`: `.gitignore`, `PROGRESS.md`, and `README.md` were the only untracked files before the S0 commit.
- `git log --oneline -3`: `43e3d1c docs: commit initial design baseline`.

### S1

Baseline red check:

```bash
cargo metadata --no-deps
```

Output summary:

- Failed before implementation with `could not find Cargo.toml`.

TDD checks:

```bash
cargo test
cargo test -p resume-daemon --test identity
```

Output summary:

- First test run failed because `core-domain`, `config`, and `meta-store` did not expose `crate_name()`.
- After adding library identities, `resume-cli --identity` failed because the binary produced no stdout.
- After adding the CLI identity output, `resume-daemon --identity` failed because the binary produced no stdout.

Acceptance:

```bash
cargo metadata --no-deps
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo metadata --no-deps`: exit 0; workspace contains `core-domain`, `config`, `meta-store`, `resume-daemon`, and `resume-cli` with edition 2021. Cargo emitted the expected compatibility warning about omitting `--format-version`.
- `cargo fmt --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; 5 identity tests passed, plus crate unit/doc test harnesses with 0 failures.

### S2

TDD red checks:

```bash
cargo test -p core-domain
cargo test -p config
```

Output summary:

- `core-domain` failed before implementation because the S2 domain ID, model, and error types were unresolved imports in the new behavior tests.
- `config` failed before implementation because `Profile` and `ProfileDefaults` were unresolved imports in the new behavior tests.

Review-fix red check:

```bash
cargo test -p core-domain
```

Output summary:

- Failed before the review-fix implementation because tests required design-aligned model fields, full document lifecycle states, the exact layered `ErrorKind` list, validated ID hydration, the golden opaque ID string, and the `ContactHash` privacy boundary.

Acceptance:

```bash
cargo fmt --check
cargo test -p core-domain
cargo test -p config
cargo clippy -p core-domain -p config --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `core-domain`: exit 0; identity test plus 7 S2 tests passed, covering design-aligned fields, full lifecycle states, exact error kinds, diagnostic redaction, redacted domain debug output, validated ID hydration, golden opaque ID generation, and `ContactHash` hydration.
- `config`: exit 0; identity test plus 2 S2 tests passed, covering default Balanced profile and deterministic Economy/Balanced/Turbo resource tiers.
- `cargo clippy -p core-domain -p config --all-targets -- -D warnings`: exit 0.

### S3

Baseline check:

```bash
cargo test -p meta-store
```

Output summary:

- Passed before S3 work with only the existing meta-store identity test.

TDD red check:

```bash
cargo test -p meta-store
```

Output summary:

- Failed before implementation because the S3 tests imported missing SQLite store APIs, migration reporting, task queue types, and index state persistence types.

Implementation check:

```bash
cargo test -p meta-store
```

Output summary:

- First implementation run passed migration idempotency, visible-document filtering, and index-state persistence tests, then failed the recovery query because the internal job query SQL was malformed.
- After fixing the query template and adding the file-backed open path, exit 0; identity plus 5 S3 tests passed.

Acceptance:

```bash
cargo fmt --check
cargo test -p meta-store
cargo clippy -p meta-store --all-targets -- -D warnings
```

Output summary:

- Initial `cargo fmt --check` reported formatting diffs; after `cargo fmt`, `cargo fmt --check` exited 0.
- `cargo test -p meta-store`: exit 0; identity test plus 5 S3 tests passed, covering migration idempotency/schema version/table existence, hidden deleted documents, recovery query filtering, job status update, resume version persistence, index state upsert/query, and file-backed SQLite reopen behavior.
- `cargo clippy -p meta-store --all-targets -- -D warnings`: exit 0.

Review-fix:

```bash
cargo test -p core-domain
cargo test -p meta-store
cargo fmt --check
cargo clippy -p core-domain -p meta-store --all-targets -- -D warnings
```

Output summary:

- Red checks failed before the review fix because `ContactHash` display exposed the full digest and `meta-store` lacked claim-next-job, job lookup, and file-backed PRAGMA APIs.
- After the review fix, `core-domain` tests passed with `ContactHash` display redacted while `.as_str()` still exposes explicit persistence material.
- After the review fix, `meta-store` tests passed with 12 S3 integration tests covering queue/recovery separation, atomic claim semantics, timestamp transitions, invalid transition errors, schema CHECK constraints, file-backed PRAGMA setup, FK rejection/cascade, file-backed reopen recovery, and SQLite metadata/task persistence.
- This remains plaintext SQLite metadata/task persistence only; no SQLCipher or production data encryption claim is made.

### S4

Baseline red checks:

```bash
cargo run -p resume-cli -- status
cargo run -p resume-cli -- import --root tests/fixtures/empty
cargo run -p resume-cli -- search "Java"
```

Output summary:

- Before S4 implementation, all three commands exited 2 with `resume-cli: no commands are implemented in S1`.

Implementation checks:

```bash
cargo fmt --check
cargo test -p meta-store
cargo test -p resume-cli
cargo test -p resume-daemon
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p meta-store`: exit 0; identity plus 16 SQLite tests passed, including import-task persistence without document FK, import-task lifecycle constraints, status aggregation, schema v2 idempotency, v1-to-v2 upgrade, CHECK constraints, recovery queries, and file-backed reopen behavior.
- `cargo test -p resume-cli`: exit 0; identity plus 3 S4 CLI tests passed, covering status, import-root task submission, no path leak, unavailable search without metadata writes, and no query echo for unavailable search.
- `cargo test -p resume-daemon`: exit 0; identity plus foreground-once lifecycle test passed.

Acceptance:

```bash
cargo run -p resume-cli -- status
cargo run -p resume-cli -- import --root tests/fixtures/empty
cargo run -p resume-cli -- search "Java"
cargo run -p resume-daemon -- run --foreground --once
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `resume-cli status`: exit 0; opened the local metadata store, ran migrations, and printed real aggregate counts plus `search index: unavailable (S4 skeleton: no full-text or vector backend)`.
- `resume-cli import --root tests/fixtures/empty`: exit 0; submitted a persistent `imp_...` import task without creating document or resume rows.
- `resume-cli search "Java"`: exit 0; returned `search index not available yet` and `results: 0`, with no fake result rows.
- `resume-daemon run --foreground --once`: exit 0; opened the metadata store, ran migrations, reported foreground readiness, and exited.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S4 is only a control-plane slice. It does not complete product search, full-text indexing, OCR, embeddings, local IPC, diagnostics, packaging, or cross-platform verification.

### S5

TDD red check:

```bash
cargo test -p fs-crawler
```

Output summary:

- Failed before implementation because `fs-crawler` lacked the S5 scanning, path normalization, filtering, fingerprinting, fake filesystem, and error classification APIs required by the new behavior tests.

Implementation checks:

```bash
cargo test -p fs-crawler
cargo fmt --check
cargo clippy -p fs-crawler --all-targets -- -D warnings
```

Output summary:

- Initial implementation test run surfaced test-side type and borrow errors; after fixing the tests, `cargo test -p fs-crawler` passed with 1 identity test and 6 S5 tests.
- Initial `cargo fmt --check` reported formatting diffs; after `cargo fmt`, `cargo fmt --check` exited 0.
- Initial `cargo clippy -p fs-crawler --all-targets -- -D warnings` reported two sort helpers that should use `sort_by_key`; after updating them, clippy exited 0.

Coverage summary:

- Tests cover Chinese paths, deterministic mixed separator, drive-relative, and UNC normalization, non-UTF-8 path rejection without lossy replacement, same-name files under different normalized paths, temporary/hidden directory/hidden file/unsupported filtering, bounded head/tail quick fingerprint sampling with redacted display/debug, and deterministic fake-filesystem simulation for permission denied, source unavailable, and locked/unreadable states.

Scope note:

- S5 is only a file discovery slice. It does not perform product import execution, document parsing, full-text/vector indexing, OCR, or search-query closure.

### S6

TDD red checks:

```bash
cargo test -p parser-common
cargo test -p parser-docx
cargo test -p parser-pdf
```

Output summary:

- `parser-common` failed before implementation because the parser trait, probe/input/output, budget, support level, and parser error mapping APIs were missing.
- `parser-docx` failed before implementation because `DocxParser` and the shared parser APIs were missing.
- `parser-pdf` failed before implementation because `PdfParser`, shared parser APIs, and the dev test dependency on `core-domain` were missing.

Implementation checks:

```bash
cargo test -p parser-common
cargo test -p parser-docx
cargo test -p parser-pdf
```

Output summary:

- `cargo test -p parser-common`: exit 0; 7 S6 tests passed, covering file probes, support ordering, zero and nonzero timeout mapping, corrupted/OCR_REQUIRED parser error mapping, and redacted parse output debug.
- `cargo test -p parser-docx`: exit 0; 6 S6 tests passed, covering synthetic zip+xml `.docx` paragraph extraction, XML entity unescape, corrupted archive handling, missing `word/document.xml` handling, input byte budget enforcement, and excessive zip entry rejection.
- `cargo test -p parser-pdf`: exit 0; 7 S6 tests passed, covering synthetic text-layer PDF extraction/status, scanned/image PDF `ParseStatus::OcrRequired`, corrupted PDF handling, input byte budget enforcement, runtime timeout enforcement for text-layer and no-text-layer scans, deadline-aware PDF scans, and redacted parse output debug.

Acceptance:

```bash
cargo fmt --check
cargo test -p parser-common
cargo test -p parser-docx
cargo test -p parser-pdf
cargo clippy -p parser-common -p parser-docx -p parser-pdf --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0 after formatting.
- `cargo test -p parser-common`: exit 0; 7 tests passed.
- `cargo test -p parser-docx`: exit 0; 6 tests passed.
- `cargo test -p parser-pdf`: exit 0; 7 tests passed.
- `cargo clippy -p parser-common -p parser-docx -p parser-pdf --all-targets -- -D warnings`: exit 0.

Scope note:

- S6 is only the parser skeleton/docx/PDF text-layer slice. It does not implement OCR execution, indexing, full-text search, text cleaning, extraction, or S7+ behavior.

Additional workspace regression:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Output summary:

- `cargo test --workspace`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.

### S10

TDD red checks:

```bash
cargo test -p extractor-rules --test s10_fields
cargo test -p rank-fusion
cargo test -p resume-cli --test s10_search_filters
```

Output summary:

- `extractor-rules --test s10_fields` failed before implementation because the S10 field variants and extraction behavior were not present.
- `rank-fusion` failed before implementation because the filter, fusion, and candidate-fold APIs were not present.
- `resume-cli --test s10_search_filters` failed before implementation because `resume-cli search` only accepted a bare query and rejected field-filter arguments.

Implementation checks:

```bash
cargo fmt --check
cargo test -p extractor-rules
cargo test -p rank-fusion
cargo test -p resume-cli --test s10_search_filters
cargo test -p resume-cli --test s9_import_search
```

Output summary:

- `cargo fmt --check`: exit 0 after formatting.
- `cargo test -p extractor-rules`: exit 0; S7 coverage plus 2 S10 tests passed, covering school, degree, skill, date-range-derived years, field confidence, original evidence offsets, and Debug redaction.
- `cargo test -p rank-fusion`: exit 0; 4 S10 tests passed, covering degree/skill/year filters, case-insensitive skill matching, candidate fold skeleton, and reciprocal-rank fusion.
- `cargo test -p resume-cli --test s10_search_filters`: exit 0; filtered synthetic search passed for degree, top-k, lower-case skill, and years-experience filters without query-label echo.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; S9 import/search behavior still passed after fixture enrichment.

Acceptance:

```bash
cargo test -p extractor-rules
cargo test -p rank-fusion
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run -p resume-cli -- import --root tests/fixtures/resumes
cargo run -p resume-cli -- search "Java" --degree bachelor --top-k 20
```

Output summary:

- `cargo test -p extractor-rules`: exit 0.
- `cargo test -p rank-fusion`: exit 0.
- `cargo fmt --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.
- `resume-cli import --root tests/fixtures/resumes`: exit 0; completed an import task for 3 synthetic files, with 2 searchable documents, 1 OCR-required document, 0 failed documents, and 0 scan errors.
- `resume-cli search "Java" --degree bachelor --top-k 20`: exit 0; returned 2 synthetic results, `synthetic-java-platform.pdf` and `synthetic-java-engineer.docx`.

Scope note:

- S10 implements MVP field filtering by overfetching full-text results and filtering in memory. It is not a persistent field index and can miss matches outside the overfetch window.
- Candidate soft dedupe is a pure `rank-fusion` skeleton and is not yet wired into CLI search output.
- S10 does not run OCR, generate embeddings, claim production-scale filtering, or package/release the app.

### S11

TDD red checks:

```bash
cargo test -p embedder
cargo test -p index-vector
cargo test -p rank-fusion
```

Output summary:

- `embedder` failed before implementation because `Embedder`, `EmbeddingInput`, `EmbeddingBudget`, and `DeterministicTestEmbedder` were unresolved.
- `index-vector` failed before implementation because `VectorIndex`, `InMemoryVectorIndex`, `VectorDocument`, and `QueryVector` were unresolved.
- `rank-fusion` failed before implementation because the typed hybrid RRF APIs were unresolved.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p embedder
cargo test -p index-vector
cargo test -p rank-fusion
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p embedder`: exit 0; 2 S11 tests passed, covering the `Embedder` trait, deterministic local test embedder stability, budget rejection, vector dimensions, and text/value Debug redaction.
- `cargo test -p index-vector`: exit 0; 2 S11 tests passed, covering the `VectorIndex` trait, in-memory cosine KNN, deletion marks, snapshots, dimension checks, and vector Debug redaction.
- `cargo test -p rank-fusion`: exit 0; S10 tests plus 2 S11 hybrid RRF tests passed, covering full-text/vector channel fusion, scale-independent RRF, and candidate-key preservation for later folding.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Review notes:

- Sub-agent review confirmed the slice should remain a skeleton only: no model download, no CLI/import pipeline wiring, and no semantic-quality claim.
- The deterministic embedder is explicitly documented as a lexical hash test vectorizer, not a licensed semantic model.

Scope note:

- S11 adds local interfaces and test scaffolding only. It does not download or bundle embedding models, persist vector indexes, wire semantic search into the CLI, or claim production vector-search latency/recall.

### S12

TDD red checks:

```bash
cargo test -p ocr-client
cargo test -p ingest-scheduler
```

Output summary:

- `ocr-client` failed before implementation because the OCR worker client, cache key, rendered page, page request, budget, cancellation, page result, and disabled client APIs were unresolved.
- `ingest-scheduler` failed before implementation because the OCR scheduler, scheduling input, scheduling policy, and scheduling decision APIs were unresolved.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p ocr-client
cargo test -p ingest-scheduler
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p ocr-client`: exit 0; 4 S12 tests passed, covering cache-key validation, page-level OCR result shape, page timeout budget, cancellation priority, disabled-worker behavior, and Debug redaction for image bytes, OCR text, and content hashes.
- `cargo test -p ingest-scheduler`: exit 0; 4 S12 tests passed, covering default OCR-disabled planning, `OCR_REQUIRED` queue membership, enabled page-limit planning, page timeout propagation, cache-key construction, and searchable-document exclusion.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed, including parser-PDF scanned-file `OcrRequired` regression and S9 import/search regressions.

Review notes:

- Sub-agent review confirmed S12 should not add a database page-task table or wire OCR into import/query yet. `DocumentStatus::OcrRequired` remains the persisted queue membership for this skeleton.
- Known follow-up risk: current document content hash is still the quick fingerprint, not a full-file OCR cache hash. Real OCR worker support must address that before cache correctness claims.
- Known follow-up risk: a real multi-worker OCR queue needs atomic claim semantics and persisted page metadata; this slice only plans in-memory page items.

Scope note:

- S12 adds local OCR client and scheduling interfaces only. It does not call Tesseract/OCRmyPDF, render pages, write OCR cache files, persist page-level OCR tasks, or run OCR from search/import paths.

### S13

TDD red check:

```bash
cargo test -p resume-cli --test s13_diagnostics
```

Output summary:

- The S13 CLI diagnostics test failed before implementation because `resume-cli doctor` and `resume-cli export-diagnostics --redact` were not implemented.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p resume-cli --test s13_diagnostics
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run -p resume-cli -- doctor
cargo run -p resume-cli -- export-diagnostics --redact
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p resume-cli --test s13_diagnostics`: exit 0; 3 tests passed, covering no-index doctor output, corrupt-index doctor output, redacted diagnostics export, no private path leakage, and no fake P95 benchmark output.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.
- `resume-cli doctor`: exit 0; reported metadata ok, index/searchable/OCR/recovery counts, full-text search-index state, a current-run query smoke result, simulated fault hooks, and diagnostics redaction availability.
- `resume-cli export-diagnostics --redact`: exit 0; emitted a redacted JSON skeleton with aggregate counts, search index state, query smoke status, and simulated fault hook names.

Review notes:

- `doctor` treats a corrupt Tantivy snapshot as a diagnostic state and keeps output path-free.
- Fault hooks are intentionally simulated names only: `daemon_restart`, `index_snapshot_corrupt`, and `disk_space_low`. No process kill or disk-fill command is run in this slice.

Scope note:

- S13 is a small diagnostics and smoke slice. It does not claim production benchmark results, P95 latency, destructive fault injection, complete diagnostic bundles, or release readiness.

### S14

Sub-agent read-only audit:

- Deletion/recovery explorer identified that deleted documents were modeled but not propagated through import rescans, and that default no-filter search trusted the full-text index without metadata visibility hydration.
- Parser/OCR explorer identified OCR handoff as a future high-value slice, but it remains separate because this slice targets the stable-blocking deletion behavior without requiring external OCR/model dependencies.

TDD red checks:

```bash
cargo test -p meta-store mark_document_deleted_sets_tombstone_hides_versions_and_status_counts
cargo test -p resume-cli --test s14_delete_search
```

Output summary:

- `meta-store` failed before implementation because `MetaStore::mark_document_deleted` did not exist.
- `resume-cli --test s14_delete_search` failed before implementation because `resume-cli delete` was not a recognized command.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p meta-store
cargo test -p import-pipeline
cargo test -p resume-cli --test s8_search_cli
cargo test -p resume-cli --test s14_delete_search
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 18 S3 tests passed, including the new soft-delete tombstone and hidden-version test.
- `cargo test -p import-pipeline`: exit 0.
- `cargo test -p resume-cli --test s8_search_cli`: exit 0 after upgrading the test to seed matching synthetic metadata for default visibility hydration.
- `cargo test -p resume-cli --test s14_delete_search`: exit 0; 3 tests passed, covering explicit CLI soft delete, import-rescan deletion propagation, and stale-index metadata filtering.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

CLI smoke:

```bash
cargo run -p resume-cli -- --data-dir "$tmp/data" import --root tests/fixtures/resumes
cargo run -p resume-cli -- --data-dir "$tmp/data" search Java
cargo run -p resume-cli -- --data-dir "$tmp/data" delete --doc-id "$doc_id"
cargo run -p resume-cli -- --data-dir "$tmp/data" search Java
```

Output summary:

- Import completed for 3 synthetic files with 2 searchable documents, 1 OCR-required document, 0 failed documents, and 0 deleted documents.
- Search before delete returned 2 synthetic Java results.
- `delete --doc-id` returned `status: deleted`, `index rebuilt: true`, and `indexed documents: 1`.
- Search after delete returned 1 synthetic Java result and did not return the deleted DOCX fixture.

Scope note:

- S14 implements soft tombstones and default-search metadata visibility filtering for full-text search. Import-rescan deletion propagation only runs after a clean crawl with no scan errors. It does not physically delete user files, cancel OCR/vector work, delete vector-index records, implement staging snapshot pointer swaps, or claim complete audit/retention policy.

### S15

Sub-agent read-only audit:

- The OCR handoff audit recommended reusing the existing `ingest_job` table for a document-level `ocr_document` job instead of adding page-level OCR tasks before a renderer/cache/worker exists.
- The boundary remains explicit: this slice makes scanned/OCR-required PDFs durable and restart-claimable, but it does not execute OCR, generate OCR text, or mark OCR as complete.

TDD red checks:

```bash
cargo test -p meta-store ocr_document_jobs_are_durable_idempotent_and_claimable_by_kind
cargo test -p resume-cli --test s15_ocr_handoff
```

Output summary:

- `meta-store` failed before implementation because `IngestJobKind::OcrDocument`, `MetaStore::enqueue_ocr_job_for_document`, `MetaStore::claim_next_job_by_kind`, and OCR job queue status counts did not exist.
- `resume-cli --test s15_ocr_handoff` failed before implementation because imports could persist `DocumentStatus::OcrRequired` without a durable OCR handoff job.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p meta-store
cargo test -p import-pipeline
cargo test -p resume-cli --test s15_ocr_handoff
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 19 S3 tests passed, including durable, idempotent, kind-filtered claim behavior for `ocr_document` jobs and schema V3 migration coverage.
- `cargo test -p import-pipeline`: exit 0.
- `cargo test -p resume-cli --test s15_ocr_handoff`: exit 0; 2 tests passed, covering scanned synthetic PDF import into a durable OCR document job, restart claim by kind, no searchable OCR text, and no duplicate OCR job on repeated import.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

CLI smoke:

```bash
cargo run -p resume-cli -- --data-dir "$tmp/data" import --root tests/fixtures/resumes
cargo run -p resume-cli -- --data-dir "$tmp/data" status
cargo run -p resume-cli -- --data-dir "$tmp/data" doctor
cargo run -p resume-cli -- --data-dir "$tmp/data" search scanned --top-k 20
cargo run -p resume-cli -- --data-dir "$tmp/data" export-diagnostics --redact
```

Output summary:

- Import completed for 3 synthetic files with 2 searchable documents, 1 OCR-required document, 1 queued OCR handoff job, 0 failed documents, and 0 scan errors.
- `status` and `doctor` reported `ocr queue: 1` and `ocr jobs queued: 1`.
- Searching `scanned` returned `results: 0`, confirming the scanned fixture was not made searchable without OCR.
- Redacted diagnostics included aggregate `ocr_jobs_queued` and did not include raw paths, queries, or resume text.

Scope note:

- S15 implements a durable document-level OCR handoff queue only. It does not call Tesseract/OCRmyPDF, render PDF pages, persist page-level OCR tasks, write OCR cache files, index OCR output, persist bbox/confidence evidence, or claim worker crash recovery beyond the existing retryable job claim primitive.

### S16

Sub-agent read-only audit:

- The field-search audit confirmed the next highest-value local slice was to move rule field extraction out of the CLI query path and persist extracted evidence as `EntityMention` rows during import.
- The audit also flagged that candidate folding, Tantivy field fast fields, contact hash indexes, and field F1/performance claims must remain out of scope unless separately implemented and verified.

TDD red checks:

```bash
cargo test -p extractor-rules extracts_company_title_and_certificate_with_evidence
cargo test -p meta-store entity_mentions_replace_query_and_redact_values
cargo test -p resume-cli --test s16_persisted_fields
```

Output summary:

- `extractor-rules` failed before implementation because `FieldType::Company`, `FieldType::Title`, and `FieldType::Certificate` did not exist.
- `meta-store` failed before implementation because `EntityMention`, `EntityMentionId`, `EntityType`, `replace_entity_mentions`, `entity_mentions_for_version`, and `StoreStatusSummary::entity_mentions` were not exposed.
- `resume-cli --test s16_persisted_fields` failed before implementation because field filtering depended on re-extracting from `ResumeVersion.clean_text` during search.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p extractor-rules
cargo test -p meta-store
cargo test -p import-pipeline
cargo test -p resume-cli --test s10_search_filters
cargo test -p resume-cli --test s16_persisted_fields
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p extractor-rules`: exit 0; 7 tests passed, including company/title/certificate extraction with span evidence and Debug redaction.
- `cargo test -p meta-store`: exit 0; 20 S3 tests passed, including schema V4 migration, version-scoped mention replacement/query, mention count, and raw value Debug redaction.
- `cargo test -p import-pipeline`: exit 0.
- `cargo test -p resume-cli --test s10_search_filters`: exit 0; existing degree/skill/years filters still pass after moving to persisted mentions.
- `cargo test -p resume-cli --test s16_persisted_fields`: exit 0; filtered search still worked after test code cleared persisted `raw_text` and `clean_text`, proving the filter path reads persisted mentions instead of doing search-time extraction.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

CLI smoke:

```bash
cargo run -p resume-cli -- --data-dir "$tmp/data" import --root tests/fixtures/resumes
cargo run -p resume-cli -- --data-dir "$tmp/data" status
cargo run -p resume-cli -- --data-dir "$tmp/data" search Java --degree bachelor --skills-any java --years-experience-min 4 --top-k 20
cargo run -p resume-cli -- --data-dir "$tmp/data" export-diagnostics --redact
```

Output summary:

- Import completed for 3 synthetic files with 2 searchable documents, 1 OCR-required document, and 1 queued OCR handoff job.
- `status` reported `entity mentions: 16` for the two searchable synthetic resumes.
- Filtered search using degree, skill, and years constraints returned the 2 synthetic Java resumes.
- Redacted diagnostics included aggregate `entity_mentions` only; it did not include field raw values, paths, queries, or resume text.

Scope note:

- S16 persists rule field mentions and removes search-time field extraction from CLI filtering. It does not implement Tantivy fast-field filtering, pre-recall DB/index field filtering, candidate soft dedupe/folding, hashed contact indexes, model-based extraction, field F1 evaluation, or production-scale field latency claims.

### S17

Sub-agent read-only audit:

- The benchmark audit confirmed the next local slice should be a synthetic query benchmark runner, not a 10万/100万 production benchmark or P95 pass claim.
- The audit also flagged that small synthetic runs must keep `target_claim` as `not_evaluated` and `million_scale_verified` as false unless a real large-scale run is actually executed.

TDD red checks:

```bash
cargo test -p benchmark-runner --test s17_benchmark_runner
cargo test -p benchmark-runner --test s17_benchmark_cli
```

Output summary:

- The first benchmark-runner test failed before implementation because `SyntheticBenchmarkConfig` and `run_synthetic_query_benchmark` did not exist.
- The CLI test failed before implementation because no `resume-benchmark` binary existed for Cargo to expose as `CARGO_BIN_EXE_resume-benchmark`.
- After adding initial implementation, the report-field red check failed because `BenchmarkReport` lacked `qps`, `index_size_bytes`, and `percentile_confidence`.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p benchmark-runner
cargo clippy -p benchmark-runner --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p benchmark-runner`: exit 0; 3 S17 tests passed, covering synthetic benchmark config validation, real Tantivy-backed query measurements, redacted JSON, and the `resume-benchmark` CLI.
- `cargo clippy -p benchmark-runner --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

CLI smoke:

```bash
cargo run -p benchmark-runner -- synthetic-query --index-dir "$tmp/index" --documents 128 --queries 20 --top-k 10 --json
```

Output summary:

- The command generated a synthetic Tantivy full-text index and emitted redacted JSON with `run_id`, `platform`, `dataset_kind`, document/query counts, build time, query total time, QPS, index size, latency min/mean/P50/P95/P99/max, zero-result count, and total hits.
- The output explicitly included `million_scale_verified:false`, `percentile_confidence:"smoke"`, and `target_claim:"not_evaluated"`.
- The output did not include raw synthetic resume text, raw query text, or the local index path.

Scope note:

- S17 adds a synthetic query benchmark runner only. It does not execute 10万/100万 mixed-corpus benchmarks, does not benchmark OCR or vector recall, does not collect RSS/CPU/disk telemetry, does not verify Windows/macOS benchmark parity, and does not claim any P95 target is met.

### S18

Sub-agent read-only audit:

- The candidate-folding audit confirmed the smallest safe slice is CLI search folding over already assigned `candidate_id` values after metadata hydration.
- The audit also flagged that filtering must happen before folding in filtered search, so a non-matching version cannot hide a matching version for the same candidate.

TDD red check:

```bash
cargo test -p resume-cli --test s18_candidate_folding
```

Output summary:

- The new CLI integration test failed before implementation because default search returned both synthetic versions sharing the same assigned candidate instead of folding to the best version.

Implementation and acceptance:

```bash
cargo test -p resume-cli --test s18_candidate_folding
cargo test -p resume-cli --test s8_search_cli
cargo test -p resume-cli --test s10_search_filters
cargo test -p resume-cli --test s14_delete_search
cargo test -p resume-cli --test s16_persisted_fields
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo test -p resume-cli --test s18_candidate_folding`: exit 0; default and filtered CLI search folded two synthetic versions with the same assigned `candidate_id` to the best search hit while preserving two synthetic documents without `candidate_id` as independent results.
- `cargo test -p resume-cli --test s8_search_cli`: exit 0; existing no-candidate full-text CLI search behavior still passed.
- `cargo test -p resume-cli --test s10_search_filters`: exit 0; persisted field filtering and top-k behavior still passed.
- `cargo test -p resume-cli --test s14_delete_search`: exit 0; soft-deleted and stale-index hits remained hidden.
- `cargo test -p resume-cli --test s16_persisted_fields`: exit 0; filtered search still used persisted entity mentions.
- `cargo fmt --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S18 adds search result folding only for versions that already have an assigned `candidate_id`. It does not implement automatic candidate assignment, contact hashing, candidate soft-dedupe rules, merge confidence, suspected-duplicate hints, or UI/API support for expanding all versions of a candidate.

### S19

Sub-agent read-only audit:

- The candidate-store audit recommended a meta-store-first slice: persist `Candidate`, index already-keyed `ContactHash` values, and expose explicit assignment APIs without deriving hashes from extracted email/phone text.
- The audit recommended not wiring import-pipeline yet because no keyed hashing/key-management boundary exists in the repo.

TDD red checks:

```bash
cargo test -p meta-store candidates_persist_and_are_found_only_by_hashed_contact_material
cargo test -p meta-store explicit_candidate_assignment_requires_existing_candidate
```

Output summary:

- The first candidate persistence test failed before implementation because `meta-store` did not re-export `Candidate`/`ContactHash` and did not expose candidate persistence or contact-hash lookup APIs.
- The explicit assignment test failed before implementation because `MetaStore::assign_candidate_to_version` did not exist.

Implementation and acceptance:

```bash
cargo test -p core-domain contact_hash_only_hydrates_external_keyed_digests
cargo test -p meta-store
cargo test -p import-pipeline
cargo test -p resume-cli --test s16_persisted_fields
cargo test -p resume-cli --test s18_candidate_folding
cargo fmt --check
cargo clippy -p core-domain -p meta-store --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo test -p core-domain contact_hash_only_hydrates_external_keyed_digests`: exit 0; `ContactHash` still requires external keyed digest material, redacts display/debug, rejects invalid digests, and now canonicalizes uppercase hex to lowercase.
- `cargo test -p meta-store`: exit 0; 24 meta-store tests passed, including schema v5 migration, candidate persistence, contact-hash lookup, unique contact-hash indexes, explicit assignment requiring an existing candidate, hashed-contact assignment reuse, version-count updates, and v1-to-v5 upgrade preservation.
- `cargo test -p import-pipeline`: exit 0; import-pipeline still compiles without automatic candidate assignment.
- `cargo test -p resume-cli --test s16_persisted_fields`: exit 0; persisted field mentions remain the filtering source and no search-time extraction was reintroduced.
- `cargo test -p resume-cli --test s18_candidate_folding`: exit 0; assigned-candidate folding still works with schema v5.
- `cargo fmt --check`: exit 0.
- `cargo clippy -p core-domain -p meta-store --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S19 persists candidate records and supports assignment only from existing `CandidateId` values or already-keyed `ContactHash` values. It deliberately does not derive hashes from `EntityMention` email/phone raw values, does not add import-time automatic candidate assignment, does not implement key storage/rotation, does not enforce a `resume_version.candidate_id` foreign key yet, and does not provide merge review or suspected-duplicate UI.

### S20

Sub-agent read-only audit:

- The IPC audit recommended a loopback-only status endpoint first, exposed as `resume-daemon run --foreground --ipc-listen 127.0.0.1:0`, with stdout printing a machine-readable `ipc status endpoint: http://127.0.0.1:<port>/status` line.
- Review flagged that raw snapshot tokens must not leave the store boundary through IPC; the implementation now exposes only the aggregate boolean `snapshot_present`.
- Review also flagged missing negative-path coverage; tests now cover no SQLite fallback on IPC failures, non-loopback rejection, and wrong-path rejection.
- Follow-up sub-agent review reported no remaining S20 must-fix findings.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc
```

Output summary:

- Before the CLI IPC implementation, the new CLI IPC test failed because `resume-cli status --ipc` did not connect to the fake daemon. The test server was then tightened to read complete HTTP headers instead of one partial TCP read.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 3 tests passed, covering loopback `/status` JSON, non-loopback bind rejection, and 404 for non-status IPC paths. The JSON includes aggregate counts plus `snapshot_present`, and test-seeded private snapshot/manifest tokens are not emitted.
- `cargo test -p resume-cli --test s20_status_ipc`: exit 0; 4 tests passed, covering text rendering from daemon IPC, connect failure without SQLite fallback, HTTP error without SQLite fallback, and non-loopback/wrong-path URL rejection.
- `cargo fmt --check`: exit 0.
- `cargo clippy -p resume-cli -p resume-daemon --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed, including the new S20 daemon and CLI IPC tests.

Scope note:

- S20 completes only a local loopback HTTP/JSON status IPC slice. It does not complete the final IPC target: no gRPC/UDS/Named Pipe transport, authenticated command API, import/search IPC endpoints, daemon service lifecycle integration, Windows IPC validation, or cross-platform IPC packaging is implemented.

### S21

Sub-agent read-only audit:

- The candidate import audit recommended a separate privacy boundary for keyed contact hashing, rather than adding PII hashing to `core-domain`.
- The audit recommended deriving hashes only from normalized email/phone `EntityMention` values, then using the existing `MetaStore::assign_candidate_from_hashed_contacts` API after each resume-version upsert to preserve idempotency across reimports.
- The audit also flagged search snippets as a possible PII path; this slice now redacts email and phone patterns in full-text snippets.
- Follow-up review found two must-fix gaps: compact phone numbers such as `+14155550132` still leaked through snippets, and reimport could clear an existing candidate assignment before reassignment. Both were fixed with targeted regression coverage.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext snippets_redact_contact_values_near_query_matches
```

Output summary:

- Before implementation, `resume-cli --test s21_import_candidate_assignment` failed because no local contact-hash key was created and import did not assign candidates from extracted contacts.
- After import assignment was added, the same test exposed a search snippet leakage path for `Shared.Candidate@Example.Test`; the index-fulltext redaction test failed until snippets redacted email/phone patterns before returning hits.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p privacy
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext snippets_redact_contact_values_near_query_matches
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p privacy -p import-pipeline -p index-fulltext -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo test -p privacy`: exit 0; 2 tests passed, covering deterministic HMAC contact hashes, lowercase 64-hex digest output, Debug redaction, local key creation, key reload stability, and Unix 0600 key-file permissions.
- `cargo test -p index-fulltext snippets_redact_contact_values_near_query_matches`: exit 0; snippets preserve the query context while replacing email, separated phone, and compact phone values with redaction markers.
- `cargo test -p resume-cli --test s21_import_candidate_assignment`: exit 0; 2 tests passed, covering two synthetic PDFs sharing normalized email/phone importing to the same assigned candidate, durable local key creation under `data_dir/secrets/contact-hash-key-v1`, key/assignment stability across reimport, `version_count` remaining stable, search folding without contact leakage, and preservation of an existing manual candidate assignment on same-version reimport without contacts.
- `cargo test -p resume-cli --test s18_candidate_folding`: exit 0; pre-existing assigned-candidate folding still passes.
- `cargo test -p import-pipeline`: exit 0.
- `cargo fmt --check`: exit 0.
- `cargo clippy -p privacy -p import-pipeline -p index-fulltext -p resume-cli --all-targets -- -D warnings`: exit 0 after replacing a range-loop key decoder with iterator-based decoding.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed, including the new privacy, snippet redaction, and S21 import assignment tests.

Scope note:

- S21 implements import-time automatic candidate assignment only when normalized email or phone fields are available. It does not encrypt the existing SQLite `entity_mention` raw/normalized fields, rotate or back up contact-hash keys, implement merge-review UX, resolve conflicting multi-contact candidates, add low-confidence duplicate hints, or prove dedupe precision/recall on a real corpus.

### S22

Sub-agent review:

- A read-only explorer recommended S22 as the highest-value local production slice after S21: stop duplicating email/phone plaintext in `entity_mention` while preserving keyed contact assignment.
- Spec review found one blocking gap in the first implementation: future writes were redacted, but existing v5 databases would keep plaintext `entity_mention` contact rows. S22 added schema v6 to rewrite those rows.
- Code-quality review reported no blocking or non-blocking findings after the v6 migration and tests were added.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store contact_entity_mentions_do_not_persist_contact_values
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store schema_v6_redacts_existing_contact_entity_mentions
```

Output summary:

- `contact_entity_mentions_do_not_persist_contact_values` failed before implementation because the hydrated email mention still returned `Sensitive.Candidate@Example.Test` instead of `<redacted:email>`.
- `schema_v6_redacts_existing_contact_entity_mentions` failed before the migration because `run_migrations()` applied no version 6 migration and legacy contact rows kept plaintext values.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 26 tests passed, including direct SQLite assertions that future and legacy email/phone `entity_mention` rows store `<redacted:email>`/`<redacted:phone>` with `normalized_value = NULL` while retaining spans, confidence, extractor, and non-contact fields.
- `cargo test -p resume-cli --test s21_import_candidate_assignment`: exit 0; 2 tests passed, including keyed-contact candidate assignment stability and imported contact mentions hydrating without email/phone plaintext.
- `cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed, including S22 meta-store migration/redaction coverage and S21 import assignment regression coverage.

Scope note:

- S22 only removes email/phone plaintext duplication from `entity_mention.raw_value` and `entity_mention.normalized_value` for future writes and existing rows reached by schema v6 migration. It does not encrypt SQLite, scrub `resume_version.raw_text`/`clean_text`, remove contact text from the full-text index, prove physical deletion from SQLite free pages or WAL files, implement SQLCipher, rotate or back up contact-hash keys, or complete a full PII audit.

### S23

Sub-agent review:

- A read-only explorer recommended full-text index contact redaction over doctor/key-health work because S22 had already removed one duplicate contact storage path while Tantivy stored fields still accepted raw contact values.
- Spec/quality review found one blocking gap in the first S23 diff: phone redaction missed no-space parenthesized forms like `(415)555-0132` and `+1(415)555-0132`. The regex and stored-field test were expanded to cover those forms.
- The same review noted that stored-field inspection alone is weaker than checking indexed query behavior. The S23 test now also asserts raw email/phone queries do not match after redaction.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext stored_index_fields_redact_contact_values_before_commit
```

Output summary:

- Before implementation, the stored-field test failed because the Tantivy stored text did not contain `<redacted-email>`, proving raw contact values were still written to index fields.
- After the initial implementation, the expanded no-space phone coverage failed because `(415)555-0132` and `+1(415)555-0132` still left `415` in stored fields. The phone regex was tightened and the test then passed.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p index-fulltext`: exit 0; 9 tests passed, including direct Tantivy stored-field assertions for `file_name`, `clean_text`, `all_sections`, and `section_text` contact redaction plus non-contact search preservation and raw contact query non-match behavior.
- `cargo test -p resume-cli --test s21_import_candidate_assignment`: exit 0; 2 tests passed, including import-time keyed-contact assignment, redacted contact mention hydration, folded search results without contact leakage, and raw contact search returning zero results.
- `cargo clippy -p index-fulltext -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed, including S23 full-text storage redaction and S21 import assignment regressions.

Scope note:

- S23 redacts email/phone-like contact values only on future full-text index writes for `file_name`, `clean_text`, `all_sections`, and `section_text`, and raw contact queries are not claimed as a supported full-text feature. It does not rewrite already-existing Tantivy segments, encrypt or scrub SQLite `resume_version.raw_text`/`clean_text`, implement hash-based contact search, prove physical deletion from SQLite/Tantivy storage, rotate keys, or complete a full PII audit.

### S24

Sub-agent review:

- A read-only review found one blocking issue in the first S24 implementation: `Path::exists()` could collapse metadata/access errors into `missing`, so unreadable key paths might not be reported as `unreadable`.
- S24 changed the inspector to use `try_exists()` and added Unix-only unreadable coverage at both the privacy and CLI layers.
- The review found no evidence that doctor/export creates keys, repairs permissions, outputs key paths/material, or claims key rotation/backup/SQLCipher/full privacy audit completion.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p privacy contact_hash_key_inspection_is_read_only_and_redacted
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics
```

Output summary:

- Before implementation, the privacy test failed to compile because `inspect_contact_hash_key` and `ContactHashKeyState` did not exist.
- Before CLI integration, `resume-cli --test s13_diagnostics` failed because doctor/export output did not include contact-hash key health and could not report invalid key material.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p privacy
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics
/Users/frankqdwang/.cargo/bin/cargo clippy -p privacy -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p privacy`: exit 0; 4 tests passed, covering read-only missing/invalid/weak-permissions/ready/unreadable key inspection, key material/path redaction in debug output, and no key creation during inspection.
- `cargo test -p resume-cli --test s13_diagnostics`: exit 0; 5 tests passed, covering missing, invalid, and unreadable contact-hash key diagnostics in doctor/export output without path or key-material leakage.
- `cargo clippy -p privacy -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0 on rerun; all workspace tests passed. The first workspace run exposed a transient S20 IPC connection test failure, and a focused rerun of that single S20 test passed before the final full workspace rerun passed.

Scope note:

- S24 adds only read-only contact-hash key health reporting for `missing`, `ready`, `invalid`, `weak_permissions`, and `unreadable` states in doctor/export diagnostics. It does not rotate keys, back up or restore keys, encrypt SQLite, verify all diagnostic package contents, implement SQLCipher, or complete a full PII/security audit.

### S25

Sub-agent review:

- One read-only explorer recommended making the next production slice a real atomic full-text snapshot publish path so failed writes do not destroy the last committed query surface.
- A second read-only explorer recommended adding meta-store `index_health` to doctor/export diagnostics to avoid filesystem-only index-health misreports.
- S25 combines these local, non-external parts: active full-text snapshot publishing, active/legacy read resolution, staging orphan reporting, and redacted metadata index-health diagnostics.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext published_snapshot_becomes_active_without_reading_staging_orphans
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_metadata_index_health_with_active_snapshot
```

Output summary:

- Before implementation, the index-fulltext test failed to compile because `publish_snapshot`, `inspect_snapshot_root`, `SnapshotReadTarget`, `SnapshotRootState`, and `FullTextIndex::open_active` did not exist.
- Before CLI integration, the diagnostics test failed to compile because `publish_snapshot` did not exist and doctor/export did not report meta-store index-health alongside filesystem/Tantivy state.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p index-fulltext`: exit 0; 10 tests passed, including published snapshot activation, active snapshot read resolution, and staging orphan detection while preserving existing full-text behavior.
- `cargo test -p resume-cli --test s13_diagnostics`: exit 0; 6 tests passed, including active snapshot diagnostics, meta-store `index_health`, last-snapshot redaction, read-target reporting, staging orphan count, and no data-dir leakage.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 3 tests passed, covering import-built active snapshots, status/search reopening, recoverable import task reuse, and no live-running task takeover.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p index-fulltext -p import-pipeline -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Synthetic CLI smoke:

```bash
mktemp -d /tmp/resume-ir-s25-smoke.XXXXXX
/Users/frankqdwang/.cargo/bin/cargo run -p resume-cli -- --data-dir /tmp/resume-ir-s25-smoke.5l8aly import --root tests/fixtures/resumes
/Users/frankqdwang/.cargo/bin/cargo run -p resume-cli -- --data-dir /tmp/resume-ir-s25-smoke.5l8aly status
/Users/frankqdwang/.cargo/bin/cargo run -p resume-cli -- --data-dir /tmp/resume-ir-s25-smoke.5l8aly search Java
/Users/frankqdwang/.cargo/bin/cargo run -p resume-cli -- --data-dir /tmp/resume-ir-s25-smoke.5l8aly doctor
/Users/frankqdwang/.cargo/bin/cargo run -p resume-cli -- --data-dir /tmp/resume-ir-s25-smoke.5l8aly export-diagnostics --redact
```

Output summary:

- Import completed against synthetic fixtures: 3 files discovered, 2 searchable documents, 1 OCR-required document, 1 OCR job queued, 0 failed documents, and 0 scan errors.
- Status reported `index health: ready`, a full-text snapshot token, and `search index: available (full-text snapshot)`.
- Search for `Java` returned the 2 synthetic searchable fixtures through the active snapshot.
- Doctor reported `last snapshot: present`, `search index read target: published_snapshot`, `query smoke: ok`, `staging orphans: 0`, and no data-dir path.
- `export-diagnostics --redact` reported `search_index_state: available`, `search_index_read_target: published_snapshot`, `index_health: ready`, `last_snapshot: present`, and `staging_orphans: 0` without raw paths, raw queries, or raw resume text.

Scope note:

- S25 publishes future full-text writes into staging directories, validates them, then switches an active snapshot pointer; search/status/doctor/export now resolve the active snapshot and remain compatible with legacy root indexes. This does not yet implement fallback if the active pointer itself is later corrupted, old snapshot garbage collection, physical purge of old Tantivy segments, vector snapshots, SQLCipher, full disk-full or kill-daemon fault injection, or Windows/macOS atomic rename validation.

### S26

Sub-agent review:

- A read-only explorer confirmed the S25 read path failed hard when `active-snapshot` was invalid, missing, pointed to a missing snapshot directory, or pointed to a corrupt snapshot despite other usable snapshots being present.
- The recommended S26 scope was read-only last-good selection only: enumerate published snapshots, ignore staging, pick the newest usable snapshot, report recovered state in redacted diagnostics, and avoid GC, repair, or retention policy changes.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext active_snapshot_corruption_falls_back_to_last_good_snapshot
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_search_use_last_good_snapshot_after_active_snapshot_corruption
```

Output summary:

- Before implementation, the index-fulltext test failed to compile because `SnapshotRootState::Recovered` and `SnapshotRootInspection::fallback_snapshot()` did not exist.
- Before CLI integration, the diagnostics test failed because search failed instead of falling back when the active snapshot was corrupted.
- The first green attempt exposed that Tantivy could still open a snapshot after a weak metadata corruption; S26 now also checks that `meta.json` has JSON-shaped metadata before considering a snapshot usable.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p index-fulltext`: exit 0; 11 tests passed, including active snapshot corruption falling back to the previous usable published snapshot without reading staging or corrupt active content.
- `cargo test -p resume-cli --test s13_diagnostics`: exit 0; 7 tests passed, including search using last-good fallback, doctor reporting `recovered (full-text snapshot)`, export-diagnostics reporting `search_index_state: recovered`, and no snapshot token/path leakage.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 3 import/search snapshot regressions passed.
- `cargo clippy -p index-fulltext -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S26 adds read-path fallback only. It does not mutate or repair `active-snapshot`, delete corrupt snapshots, clean staging orphans, implement retention/GC, physically purge deleted content from old segments, add vector snapshot fallback, run real disk-full/kill-daemon fault injection, or validate atomic rename semantics on Windows.

### S27

Sub-agent review:

- A read-only explorer confirmed the existing product shape should stay unified around authorized `roots`: specified-directory scanning is the base capability, and whole-disk or large-root discovery is a safer profile over the same root scanning path rather than a separate pipeline.
- Review agents found and drove fixes for discovery-specific risks: an overly narrow system-directory skip list, profile-split task identity, deletion propagation across skipped or unreadable subtrees, duplicate CLI flags, and misplaced import deletion semantics in `fs-crawler`.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler discovery_profile_skips_system_cache_and_dependency_directories
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search discovery_profile_reuses_root_scan_without_deleting_skipped_directories
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search discovery_import_does_not_take_over_live_running_task_for_same_root
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli import_rejects_duplicate_root_and_profile_flags_without_path_leak
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline discovery_deletion_requires_direct_parent_directory_to_be_scanned
```

Output summary:

- Before implementation, the crawler red test failed to compile because `ScanProfile` and `crawl_with_fs_profile` did not exist.
- Before CLI integration, `import --root <path> --profile discovery` failed with the old usage string.
- After the first green path, review red tests exposed that discovery still split task identity by profile, accepted duplicate flags with last-wins behavior, skipped too little at disk roots, and globally disabled deletion instead of applying deletion only to safely traversed directories.
- Final import-pipeline red test showed that using the scanned root as a deletion parent could still delete historical documents under unreadable child directories; S27 now requires a direct scanned parent directory for discovery deletion propagation.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p fs-crawler -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p fs-crawler`: exit 0; 7 crawler tests passed, including discovery skipping root-level system directories and dependency/cache directories while preserving nested business directories such as `Target`.
- `cargo test -p import-pipeline`: exit 0; 2 import-pipeline tests passed, including discovery deletion requiring a directly scanned parent directory and excluding skipped subtrees.
- `cargo test -p resume-cli`: exit 0; 30 CLI tests passed, including discovery import, duplicate flag rejection, same-root running-task protection across profiles, and discovery reimport preserving skipped subtree documents while deleting missing documents from traversed directories.
- `cargo clippy -p fs-crawler -p import-pipeline -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S27 keeps `resume-cli import --root <path>` as the unified import path and adds `--profile discovery` for local large-root discovery. It does not add automatic whole-machine root selection, multi-root CLI/UI, scan progress/cancel, time/file-count/IO budgets, real local resume witness runs, follow-symlink traversal, persisted scan-profile metadata, or cross-platform validation of root exclusions.

### S28

Sub-agent review:

- A read-only explorer recommended keeping `ImportTask.root_path` as a single canonical root and implementing multi-root import as CLI batching over existing per-root tasks, rather than storing a composite root key.
- A final read-only reviewer confirmed the implemented S28 path uses one existing `ImportTask` per canonical root, preserves running/retryable task behavior, rejects duplicate/overlapping roots without path leakage, and keeps deletion propagation isolated to each single-root import.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_multiple_roots_builds_searchable_index_without_path_leak
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli import_rejects_overlapping_roots_without_path_leak
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search multi_root_reimport_marks_missing_files_deleted_per_root
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search multi_root_import_does_not_take_over_live_running_task_for_any_root
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search multi_root_import_reuses_recoverable_task_for_each_root
```

Output summary:

- Before implementation, multi-root import tests failed because `resume-cli import` still rejected a second `--root` with the old usage path.
- After the first implementation, review red tests showed the composite multi-root task key bypassed per-root running and retryable task semantics.
- S28 now validates canonical roots as distinct and non-overlapping, then executes each root through its own existing `ImportTask` and merges the user-facing summary without printing requested or canonical paths.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p import-pipeline`: exit 0; 2 import-pipeline tests passed.
- `cargo test -p resume-cli --test s4_cli`: exit 0; 5 CLI base tests passed, including duplicate and overlapping root rejection without path leakage.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 7 import/search tests passed, including multi-root import, multi-root running-task refusal, and per-root retryable task reuse.
- `cargo test -p resume-cli --test s14_delete_search`: exit 0; 5 delete/search tests passed, including multi-root reimport tombstoning a missing file in one root without hiding the other root.
- `cargo test -p resume-cli`: exit 0; 34 CLI tests passed.
- `cargo clippy -p import-pipeline -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S28 adds multi-root CLI import over existing per-root task semantics. It does not add automatic root presets, persisted scan-scope records, progress/cancel, per-root partial-failure reporting beyond the merged summary, a true all-or-nothing multi-root transaction, real local resume witness runs, or cross-platform validation of Windows/macOS root overlap behavior.

### S29

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client
```

Output summary:

- Before implementation, `ocr-client` failed because `LocalOcrCommandClient`, `LocalOcrCommandSpec`, dynamic `CancellationToken::cancel`, and `OcrErrorKind::EngineFailed` did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client
/Users/frankqdwang/.cargo/bin/cargo test -p ingest-scheduler
/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p ingest-scheduler --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p ocr-client`: exit 0; 14 OCR client tests passed, covering disabled mode, redacted debug output, local command execution, structured stdout with confidence, missing binary as `WorkerUnavailable`, timeout, running cancellation, descendant process cleanup, owner-only input files, CRLF schema output, non-schema output rejection, out-of-range confidence rejection, and malformed engine output as `EngineFailed`.
- `cargo test -p ingest-scheduler`: exit 0; 4 ingest scheduler tests passed after the cancellation token became dynamically cancellable.
- `cargo clippy -p ocr-client -p ingest-scheduler --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `git diff --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed, including the 14 OCR client tests.

Sub-agent review:

- A read-only S29 reviewer found three pre-commit blockers: timeout/cancel did not terminate OCR descendant processes that kept inherited pipes open, the temporary rendered-page input file used default permissions, and non-structured stdout was accepted as successful OCR output. S29 now starts Unix OCR commands in a new process group and terminates that group on timeout/cancel/direct-child exit, creates a private temp directory plus `0600` input file on Unix, and requires the `resume-ir-ocr-v1` structured stdout schema with valid confidence.

Scope note:

- S29 adds a production local command OCR client that launches a configured local executable, passes rendered page bytes through a private temporary local input file, supplies page/options via environment variables, parses only `resume-ir-ocr-v1` stdout with valid confidence and text, enforces page timeout, kills on cancellation, terminates Unix descendant processes in the OCR process group, and redacts debug/error surfaces. It does not bundle or license a concrete OCR engine, render PDF pages into images, persist OCR page cache/results, connect the durable OCR queue to this client, index OCR text, persist bbox evidence, run a real scanned-resume witness, implement Windows job-object process-tree termination, or validate Windows command execution.

### S30

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store ocr_page_cache
```

Output summary:

- Before implementation, `meta-store` failed because `OcrPageCacheKey`, `OcrPageCacheEntry`, `OcrPageCacheStatus`, `MetaStore::upsert_ocr_page_cache_entry`, and `MetaStore::ocr_page_cache_entry` did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 28 meta-store tests passed, including V7 migration creation, OCR page cache success/failure upsert, redacted Debug output, key lookup, and invalid key/confidence rejection.
- `cargo clippy -p meta-store --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `git diff --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S30 adds a V7 SQLite OCR page cache table plus redacted key/result APIs for success and retryable/permanent failures. It does not connect the cache to real OCR execution, render PDF pages, store bbox evidence, index OCR output, run a scanned-resume witness, implement cache GC/retention, or encrypt/purge the cached OCR text beyond existing local SQLite behavior.

### S31

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff
```

Output summary:

- Before implementation, `resume-cli` failed because `ocr-worker` was not a recognized command and no CLI path claimed `OcrDocument` jobs for local command OCR execution.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p resume-cli --test s15_ocr_handoff`: exit 0; 4 OCR handoff tests passed, including the blocked no-command worker path and the local command cache-write path.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S31 adds `resume-cli ocr-worker --once --command <path>` to claim one durable `OcrDocument` job, invoke a configured local OCR command, persist a page-1 OCR cache entry, and complete the OCR job/document without printing raw OCR text or paths. The no-command path reports a blocked worker and leaves the queued job untouched. This slice passes local source-document bytes to the command-wrapper input; it still does not render PDF pages, split multi-page documents, index OCR text into search, persist bounding boxes, run the daemon OCR loop, install or license a concrete OCR engine, run a real scanned-resume witness, or validate Windows process-tree cleanup.

### S32

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_uses_discovery_profile_without_path_leak
```

Output summary:

- Before implementation, `resume-cli import --root-preset local-discovery` failed with the import usage message because the CLI only accepted explicit `--root` values.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p resume-cli --test s4_cli`: exit 0; 5 CLI usage/status/import tests passed, including rejection of mixed `--root` and `--root-preset`.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 8 import/search tests passed, including `--root-preset local-discovery` with a synthetic env-overridden root, discovery-profile skipping of dependency directories, path redaction, and searchability of the discovered synthetic PDF.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p resume-cli --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Sub-agent review:

- A read-only explorer recommended modeling whole-machine/local discovery as a root-selection preset rather than a new crawler. S32 follows that by adding `--root-preset local-discovery`, keeping `--root` and preset selection mutually exclusive, defaulting the preset to `ScanProfile::Discovery`, and still using existing canonical root validation plus `import_root_with_options`.

Scope note:

- S32 adds a root preset layer over the existing explicit-root import path. On non-Windows hosts the default local-discovery root set starts at `/`; on Windows it enumerates available drive roots, and tests use the local `RESUME_IR_LOCAL_DISCOVERY_ROOTS` override to avoid reading real user files. This does not prove that the product can find every resume on a real machine, does not add progress/cancel/budget controls, does not persist scan-scope metadata, does not implement explicit real-data confirmation UX, does not run a real local witness scan, and does not validate Windows drive enumeration in a Windows environment.

### S33

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store worker_task_control_defaults_to_running_and_persists_pause_state
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff pause_and_resume_ocr_task_persistently_controls_worker_claims
```

Output summary:

- Before implementation, `meta-store` failed because `WorkerTaskControl`, `WorkerTaskKind`, `MetaStore::worker_task_control`, and `MetaStore::set_worker_task_paused` did not exist.
- Before implementation, the CLI test failed because `resume-cli pause --task ocr` was not implemented.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 29 meta-store tests passed, including V8 migration creation, file-backed pause-state persistence, default running state, resume-state update, and legacy V1 upgrade through V8.
- `cargo test -p resume-cli --test s15_ocr_handoff`: exit 0; 5 OCR handoff tests passed, including pause/resume control preventing `ocr-worker` from claiming queued OCR jobs while paused and allowing claim after resume.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S33 adds SQLite schema V8 `worker_task_control`, `resume-cli pause --task ocr`, `resume-cli resume --task ocr`, status reporting for `ocr task`, and an `ocr-worker` pre-claim pause gate that returns without consuming queued jobs. It does not interrupt an OCR process that is already running, does not add daemon-loop orchestration, does not render PDF pages, does not bundle or license a concrete OCR engine, does not index OCR output into search, does not persist OCR bounding boxes, does not run a real scanned-resume witness, and does not validate Windows process-control behavior.

### S34

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p embedder
```

Output summary:

- Before implementation, `embedder` failed because `LocalEmbeddingCommandSpec`, `LocalEmbeddingCommandEmbedder`, and the command-execution error variants were unresolved.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p embedder
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector
/Users/frankqdwang/.cargo/bin/cargo clippy -p embedder -p index-vector --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p embedder`: exit 0; 5 embedder tests passed, including configured local command execution, structured vector parsing, missing-worker classification, malformed-output rejection without payload leakage, timeout handling, and private input-file permissions.
- `cargo test -p index-vector`: exit 0; 2 vector-index tests passed.
- `cargo clippy -p embedder -p index-vector --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S34 adds a structured local embedding command client that writes a private local input file, invokes a configured local executable, parses the `resume-ir-embedding-v1` stdout protocol, validates model/dimension/output shape, times out stalled workers, and redacts payloads from errors/debug output. It does not select, bundle, license, download, or install a concrete embedding model; the deterministic embedder remains test-only scaffolding, `index-vector` remains in-memory, and product semantic/hybrid search is still not complete.

### S35

Sub-agent note:

- A read-only explorer confirmed the next scan/import slice should persist scan-scope metadata before implementing progress or cancel/budget controls, because progress and cancellation need a durable scan-scope object to attach to.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_scan_scope_persists_root_profile_and_redacted_progress_counts
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_uses_discovery_profile_without_path_leak
```

Output summary:

- Before implementation, `meta-store` failed because `ImportScanScope`, `ImportRootKind`, `ImportRootPreset`, `ImportScanProfile`, `MetaStore::upsert_import_scan_scope`, `MetaStore::import_scan_scope_by_task_id`, `MetaStore::latest_import_scan_scope`, and `StoreStatusSummary::import_scan_scopes` did not exist.
- Before implementation, the CLI test failed because `latest_import_scan_scope` and the scan-scope enum types did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_ipc_connect_failure_does_not_fallback_to_sqlite -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p embedder
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli -p resume-daemon --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0 after formatting.
- `git diff --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 30 meta-store tests passed, including V9 `import_scan_scope` migration, V1-to-V9 upgrade, scope persistence/reopen, redacted Debug output, and status-summary counts.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 8 import/search tests passed, including local-discovery preset scope persistence without stdout/path leakage.
- `cargo test -p resume-cli --test s14_delete_search`: exit 0; 5 delete/search regression tests passed.
- `cargo test -p resume-cli --test s20_status_ipc status_ipc_connect_failure_does_not_fallback_to_sqlite -- --exact`: exit 0 after an earlier full-file run hit a transient port collision in the negative IPC test.
- `cargo test -p embedder`: exit 0; 5 embedder tests passed after hardening the timeout test to record private input-file permissions before sleeping.
- `cargo test -p resume-cli`: exit 0; all CLI integration tests passed.
- `cargo clippy -p meta-store -p resume-cli -p resume-daemon --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S35 adds SQLite schema V9 `import_scan_scope`, typed scan-scope APIs, redacted scan-scope Debug output, CLI import writes for explicit roots and `local-discovery` preset roots, persisted summary counts, and status/doctor/diagnostics/daemon status counters. It does not implement live progress streaming, import cancellation, scan budget enforcement, per-file error UX, encrypted path metadata, a real whole-machine witness scan, or Windows root validation.

### S36

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler scan_options_stop_after_file_budget_without_path_leakage
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_scan_scope_persists_root_profile_and_redacted_progress_counts
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_max_files_limits_scan_and_persists_budget_state_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search budgeted_reimport_does_not_mark_unscanned_missing_files_deleted -- --exact
```

Output summary:

- Before implementation, `fs-crawler` failed because `ScanOptions`, `ScanBudgetKind`, and `crawl_with_fs_options` did not exist.
- Before implementation, `meta-store` and CLI scan-scope tests failed because `ImportScanBudgetKind` and scan budget fields were missing.
- Before implementation, the budgeted reimport CLI test failed because `resume-cli import --max-files` was rejected by usage parsing.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p fs-crawler -p meta-store -p import-pipeline -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p fs-crawler`: exit 0; 8 crawler tests passed, including deterministic file-budget stop and redacted budget Debug output.
- `cargo test -p meta-store`: exit 0; 30 meta-store tests passed, including V10 scan budget fields on `import_scan_scope` and V1-to-V10 upgrade.
- `cargo test -p import-pipeline`: exit 0; import-pipeline unit tests passed.
- `cargo test -p resume-cli`: exit 0; all CLI integration tests passed, including `--max-files`, persisted budget state, and no deletion propagation on budgeted partial reimport.
- `cargo clippy -p fs-crawler -p meta-store -p import-pipeline -p resume-cli --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S36 adds `ScanOptions::max_files`, `resume-cli import --max-files <count>`, scan budget reporting, SQLite schema V10 scan-budget columns on `import_scan_scope`, and disables missing-file deletion propagation when a scan is budget-exhausted. It does not implement live progress streaming, user-triggered cancellation, time/byte/CPU budgets, persisted per-file scan errors, real whole-machine witness scans, encrypted path metadata, or cross-platform full-disk validation.

### S39

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
```

Output summary:

- Before implementation, the new CLI tests failed because `resume-cli` did not recognize `embed-worker`; the expected blocked/no-command behavior and local vector snapshot persistence were absent.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli embed_worker_debug_output_redacts_candidate_text_and_command_path
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p embedder
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p embedder -p index-vector --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p resume-cli embed_worker_debug_output_redacts_candidate_text_and_command_path`: exit 0; the new CLI unit test passed and confirms `EmbedWorkerCandidate` redacts resume text and `EmbedWorkerArgs` redacts the configured command path from Debug output.
- `cargo test -p resume-cli --test s39_embedding_worker`: exit 0; 2 tests passed, covering blocked operation without a local embedding command and local command execution that writes 2 synthetic searchable resume vectors to the persistent vector snapshot without leaking paths or hiding full-text search results.
- `cargo clippy -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo test -p resume-cli`: exit 0; all CLI integration tests passed.
- `cargo test -p embedder`: exit 0; 5 embedder tests passed.
- `cargo test -p index-vector`: exit 0; 4 vector-index tests passed.
- `cargo clippy -p resume-cli -p embedder -p index-vector --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S39 adds `resume-cli embed-worker --once`, an explicit local embedding command requirement, model/dimension/budget/timeout parsing, visible searchable resume-version candidate selection, local command execution through the S34 embedder protocol, persistent local vector snapshot writes, and redacted Debug output for embedding worker candidates/args. It does not choose, bundle, license, download, or install a concrete embedding model; the configured command is trusted to be local/offline and OS-enforced no-network sandboxing is not yet implemented. It does not add daemon-loop embedding, semantic/hybrid query execution, vector snapshot GC/repair, real-data validation, or cross-platform command validation.

### S40

Design note:

- Whole-machine scanning remains a root-selection case over the existing import scanner. This slice does not add a second scanning pipeline; it makes the existing `local-discovery` preset safer by adding a default file-count budget that explicit roots do not inherit and that users can override with `--max-files`.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_uses_discovery_profile_without_path_leak -- --exact
```

Output summary:

- Before implementation, the focused CLI test failed because `--root-preset local-discovery` printed `scan file limit: none` and did not persist budget metadata when the default discovery scan was not exhausted.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_scan_scope_persists_root_profile_and_redacted_progress_counts -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_uses_discovery_profile_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_allows_explicit_file_budget_override_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search explicit_root_import_without_max_files_has_no_default_scan_budget -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search multi_root_import_reports_budget_exhausted_when_later_root_hits_file_limit -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p meta-store import_scan_scope_persists_root_profile_and_redacted_progress_counts -- --exact`: exit 0; the scan-scope test now covers configured but not exhausted file budgets.
- `cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_uses_discovery_profile_without_path_leak -- --exact`: exit 0; the preset now reports `scan file limit: 10000`, persists the non-exhausted file budget, and does not leak local roots.
- `cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_allows_explicit_file_budget_override_without_path_leak -- --exact`: exit 0; explicit `--max-files 1` still overrides the preset default and records an exhausted budget without path leakage.
- `cargo test -p resume-cli --test s9_import_search explicit_root_import_without_max_files_has_no_default_scan_budget -- --exact`: exit 0; explicit roots without `--max-files` still report `scan file limit: none` and persist no scan budget.
- `cargo test -p resume-cli --test s9_import_search multi_root_import_reports_budget_exhausted_when_later_root_hits_file_limit -- --exact`: exit 0 after a sub-agent review found and the implementation fixed aggregate multi-root budget reporting when a later root exhausts the file limit.
- `cargo test -p meta-store`: exit 0; 31 meta-store tests passed.
- `cargo test -p import-pipeline`: exit 0; 2 import-pipeline tests passed.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 13 import/search tests passed.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p meta-store -p import-pipeline -p resume-cli --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S40 sets `local-discovery` to a default 10,000 file budget, keeps explicit `--root` imports unbudgeted unless `--max-files` is supplied, allows user override of the preset budget, persists configured file-budget metadata even when the scan is not exhausted, and reports aggregate multi-root budget exhaustion if any root exhausts the file limit. It does not add progress streaming, user cancellation, time/byte/CPU budgets, a UI for partial results, real whole-machine witness scans, encrypted path metadata, or Windows/macOS full-disk validation.

### S41

Design note:

- Successful OCR output is now part of the same local import/index pipeline as text-layer documents: normalize text, persist a searchable OCR resume version, refresh rule-extracted fields/candidate assignment, mark the document `Searchable`, and rebuild the active full-text snapshot. Whole-machine scanning remains a root-selection case over the existing scanner; explicit directory scanning is retained, and selecting `/`, `/Users`, `C:\`, or `D:\` should use the same scanner with stronger defaults and user-facing guardrails rather than a separate pipeline.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_executes_local_command_persists_cache_and_indexes_searchable_text -- --exact
```

Output summary:

- Before implementation, the focused OCR worker test failed because a successful local OCR command left the scanned document in `OcrDone` and searching the OCR-only token returned `results: 0`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_executes_local_command_persists_cache_and_indexes_searchable_text -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff pause_and_resume_ocr_task_persistently_controls_worker_claims -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p import-pipeline -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p import-pipeline`: exit 0; import-pipeline unit tests passed.
- `cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_executes_local_command_persists_cache_and_indexes_searchable_text -- --exact`: exit 0; the local OCR command writes the page cache, marks the scanned document searchable, and searching the OCR-only token returns one redacted result without leaking local data or fixture paths.
- `cargo test -p resume-cli --test s15_ocr_handoff pause_and_resume_ocr_task_persistently_controls_worker_claims -- --exact`: exit 0; pause/resume still controls worker claims and the eventual successful OCR output becomes searchable.
- `cargo test -p resume-cli --test s15_ocr_handoff`: exit 0; 7 OCR handoff tests passed, including direct cache-hit indexing and empty OCR text staying non-searchable.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p import-pipeline -p resume-cli --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Sub-agent review fix:

- Newton found two P2 issues before commit: `index_ocr_text` promoted the document to `Searchable` before full-text snapshot publish, and tests did not directly cover OCR cache-hit indexing or empty OCR text. The implementation now writes the rebuilt full-text snapshot with pending OCR documents first, then promotes document status and index state after publish succeeds. The S15 handoff suite now covers command success, cache-hit success without invoking the command, and empty successful OCR text remaining `OcrDone` with no searchable version.

Scope note:

- S41 adds `import_pipeline::index_ocr_text`, connects OCR worker cache-hit and command-success paths to OCR text indexing, keeps empty OCR text non-searchable as `OcrDone`, persists OCR text in local SQLite resume versions, reuses existing rule extraction/contact-hash assignment, and rebuilds the full-text index after OCR completion. It does not render multi-page PDF pages, run OCR from the daemon loop, choose/install/license a concrete OCR engine, persist bounding boxes, prove behavior on real scanned resumes, encrypt OCR text at rest, physically purge SQLite/WAL data, or validate Windows process-tree behavior.

### S42

Design note:

- S42 completes the local P3 query loop that remained after the embedding worker slice: `resume-cli search --mode semantic` embeds the query through an explicit local command, opens the persisted vector snapshot, performs KNN, hydrates visible documents from SQLite, applies persisted field filters, folds candidates, and prints redacted output. `--mode hybrid` combines full-text and vector channels with existing RRF. Full-text search remains the default and does not create metadata when the full-text index is missing.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker semantic_and_hybrid_search_use_persistent_vector_snapshot_with_local_query_embedding -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker semantic_search_reports_missing_vector_snapshot_even_when_dimension_is_supplied -- --exact
```

Output summary:

- Before implementation, `semantic_and_hybrid_search_use_persistent_vector_snapshot_with_local_query_embedding` failed because `resume-cli search` did not accept `--mode` or any query embedding/vector options.
- Before the missing-snapshot fix, `semantic_search_reports_missing_vector_snapshot_even_when_dimension_is_supplied` failed because semantic search succeeded against an implicitly created empty vector index when `--dimension` was supplied.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p rank-fusion -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p resume-cli --test s39_embedding_worker`: exit 0; 4 embedding/semantic tests passed, covering local command query embedding, semantic search over the persisted vector snapshot, hybrid RRF search, missing command behavior, and missing vector snapshot behavior without query/path leakage.
- `cargo test -p index-fulltext`: exit 0; 11 full-text tests passed.
- `cargo test -p rank-fusion`: exit 0; 6 rank-fusion tests passed.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p index-fulltext -p rank-fusion -p resume-cli --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Sub-agent review:

- Kant completed a read-only P0-P6 audit and identified P3 semantic/hybrid closure as the highest-value local production feature after restoring build health. It did not edit files. Its stale source snapshot saw missing search symbols before this implementation; the final local verification above covers the corrected state.

Scope note:

- S42 does not choose, install, license, or distribute a production embedding model; it does not add ONNX/HNSW/FAISS or another ANN engine; it does not run a daemon embedding queue, section-level vectors, OS-enforced no-network sandboxing for user embedding commands, real semantic quality benchmarks, real resume witness scans, or cross-platform validation. Those remain incomplete or BLOCKED.

### S43

Design note:

- S43 moves import execution closer to the daemon-owned production control plane. `resume-cli import --enqueue` now persists queued import tasks and scan scope metadata without doing foreground indexing. `resume-daemon run --foreground --once --work-imports-once` claims queued/retryable import tasks from SQLite, reconstructs scan options from persisted scope, runs the existing real import/index pipeline, records updated scan counts, and continues past retryable failures in the same worker pass.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_worker_claim_atomically_marks_next_task_running_and_skips_attempted_tasks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_enqueue_persists_task_without_running_foreground_import -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_once_worker_processes_queued_import_task_from_persistent_scope -- --exact
```

Output summary:

- Before implementation, the meta-store test failed because there was no atomic import-task claim API for daemon workers.
- Before implementation, the CLI enqueue test failed because `resume-cli import` did not accept `--enqueue`.
- Before implementation, the daemon worker test failed because `resume-daemon run` did not accept `--work-imports-once`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-cli -p resume-daemon --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 32 meta-store tests passed, including atomic import worker claim and attempted-task exclusion.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 14 import/search tests passed, including enqueue without foreground import and preserved scan budget metadata.
- `cargo test -p resume-daemon`: exit 0; daemon identity, IPC, foreground once, queued import worker, and failure-continuation tests passed.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p meta-store -p import-pipeline -p resume-cli -p resume-daemon --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Sub-agent review fix:

- Feynman found three issues before commit: queued imports dropped scan-budget metadata, worker task selection was not atomic, and a single import failure aborted the worker pass. The implementation now persists initial budget metadata, uses an atomic SQLite `UPDATE ... RETURNING` claim API, excludes attempted task IDs during a one-shot worker pass to avoid immediate retry loops, and counts failures while continuing to later queued tasks.

Scope note:

- S43 does not add a long-running scheduler loop, authenticated import command IPC, progress streaming, cancellation, background OCR/vector workers, multi-process stress proof, real whole-machine witness scans, or Windows/macOS service validation. Those remain incomplete.

### S44

Design note:

- S44 adds `resume-daemon run --foreground --work-imports` as a long-running
  local import scheduler. It polls queued import tasks after startup, keeps
  new queued tasks immediately claimable, applies a fixed retry backoff to
  retryable failures so bad roots are not hot-looped, records terminal task
  status at import finish time, heartbeats active `Running` import tasks, and
  recovers stale `Running` import tasks to retryable after a daemon crash/stall
  window.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_processes_task_enqueued_after_startup -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_worker_claim_respects_retryable_due_time_without_delaying_queued_tasks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store stale_running_import_tasks_can_be_recovered_for_worker_retry -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store running_import_task_heartbeat_prevents_stale_recovery -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_backs_off_retryable_failures -- --exact
```

Output summary:

- Before implementation, the scheduler test failed because `resume-daemon run`
  did not accept `--work-imports`.
- Before implementation, the retry-due and stale-running meta-store tests
  failed because the worker claim API had no retryable due cutoff and there was
  no stale running import recovery API.
- Before implementation, the running-task heartbeat test failed because there
  was no worker heartbeat API to keep active long imports out of stale recovery.
- Before the backoff fix, the bad-root scheduler test failed with 30 retryable
  failures across 30 worker ticks instead of one failure followed by backoff.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_worker_claim_respects_retryable_due_time_without_delaying_queued_tasks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store stale_running_import_tasks_can_be_recovered_for_worker_retry -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store running_import_task_heartbeat_prevents_stale_recovery -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_backs_off_retryable_failures -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_recovers_stale_running_import_task -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_processes_task_enqueued_after_startup -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
```

Output summary:

- The retry-due meta-store test passed and confirms retryable task due time no
  longer delays fresh queued work.
- The stale-running meta-store test passed and confirms stale `Running` import
  tasks can be moved to `FailedRetryable` with finished/updated timestamps.
- The running-task heartbeat meta-store test passed and confirms active
  `Running` imports can refresh `updated_at` to avoid stale recovery.
- The scheduler backoff test passed and confirms a missing root produces one
  retryable failure across 30 short ticks, without leaking local paths.
- The scheduler stale recovery test passed and confirms daemon loop recovery
  emits only redacted counts and leaves the task retryable instead of stuck
  running.
- `cargo test -p import-pipeline`: exit 0; import-pipeline tests passed after
  terminal import-task timestamps were moved to finish time.
- `cargo test -p meta-store`: exit 0; 35 meta-store tests passed.
- `cargo test -p resume-daemon`: exit 0; daemon identity, IPC, one-shot worker,
  long-running scheduler, retry backoff, and stale recovery tests passed.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo clippy -p meta-store -p import-pipeline -p resume-daemon --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests and doc-tests passed.

Sub-agent review fix:

- Bernoulli/Cicero found a P1 hot retry loop: `--work-imports` reset the
  attempted set every tick, so a retryable bad root could be retried forever at
  the worker interval. The implementation now separates queued eligibility
  from retryable due time and applies a fixed 60-second daemon retry backoff.
- Cicero also found a P1 stale-running lifecycle gap after daemon crash. The
  implementation now heartbeats active running import tasks and recovers stale
  running import tasks after a 15-minute no-heartbeat window in the
  long-running worker loop. Cicero's P3 child-cleanup test issue was fixed by
  killing/waiting the daemon child when readiness never appears.
- Halley found that retry backoff was still measured from import start time for
  long failed imports, and that stale recovery could steal an active long import
  without a worker lease. Terminal task status is now stamped at import finish
  time, and active daemon imports now refresh a running-task heartbeat before
  they can be considered stale.

Scope note:

- S44 does not combine the IPC status server with the worker loop, add an
  authenticated import command IPC endpoint, stream import progress, implement
  user cancellation, make retry policy configurable, enforce a packaged
  singleton service lifecycle, run OCR/vector workers, execute real
  whole-machine witness scans, or validate macOS/Windows service lifecycle
  behavior. Those remain incomplete.

### S45

Design note:

- S45 removes the staged daemon restriction that forced status IPC and the
  import worker loop to run separately. `resume-daemon run --foreground
  --work-imports --ipc-listen 127.0.0.1:0` now starts the import worker on a
  separate local metadata connection while the main thread serves loopback
  `/status`. Test hooks still use `--max-requests` and `--max-worker-ticks` for
  deterministic shutdown; production mode keeps both loops running.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_serves_status_while_import_worker_processes_late_queued_task -- --exact
```

Output summary:

- Before implementation, the daemon did not print an IPC endpoint because
  `--work-imports --ipc-listen` was rejected by argument validation.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_serves_status_while_import_worker_processes_late_queued_task -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_does_not_start_import_worker_when_ipc_bind_fails -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_rejects_worker_tick_limit_in_combined_ipc_worker_mode -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings
```

Output summary:

- The focused S45 test passed and confirms a task queued after daemon startup
  is processed by the worker while status IPC remains available and reports
  `searchable_documents: 2` without leaking local paths.
- The bind-failure test passed and confirms the import worker is not started
  before IPC bind succeeds, so a failed combined daemon startup leaves queued
  tasks untouched.
- The combined-mode tick-limit test passed and confirms `--max-worker-ticks` is
  rejected with IPC to avoid a worker exiting while `/status` still reports
  healthy service.
- `cargo test -p resume-daemon`: exit 0; identity, status IPC, standalone
  worker, combined IPC plus import worker, bind failure, and combined-mode
  validation tests passed.
- `cargo clippy -p resume-daemon --all-targets -- -D warnings`: exit 0.

Sub-agent review fix:

- Gauss found a P1 where the worker could exit while IPC continued serving
  healthy status. The combined IPC loop now monitors the worker result channel
  and returns an error if the worker exits while IPC is still running; test-only
  `--max-worker-ticks` is rejected in combined mode.
- Gauss found a P2 where the worker started before IPC bind succeeded. The
  daemon now binds and prints the IPC endpoint before spawning the worker, and
  the bind-failure test confirms queued imports remain untouched.
- Gauss found P3 test cleanup and post-import leakage proof gaps. Endpoint
  readiness failure now kills/waits the child, and the S45 test checks both the
  initial and post-import status responses for local path leakage.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests and doc-tests passed,
  including S45 combined daemon IPC plus import worker coverage.

Scope note:

- S45 does not add authenticated command IPC endpoints, import cancellation,
  progress streaming, configurable retry policy, packaged singleton service
  enforcement, daemon OCR/vector workers, real whole-machine witness scans, or
  macOS/Windows service validation. Those remain incomplete.

### S46

Design target:

- S46 adds the first authenticated local command IPC surface for import
  enqueue. This closes part of the P0 control-plane gap by allowing local
  agents/UI callers to submit explicit import roots through the daemon instead
  of writing SQLite internals directly.
- The endpoint remains loopback-only, uses a locally generated bearer token
  stored under the data directory, keeps responses path/token-redacted, and
  only queues import tasks plus initial scan-scope metadata. It does not run
  OCR/vector workers or claim product completion.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_authenticates_and_queues_import_command_over_ipc -- --exact
```

Output summary:

- Failed before implementation because the daemon did not create
  `ipc.auth` and did not expose an authenticated import command IPC endpoint:
  the test panicked while reading the missing token file.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_authenticates_and_queues_import_command_over_ipc -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_requires_bearer_token_for_import_command_ipc -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_task_and_scan_scope_insert_atomically_for_daemon_command_ipc -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_rejects_malformed_ipc_request_without_stopping -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_rejects_import_command_for_running_root_without_rewriting_scope -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_import_command_ipc_feeds_running_import_worker_loop -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_rejects_wrong_bearer_token_for_import_command_ipc -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_repairs_existing_weak_ipc_token_permissions -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-daemon --all-targets -- -D warnings
```

Output summary:

- The authorized `POST /imports` test passed: a request with the local bearer
  token returns `202 Accepted`, creates one queued import task, persists an
  explicit scan scope with file budget metadata, and omits the data directory,
  root path, canonical root path, token, and raw resume text from the response.
- The unauthorized `POST /imports` test passed: missing bearer token returns
  `401 Unauthorized`, does not enqueue an import task, and does not leak local
  paths or resume text.
- The meta-store atomic insert test passed, covering daemon command enqueue
  inserting `ImportTask` and `ImportScanScope` in one SQLite transaction so the
  import worker cannot claim a scope-less task.
- Malformed IPC request testing passed: invalid `Content-Length` returns a
  per-request `400 Bad Request`, and a subsequent `/status` request still
  succeeds, proving the daemon stays alive.
- Running-root duplicate testing passed: authenticated `POST /imports` returns
  `409 Conflict` instead of silently accepting/reusing a live running task.
- Combined daemon testing passed: `POST /imports` into a daemon running both
  IPC and the import worker was processed to completion, with two searchable
  synthetic documents and no path/token leakage in responses.
- Wrong-token and existing weak-token-permission tests passed: bad bearer
  tokens do not enqueue tasks, and pre-existing Unix `0644` `ipc.auth` files
  are repaired to `0600` before use.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 13 IPC tests passed,
  covering redacted status, non-loopback rejection, 404 path handling,
  authenticated import command IPC, malformed-request liveness, wrong token,
  token permissions, running duplicate conflict, combined IPC plus import
  worker, bind failure, and worker tick-limit rejection.
- `cargo test -p meta-store`: exit 0; 36 tests passed, including atomic
  import task plus scan scope insertion.
- `cargo test -p resume-daemon`: exit 0; daemon identity, IPC, and worker
  scheduler tests passed.
- `cargo clippy -p meta-store -p resume-daemon --all-targets -- -D warnings`:
  exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  exit 0.
- `cargo test --workspace`: exit 0; all workspace tests and doc-tests passed,
  including the S46 daemon import command IPC coverage.
- The obsolete-reference marker scan exited 1 with no matches, confirming those
  obsolete preliminary references remain absent from the repository.

Sub-agent orchestration:

- Subagent-driven guidance was used as implementation discipline. After local
  implementation and verification, a separate Codex sub-agent review was
  spawned for the S46 diff to check command IPC correctness/security risks.
- The sub-agent found four actionable issues before commit: task/scope enqueue
  was not atomic in combined mode, malformed requests could terminate the
  daemon, existing weak-permission token files were trusted, and duplicate
  running imports returned misleading `202 Accepted`. All four were fixed and
  covered by the implementation checks above.

Scope note:

- S46 does not add search/detail IPC endpoints, CLI import-over-IPC UX, token
  rotation/revocation, import cancellation/progress streaming, singleton
  service lifecycle enforcement, daemon OCR/vector workers, real whole-machine
  witness scans, or macOS/Windows service validation. Those remain incomplete.

### S9

TDD red checks:

```bash
cargo test -p meta-store import_task_status_updates_support_completion_and_retry
cargo test -p resume-cli --test s9_import_search
```

Output summary:

- `meta-store` failed before implementation because `MetaStore::update_import_task_status` did not exist.
- `resume-cli --test s9_import_search` failed before implementation because `import` still left tasks queued and did not build a search index.

Implementation and review checks:

```bash
cargo test -p meta-store
cargo test -p resume-cli --test s4_cli
cargo test -p resume-cli --test s8_search_cli
cargo test -p resume-cli --test s9_import_search
```

Output summary:

- `cargo test -p meta-store`: exit 0; 17 tests passed, including import task completion/retry lifecycle, root-based pending task lookup, timestamp lifecycle rejection, resume version lookup by document, and existing SQLite recovery tests.
- `cargo test -p resume-cli --test s4_cli`: exit 0; S4 no-index behavior and no-path-leak import behavior still passed after synchronous import execution.
- `cargo test -p resume-cli --test s8_search_cli`: exit 0; CLI search still read an existing full-text index.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 3 S9 tests passed, covering synthetic docx/PDF import, OCR_REQUIRED scanned PDF, reopened full-text snapshot search, failed-retryable task retry, live running task non-takeover, and empty-root import preserving prior searchable documents.

Acceptance:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run -p resume-cli -- import --root tests/fixtures/resumes
cargo run -p resume-cli -- status
cargo run -p resume-cli -- search "Java"
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests, including the 3 S9 CLI smoke tests, passed.
- `resume-cli import --root tests/fixtures/resumes`: exit 0; completed an import task for 3 synthetic files, with 2 searchable documents, 1 OCR-required document, 0 failed documents, and 0 scan errors.
- `resume-cli status`: exit 0; reported `indexed documents: 2`, `searchable documents: 2`, `ocr queue: 1`, `import tasks queued: 0`, `import tasks recoverable: 0`, `index health: ready`, and full-text index available.
- `resume-cli search "Java"`: exit 0; returned 2 results: `synthetic-java-engineer.docx` and `synthetic-java-platform.pdf`, with snippets from synthetic fixture text.

Review notes:

- Sub-agent spec review found no P0 issues and identified missing PROGRESS evidence plus missing retry smoke; both were fixed.
- Sub-agent code-quality review found one P1 around empty-root import clearing the Tantivy index while SQLite still counted searchable documents; the pipeline now rebuilds the full-text index from persisted searchable/partial documents plus newly imported documents, and the S9 CLI test covers this.
- Remaining non-blocking P2: scan errors are counted but not yet persisted as recoverable import diagnostics. This is left for later diagnostics/fault-injection slices.

Scope note:

- S9 completes a synthetic import-to-search smoke loop only. It does not run OCR, generate embeddings, implement field-filter search, claim production-scale performance, or package/release the app.

### S8

TDD red checks:

```bash
cargo test -p index-fulltext
cargo test -p search-planner
cargo test -p resume-cli --test s8_search_cli
```

Output summary:

- `index-fulltext` failed before implementation because `FullTextIndex`, `IndexDocument`, `IndexSection`, and `SearchQuery` were unresolved imports.
- `search-planner` failed before implementation because `plan_search` and `SearchPlan` were unresolved imports.
- `resume-cli --test s8_search_cli` failed before implementation because CLI tests could not seed or read a full-text index.

Implementation checks:

```bash
cargo test -p index-fulltext
cargo test -p search-planner
cargo test -p resume-cli --test s8_search_cli
cargo test -p resume-cli --test s4_cli
```

Output summary:

- `cargo test -p index-fulltext`: exit 0; 7 S8 tests passed, covering committed documents searchable after reload, deleted documents hidden by default, duplicate sections not hiding distinct topN documents, malformed query syntax returning safe results, topN snippets only for returned hits, mixed Chinese-English query matching, and redacted debug output.
- `cargo test -p search-planner`: exit 0; 4 S8 tests passed, covering mixed query planning, debug redaction, empty/too-broad query rejection, and topN limit clamping.
- `cargo test -p resume-cli --test s8_search_cli`: exit 0; CLI search read an existing synthetic full-text index and printed rank, doc_id, version_id, file_name, and snippet without a query label.
- `cargo test -p resume-cli --test s4_cli`: exit 0; no-index search still returned unavailable/results 0 without echoing the query or creating a data directory.

Acceptance:

```bash
cargo fmt --check
cargo test -p index-fulltext
cargo test -p search-planner
cargo run -p resume-cli -- search "Java 支付"
cargo clippy -p index-fulltext -p search-planner -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p index-fulltext`: exit 0; 7 tests passed.
- `cargo test -p search-planner`: exit 0; 4 tests passed.
- `cargo run -p resume-cli -- search "Java 支付"`: exit 0; no local full-text index existed, so CLI returned `search index not available yet` and `results: 0` without fake rows.
- `cargo clippy -p index-fulltext -p search-planner -p resume-cli --all-targets -- -D warnings`: exit 0.

Scope note:

- S8 is only the Tantivy full-text index/search CLI slice. It does not implement import execution, OCR execution, embeddings, vector search, packaging, or S9+ behavior.

Additional workspace regression:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Output summary:

- `cargo test --workspace`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.

### S7

TDD red checks:

```bash
cargo test -p text-normalizer
cargo test -p sectionizer
cargo test -p extractor-rules
```

Output summary:

- `text-normalizer` failed before implementation because `TextNormalizer` and normalized offset mapping APIs were unresolved imports.
- `sectionizer` failed before implementation because `Sectionizer` and section chunk APIs were unresolved imports.
- `extractor-rules` failed before implementation because `extract_strong_fields` and `FieldType` were unresolved imports.

Implementation checks:

```bash
cargo test -p text-normalizer
cargo test -p sectionizer
cargo test -p extractor-rules
```

Output summary:

- `cargo test -p text-normalizer`: exit 0; 5 S7 tests passed, covering mixed Chinese-English whitespace cleanup, table-linearized text, offset mapping across inserted newlines, repeated page header/footer removal, simple OCR spacing repair, bullet preservation, and redacted debug output.
- `cargo test -p sectionizer`: exit 0; 5 S7 tests passed, covering Chinese/English resume heading recognition, fallback paragraph/length chunks including single overlong paragraphs, table-linearized text staying inside the nearest section, character offsets, and redacted debug output.
- `cargo test -p extractor-rules`: exit 0; 4 S7 tests passed, covering strong email, phone, and date-range extraction, normalized values, byte offsets over table-linearized text, and low-confidence candidates not entering strong filtering.

Acceptance:

```bash
cargo fmt --check
cargo test -p text-normalizer
cargo test -p sectionizer
cargo test -p extractor-rules
cargo clippy -p text-normalizer -p sectionizer -p extractor-rules --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p text-normalizer`: exit 0; 5 tests passed.
- `cargo test -p sectionizer`: exit 0; 5 tests passed.
- `cargo test -p extractor-rules`: exit 0; 4 tests passed.
- `cargo clippy -p text-normalizer -p sectionizer -p extractor-rules --all-targets -- -D warnings`: exit 0.

Scope note:

- S7 is only the text cleanup, section fallback, and strong-rule extraction slice. It does not implement import execution, DB writes, indexing, search, OCR execution, embeddings, or S8+ behavior.

Additional workspace regression:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Output summary:

- `cargo test --workspace`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
