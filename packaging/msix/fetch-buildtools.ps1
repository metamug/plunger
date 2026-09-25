<#
.SYNOPSIS
  Downloads Microsoft's Windows SDK Build Tools (makeappx, signtool) as a NuGet
  package into packaging\tools, so no full Windows SDK install is needed.

  Source: https://www.nuget.org/packages/Microsoft.Windows.SDK.BuildTools
  This is a download of a Microsoft-published package (well over 100 MB); it is
  extracted only into packaging\tools, which is git-ignored.
#>
$ErrorActionPreference = 'Stop'
$here  = Split-Path -Parent $MyInvocation.MyCommand.Path
$tools = [System.IO.Path]::GetFullPath((Join-Path $here '..\tools'))
$id    = 'microsoft.windows.sdk.buildtools'

$index = Invoke-RestMethod "https://api.nuget.org/v3-flatcontainer/$id/index.json"
$version = ($index.versions | Where-Object { $_ -notmatch '-' } | Select-Object -Last 1)
if (-not $version) { throw 'No stable version found on nuget.org' }

New-Item -ItemType Directory -Force -Path $tools | Out-Null
$nupkg = Join-Path $tools "$id.$version.nupkg.zip"
$url = "https://api.nuget.org/v3-flatcontainer/$id/$version/$id.$version.nupkg"
Write-Host "Downloading $url"
Invoke-WebRequest $url -OutFile $nupkg
Expand-Archive $nupkg -DestinationPath (Join-Path $tools "$id.$version") -Force
Remove-Item $nupkg
$hit = Get-ChildItem $tools -Recurse -Filter makeappx.exe | Where-Object { $_.FullName -match '\\x64\\' } | Select-Object -First 1
if (-not $hit) { throw 'makeappx.exe not found in the package' }
Write-Host "Ready: $($hit.FullName)"
