param(
    [Parameter(Mandatory = $true)]
    [string] $Version,

    [Parameter(Mandatory = $true)]
    [string] $TargetDir,

    [Parameter(Mandatory = $true)]
    [string] $OutDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Fail([string] $Message) {
    throw $Message
}

function Resolve-RequiredFile([string] $Directory, [string] $Name) {
    $path = Join-Path $Directory $Name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Fail "missing release binary: $Name"
    }
    return (Resolve-Path -LiteralPath $path).Path
}

function Escape-Xml([string] $Value) {
    return [System.Security.SecurityElement]::Escape($Value)
}

function File-Record([string] $Kind, [string] $Path) {
    $item = Get-Item -LiteralPath $Path
    $hash = Get-FileHash -LiteralPath $Path -Algorithm SHA256
    return [ordered]@{
        kind = $Kind
        file = $item.Name
        sha256 = $hash.Hash.ToLowerInvariant()
        bytes = $item.Length
    }
}

if ($Version -notmatch '^v[0-9]+[.][0-9]+[.][0-9]+$') {
    Fail "version must look like vX.Y.Z"
}

if (-not $IsWindows) {
    Fail "Windows packaging requires Windows"
}

$wix = Get-Command wix -ErrorAction SilentlyContinue
if ($null -eq $wix) {
    Fail "wix is required; install the WiX .NET tool before running this script"
}

if (-not (Test-Path -LiteralPath $TargetDir -PathType Container)) {
    Fail "target directory does not exist"
}

$target = (Resolve-Path -LiteralPath $TargetDir).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$out = (Resolve-Path -LiteralPath $OutDir).Path

$versionNumber = $Version.Substring(1)
$versionParts = $versionNumber.Split(".")
if ([int] $versionParts[0] -gt 255 -or [int] $versionParts[1] -gt 255 -or [int] $versionParts[2] -gt 65535) {
    Fail "MSI version parts must fit Windows Installer limits"
}

$cli = Resolve-RequiredFile $target "resume-cli.exe"
$daemon = Resolve-RequiredFile $target "resume-daemon.exe"
$benchmark = Resolve-RequiredFile $target "resume-benchmark.exe"

$msi = Join-Path $out "resume-ir-$Version-windows.msi"
$manifest = Join-Path $out "windows-package.json"
$work = Join-Path ([System.IO.Path]::GetTempPath()) ("resume-ir-windows-package-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $work | Out-Null

try {
    $wxs = Join-Path $work "resume-ir.wxs"
    $cliXml = Escape-Xml $cli
    $daemonXml = Escape-Xml $daemon
    $benchmarkXml = Escape-Xml $benchmark

    @"
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
  <Package Id="io.github.frankqdwang.resumeir" Name="resume-ir" Manufacturer="FrankQDWang" Version="$versionNumber" UpgradeCode="1E58F6BC-B687-4F8C-A4E1-D672F756FD12">
    <MajorUpgrade DowngradeErrorMessage="A newer version of resume-ir is already installed." />
    <MediaTemplate EmbedCab="yes" />
    <Feature Id="Main" Title="resume-ir" Level="1">
      <ComponentGroupRef Id="ResumeIrBinaries" />
    </Feature>
  </Package>

  <Fragment>
    <StandardDirectory Id="ProgramFilesFolder">
      <Directory Id="INSTALLFOLDER" Name="resume-ir" />
    </StandardDirectory>
  </Fragment>

  <Fragment>
    <ComponentGroup Id="ResumeIrBinaries" Directory="INSTALLFOLDER">
      <Component Id="ResumeCliComponent" Guid="2C9BD3E7-624D-4795-8EAA-C26B2C41EB70">
        <File Id="ResumeCliExe" Source="$cliXml" KeyPath="yes" />
      </Component>
      <Component Id="ResumeDaemonComponent" Guid="B4F77F90-42DF-46D3-B2E0-C804CE5D89EF">
        <File Id="ResumeDaemonExe" Source="$daemonXml" KeyPath="yes" />
      </Component>
      <Component Id="ResumeBenchmarkComponent" Guid="0371613E-A0AC-498D-81FB-FE3079434411">
        <File Id="ResumeBenchmarkExe" Source="$benchmarkXml" KeyPath="yes" />
      </Component>
    </ComponentGroup>
  </Fragment>
</Wix>
"@ | Set-Content -LiteralPath $wxs -Encoding UTF8

    & wix build -o $msi $wxs | Out-String | Write-Host
    if ($LASTEXITCODE -ne 0) {
        Fail "wix build failed"
    }

    if (-not (Test-Path -LiteralPath $msi -PathType Leaf)) {
        Fail "Windows MSI was not created"
    }

    $document = [ordered]@{
        schema_version = "release.windows_package.v1"
        version = $Version
        packaging_status = "unsigned_dry_run"
        installer_kind = "msi"
        install_location = "ProgramFilesFolder/resume-ir"
        signing_status = "unsigned"
        artifacts = @(
            File-Record "msi" $msi
        )
        blocked_release_steps = @(
            "signing",
            "github_release_upload",
            "installer_lifecycle_validation",
            "service_install_validation",
            "macos_notarization"
        )
        notes = "Unsigned Windows MSI dry run only; signing, GitHub Release upload, service install validation, and installer lifecycle validation remain blocked until explicit release approval and credentials are available."
    }

    $json = $document | ConvertTo-Json -Depth 8
    Set-Content -LiteralPath $manifest -Value $json -Encoding UTF8
    Write-Output $manifest
}
finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
