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
if (-not ($report.blocked_release_steps -contains "service_install_validation")) {
    Fail "Windows package manifest must keep service_install_validation blocked"
}

$serviceArtifacts = @()
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
    $serviceArtifacts += [ordered]@{
        kind = "msi"
        file = $fileName
        artifact_sha256 = $artifact.sha256
        bytes = $artifact.bytes
        service_validation_status = "not_executed"
    }
}

if ($serviceArtifacts.Count -lt 1) {
    Fail "Windows package manifest is missing required MSI artifact"
}

$msiFile = $serviceArtifacts[0].file
$sha256 = [System.Security.Cryptography.SHA256]::Create()
$manifestSha256 = ConvertTo-LowerHex $sha256.ComputeHash($packageBytes)

$plannedActions = @(
    [ordered]@{
        action = "install"
        command = "sc.exe"
        target_artifact = $msiFile
        dry_run_intent = "register Windows Service after administrator-elevated MSI install and verify binary binding"
        requires_approval = $true
        action_status = "blocked"
    },
    [ordered]@{
        action = "start"
        command = "sc.exe"
        target_artifact = $msiFile
        dry_run_intent = "start service and verify daemon IPC health"
        requires_approval = $true
        action_status = "blocked"
    },
    [ordered]@{
        action = "status"
        command = "sc.exe"
        target_artifact = $msiFile
        dry_run_intent = "query service status on release Windows runner"
        requires_approval = $true
        action_status = "blocked"
    },
    [ordered]@{
        action = "stop"
        command = "sc.exe"
        target_artifact = $msiFile
        dry_run_intent = "stop service and verify daemon shutdown"
        requires_approval = $true
        action_status = "blocked"
    },
    [ordered]@{
        action = "recovery"
        command = "sc.exe"
        target_artifact = $msiFile
        dry_run_intent = "configure and prove restart-after-kill recovery policy"
        requires_approval = $true
        action_status = "blocked"
    },
    [ordered]@{
        action = "uninstall"
        command = "sc.exe"
        target_artifact = $msiFile
        dry_run_intent = "delete service registration while preserving user data"
        requires_approval = $true
        action_status = "blocked"
    },
    [ordered]@{
        action = "rollback"
        command = "sc.exe"
        target_artifact = $msiFile
        dry_run_intent = "force service install/start failure and verify rollback state restoration"
        requires_approval = $true
        action_status = "blocked"
    }
)

$document = [ordered]@{
    schema_version = "release.windows_service_lifecycle_plan.v1"
    version = $Version
    execution_mode = "dry_run"
    service_lifecycle_status = "blocked"
    evidence_boundary = "dry_run_no_windows_service_registration"
    windows_package_manifest_sha256 = $manifestSha256
    service_manager = "sc.exe"
    admin_elevation = "required_not_observed"
    release_runner = "windows_required_not_observed"
    registration_status = "not_registered"
    recovery_validation_status = "blocked"
    rollback_validation_status = "blocked"
    service_artifacts = $serviceArtifacts
    planned_actions = $plannedActions
    blocked_release_steps = @(
        "windows_service_install",
        "windows_service_start",
        "windows_service_status",
        "windows_service_stop",
        "windows_service_recovery",
        "windows_service_uninstall",
        "windows_service_rollback"
    )
    prohibited_public_material = @(
        "service_tokens",
        "administrator_passwords",
        "local_paths",
        "raw_service_logs",
        "raw_resume_data",
        "diagnostic_packages",
        "model_artifact_caches"
    )
    notes = "Dry-run operator plan only. It does not register, start, stop, query, recover, uninstall, or roll back a Windows service; release-runner transcripts are required before stable release."
}

$outDir = Split-Path -Parent $Out
if (-not [string]::IsNullOrWhiteSpace($outDir)) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
}

$document | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Out -Encoding utf8NoBOM
Write-Output $Out
