<#
.SYNOPSIS
  Packs the release exe into an .msix for the Microsoft Store.

.EXAMPLE
  # Real submission: use the values from Partner Center > Product identity
  .\build-msix.ps1 -IdentityName "12345Metamug.Plunger" `
                   -Publisher "CN=AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE" `
                   -PublisherDisplayName "Metamug"

.EXAMPLE
  # Local dry run with placeholder identity (not submittable)
  .\build-msix.ps1 -Dev

The Store signs the package itself, so upload the unsigned .msix. Requires
makeappx.exe from the Windows SDK or from packaging\tools (fetch-buildtools.ps1).
#>
param(
    [string]$IdentityName,
    [string]$Publisher,
    [string]$PublisherDisplayName,
    [string]$Version,
    [switch]$Dev
)
$ErrorActionPreference = 'Stop'

$here    = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo    = [System.IO.Path]::GetFullPath((Join-Path $here '..\..'))
$exe     = Join-Path $repo 'target\release\plunger.exe'
$assets  = Join-Path $here 'Assets'
$outDir  = Join-Path $repo 'target\msix'
$stage   = Join-Path $outDir 'stage'

if (-not (Test-Path $exe))    { throw "Release exe not found: $exe. Run 'cargo build --release' first." }
if (-not (Test-Path $assets)) { throw "Assets not found. Run packaging\icons\make-icons.ps1 first." }

if ($Dev) {
    if (-not $IdentityName)         { $IdentityName = 'Metamug.Plunger.Dev' }
    if (-not $Publisher)            { $Publisher = 'CN=Metamug Dev' }
    if (-not $PublisherDisplayName) { $PublisherDisplayName = 'Metamug (dev)' }
}
foreach ($p in 'IdentityName', 'Publisher', 'PublisherDisplayName') {
    if (-not (Get-Variable $p -ValueOnly)) { throw "-$p is required (copy it from Partner Center > Product identity), or pass -Dev for a local dry run." }
}
if ($Publisher -notmatch '^CN=') { throw "-Publisher must look like 'CN=...' exactly as shown in Partner Center." }

# MSIX versions are four numbers; the Store reserves the last one (must be 0).
if (-not $Version) {
    $cargo = Get-Content (Join-Path $repo 'Cargo.toml') -Raw
    if ($cargo -notmatch '(?m)^version\s*=\s*"(\d+)\.(\d+)\.(\d+)"') { throw 'Could not read the version from Cargo.toml' }
    $Version = "$($Matches[1]).$($Matches[2]).$($Matches[3]).0"
}
if ($Version -notmatch '^\d+\.\d+\.\d+\.0$') { throw "Version '$Version' must be A.B.C.0 (the Store reserves the fourth number)." }

# Find makeappx: Windows SDK first, then a local copy fetched by fetch-buildtools.ps1.
function Find-MakeAppx {
    $roots = @(
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin",
        (Join-Path $here '..\tools')
    )
    foreach ($r in $roots) {
        if (Test-Path $r) {
            $hit = Get-ChildItem $r -Recurse -Filter makeappx.exe -ErrorAction SilentlyContinue |
                   Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object FullName -Descending | Select-Object -First 1
            if ($hit) { return $hit.FullName }
        }
    }
    return $null
}

# --- stage ------------------------------------------------------------------
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item $exe $stage
Copy-Item $assets (Join-Path $stage 'Assets') -Recurse

$manifest = Get-Content (Join-Path $here 'AppxManifest.template.xml') -Raw
$manifest = $manifest.Replace('{{IDENTITY_NAME}}', $IdentityName).
                      Replace('{{PUBLISHER}}', $Publisher).
                      Replace('{{PUBLISHER_DISPLAY_NAME}}', $PublisherDisplayName).
                      Replace('{{VERSION}}', $Version)
if ($manifest -match '\{\{') { throw 'Unfilled placeholder left in the manifest.' }
Set-Content -Path (Join-Path $stage 'AppxManifest.xml') -Value $manifest -Encoding UTF8

# Sanity checks that don't need the SDK: well-formed XML, every referenced file exists.
[xml]$xml = Get-Content (Join-Path $stage 'AppxManifest.xml') -Raw
$refs = [regex]::Matches($manifest, '(?:Logo|Square\d+x\d+Logo|Wide\d+x\d+Logo)(?:="|>)(Assets\\[^"<]+)') | ForEach-Object { $_.Groups[1].Value }
foreach ($r in ($refs | Sort-Object -Unique)) {
    if (-not (Test-Path (Join-Path $stage $r))) { throw "Manifest references a missing file: $r" }
}
Write-Host "Manifest OK: $IdentityName $Version ($($refs.Count) asset references checked)"

# --- pack -------------------------------------------------------------------
$makeappx = Find-MakeAppx
if (-not $makeappx) {
    Write-Warning "makeappx.exe not found, so the package was staged but not packed."
    Write-Warning "Staged folder: $stage"
    Write-Warning "Install the Windows SDK, or run packaging\msix\fetch-buildtools.ps1, then re-run this script."
    exit 2
}
$msix = Join-Path $outDir "Plunger_$Version`_x64.msix"
if (Test-Path $msix) { Remove-Item $msix -Force }
& $makeappx pack /d $stage /p $msix /o | Write-Host
if ($LASTEXITCODE -ne 0) { throw "makeappx failed with exit code $LASTEXITCODE" }
Write-Host ("Built {0} ({1:N2} MB)" -f $msix, ((Get-Item $msix).Length / 1MB))
