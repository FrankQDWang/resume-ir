#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required runtime bundle policy text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  if grep -Fq -- "$text" "$file"; then
    fail "$file still contains obsolete runtime bundle policy text: $text"
  fi
}

license_doc="LICENSES/README.md"
dependency_doc="02_execution_plan_执行方案/08_依赖许可与参考资料.md"
tech_stack_doc="02_execution_plan_执行方案/01_技术栈决策.md"
worker_runbook="docs/runbooks/ocr-embedding-workers.md"
release_runbook="docs/runbooks/release-blockers.md"
current_stage_script="scripts/local/run-current-stage-validation.sh"
current_stage_guard="scripts/ci/check-current-stage-validation.sh"
license_check="scripts/ci/check-licenses.sh"
goal_doc="GOAL.md"
readme_doc="README.md"

for file in "$license_doc" "$dependency_doc" "$tech_stack_doc" "$worker_runbook" "$release_runbook" "$current_stage_script" "$current_stage_guard" "$license_check" "$goal_doc" "$readme_doc"; do
  [ -f "$file" ] || fail "missing runtime bundle policy file: $file"
done

require_text "$license_doc" "repository source license remains GPL-3.0-or-later"
require_text "$license_doc" "statically linked PDFium renderer"
require_text "$license_doc" "Poppler/pdftoppm is not a production desktop runtime"
reject_text "$license_doc" "The current source license is MIT"
reject_text "$license_doc" "MIT is not a product packaging constraint"

require_text "$readme_doc" "Current source code is licensed under GNU GPL v3 or later"
require_text "$readme_doc" "exact source/build identity"
require_text "$readme_doc" "SBOM evidence"
reject_text "$readme_doc" "Current source code is licensed under the MIT License"

require_text "$goal_doc" "bundled-first"
require_text "$goal_doc" "external override"
require_text "$goal_doc" "静态链接的 PDFium"
require_text "$goal_doc" "生产桌面包不再包含或探测"
require_text "$goal_doc" "macOS 与 Windows"
reject_text "$goal_doc" "不默认打包"
reject_text "$goal_doc" "只作为用户本机外部 PDF"

require_text "$dependency_doc" "唯一生产 PDF runtime"
require_text "$dependency_doc" "锁定源码"
require_text "$dependency_doc" "静态链接"
reject_text "$dependency_doc" "当前只走外部命令边界"

require_text "$tech_stack_doc" "静态链接 PDFium"
require_text "$tech_stack_doc" "生产桌面不探测 Poppler/pdftoppm"
require_text "$tech_stack_doc" "runtime_package_binaries_included"
reject_text "$tech_stack_doc" "当前不默认打包 Poppler"

require_text "$worker_runbook" "## Bundled-first runtime decision"
require_text "$worker_runbook" "statically linked PDFium"
require_text "$worker_runbook" "runtime_distribution_mode"
require_text "$worker_runbook" "runtime_binaries_included"
require_text "$worker_runbook" "runtime_package_binaries_included"
require_text "$worker_runbook" "source commit"
require_text "$worker_runbook" "static-pack digest"
require_text "$worker_runbook" "External override"
reject_text "$worker_runbook" "Do not bundle Poppler/pdftoppm"
reject_text "$worker_runbook" "MIT project may call a user-installed Poppler command"
reject_text "$worker_runbook" "repository can keep MIT-licensed source"

require_text "$release_runbook" "only production PDF text and page-rendering runtime"
require_text "$release_runbook" "runtime_distribution_mode"
require_text "$release_runbook" "runtime_binaries_included"
require_text "$release_runbook" "runtime_package_binaries_included"
require_text "$release_runbook" "dependency closure"
require_text "$release_runbook" "source-offer"
reject_text "$release_runbook" "not bundled by default"
reject_text "$release_runbook" "MIT repository's default"

require_text "$current_stage_script" "--runtime-distribution-mode bundled|external"
require_text "$current_stage_script" "runtime_distribution_mode="
require_text "$current_stage_script" "runtime_package_binaries_included="
require_text "$current_stage_script" '"runtime_distribution_mode": "$runtime_distribution_mode"'
require_text "$current_stage_script" '"runtime_package_binaries_included": $runtime_package_binaries_included'
require_text "$current_stage_script" '"runtime_binaries_included": false'

require_text "$current_stage_guard" "--runtime-distribution-mode bundled"
require_text "$current_stage_guard" '"runtime_distribution_mode": "bundled"'
require_text "$current_stage_guard" '"runtime_package_binaries_included": true'
require_text "$current_stage_guard" '"runtime_binaries_included": false'
require_text "$current_stage_guard" "--runtime-distribution-mode external"
require_text "$current_stage_guard" '"runtime_distribution_mode": "external"'
require_text "$current_stage_guard" '"runtime_package_binaries_included": false'

require_text "$license_check" "GPL-compatible license choice"
reject_text "$license_check" "no reviewed permissive license choice"
reject_text "$license_check" 'forbidden_prefixes = ("GPL"'

printf '%s\n' "runtime bundle policy check passed"
