param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$WindowsPackageManifest,

    [Parameter(Mandatory = $true)]
    [string]$Out,

    [switch]$DryRun
)

$ErrorActionPreference = "Stop"

function Fail([string]$Message) {
    Write-Error $Message
    exit 1
}

function Require-Basename([object]$Value, [string]$Label) {
    if (-not ($Value -is [string]) -or [string]::IsNullOrWhiteSpace($Value)) {
        Fail "$Label must be a non-empty string"
    }
    if ([System.IO.Path]::GetFileName($Value) -ne $Value) {
        Fail "$Label must be a basename"
    }
    return $Value
}

function ConvertTo-LowerHex([byte[]]$Bytes) {
    $builder = [System.Text.StringBuilder]::new()
    foreach ($byte in $Bytes) {
        [void]$builder.Append($byte.ToString("x2"))
    }
    return $builder.ToString()
}

if (-not $DryRun) {
    Fail "only -DryRun is supported without release-runner approval"
}

if ($Version -notmatch '^v[0-9]+[.][0-9]+[.][0-9]+$') {
    Fail "version must look like vX.Y.Z"
}

if (-not (Test-Path -LiteralPath $WindowsPackageManifest -PathType Leaf)) {
    Fail "Windows package manifest does not exist"
}

$manifestPath = (Resolve-Path -LiteralPath $WindowsPackageManifest).ProviderPath
$packageBytes = [System.IO.File]::ReadAllBytes($manifestPath)
$packageText = [System.Text.Encoding]::UTF8.GetString($packageBytes)

try {
    $report = $packageText | ConvertFrom-Json
} catch {
    Fail "Windows package manifest must be UTF-8 JSON"
}

if ($report.schema_version -ne "release.windows_package.v1") {
    Fail "Windows package manifest schema_version must be release.windows_package.v1"
}
if ($report.version -ne $Version) {
    Fail "Windows package manifest version does not match requested version"
}
if ($report.packaging_status -ne "unsigned_dry_run") {
    Fail "Windows package manifest must be an unsigned dry run"
}
if ($report.installer_kind -ne "msi") {
    Fail "Windows package manifest installer_kind must be msi"
}
if ($report.install_location -ne "ProgramFilesFolder/resume-ir") {
    Fail "Windows package manifest install_location must be ProgramFilesFolder/resume-ir"
}
if ($report.signing_status -ne "unsigned") {
    Fail "Windows package manifest signing_status must be unsigned"
}
if (-not ($report.blocked_release_steps -contains "installer_lifecycle_validation")) {
    Fail "Windows package manifest must keep installer_lifecycle_validation blocked"
}

$installerArtifacts = @()
foreach ($artifact in $report.artifacts) {
    if ($artifact.kind -ne "msi") {
        continue
    }
    $fileName = Require-Basename $artifact.file "Windows package artifact file"
    if (-not ($artifact.sha256 -is [string]) -or $artifact.sha256 -notmatch '^[0-9a-f]{64}$') {
        Fail "Windows package artifact sha256 must be lowercase hex"
    }
    if (-not ($artifact.bytes -is [long] -or $artifact.bytes -is [int]) -or $artifact.bytes -le 0) {
        Fail "Windows package artifact bytes must be a positive integer"
    }
    $installerArtifacts += [ordered]@{
        kind = "msi"
        file = $fileName
        artifact_sha256 = $artifact.sha256
        bytes = $artifact.bytes
    }
}

if ($installerArtifacts.Count -lt 1) {
    Fail "Windows package manifest is missing required MSI artifact"
}

$msiFile = $installerArtifacts[0].file
$sha256 = [System.Security.Cryptography.SHA256]::Create()
$manifestSha256 = ConvertTo-LowerHex $sha256.ComputeHash($packageBytes)

$plannedActions = @(
    [ordered]@{
        action = "install"
        command = "msiexec.exe"
        target_artifact = $msiFile
        dry_run_intent = "validate administrator-elevated MSI install on release runner"
        requires_approval = $true
        action_status = "blocked"
    },
    [ordered]@{
        action = "upgrade"
        command = "msiexec.exe"
        target_artifact = $msiFile
        dry_run_intent = "install prior version, upgrade, and verify binary replacement"
        requires_approval = $true
        action_status = "blocked"
    },
    [ordered]@{
        action = "repair"
        command = "msiexec.exe"
        target_artifact = $msiFile
        dry_run_intent = "run MSI repair and verify installed-file integrity"
        requires_approval = $true
        action_status = "blocked"
    },
    [ordered]@{
        action = "uninstall"
        command = "msiexec.exe"
        target_artifact = $msiFile
        dry_run_intent = "uninstall MSI and verify user-data preservation"
        requires_approval = $true
        action_status = "blocked"
    },
    [ordered]@{
        action = "rollback"
        command = "msiexec.exe"
        target_artifact = $msiFile
        dry_run_intent = "force MSI failure and verify rollback state restoration"
        requires_approval = $true
        action_status = "blocked"
    }
)

$document = [ordered]@{
    schema_version = "release.windows_installer_lifecycle_plan.v1"
    version = $Version
    execution_mode = "dry_run"
    installer_lifecycle_status = "blocked"
    evidence_boundary = "dry_run_no_windows_installer_execution"
    windows_package_manifest_sha256 = $manifestSha256
    installer_engine = "msiexec.exe"
    admin_elevation = "required_not_observed"
    release_runner = "windows_required_not_observed"
    installation_status = "not_installed"
    rollback_validation_status = "blocked"
    installer_artifacts = $installerArtifacts
    planned_actions = $plannedActions
    blocked_release_steps = @(
        "windows_msi_install",
        "windows_msi_upgrade",
        "windows_msi_repair",
        "windows_msi_uninstall",
        "windows_msi_rollback"
    )
    prohibited_public_material = @(
        "installer_tokens",
        "administrator_passwords",
        "local_paths",
        "raw_installer_logs",
        "raw_resume_data",
        "diagnostic_packages",
        "model_artifact_caches"
    )
    notes = "Dry-run operator plan only. It does not execute installer lifecycle commands or clear release blockers; release-runner transcripts are required before stable release."
}

$outDir = Split-Path -Parent $Out
if (-not [string]::IsNullOrWhiteSpace($outDir)) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
}

$document | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Out -Encoding utf8NoBOM
Write-Output $Out
