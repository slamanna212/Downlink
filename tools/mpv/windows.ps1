# UNTESTED on Windows (the pin was verified from Linux: download + SHA-256 only).
# Fetch the pinned zhongfly GPL libmpv build into <OutDir> (PLAN §4.10).
# Usage: tools\mpv\windows.ps1 -OutDir <dir>
param([Parameter(Mandatory = $true)][string]$OutDir)
$ErrorActionPreference = 'Stop'

# Read pins.env (KEY=VALUE lines).
$pins = @{}
Get-Content (Join-Path $PSScriptRoot 'pins.env') | ForEach-Object {
  if ($_ -match '^\s*([A-Z0-9_]+)=(.*)$') { $pins[$Matches[1]] = $Matches[2].Trim() }
}
$Tag = $pins['MPV_WIN_TAG']; $Asset = $pins['MPV_WIN_ASSET']; $Sha = $pins['MPV_WIN_SHA256']
if (-not $Tag -or -not $Asset -or -not $Sha) { throw 'pins.env is missing MPV_WIN_* entries' }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$Archive = Join-Path $env:TEMP $Asset
Invoke-WebRequest -Uri "https://github.com/zhongfly/mpv-winbuild/releases/download/$Tag/$Asset" -OutFile $Archive
$actual = (Get-FileHash $Archive -Algorithm SHA256).Hash.ToLower()
if ($actual -ne $Sha.ToLower()) { throw "SHA-256 mismatch for $Asset (got $actual)" }

& 7z x $Archive "-o$OutDir" -y | Out-Null
$dll = Get-ChildItem -Path $OutDir -Recurse -Filter 'libmpv-2.dll' | Select-Object -First 1
if (-not $dll) { throw 'libmpv-2.dll not found in the archive' }
if ($dll.DirectoryName -ne (Resolve-Path $OutDir).Path) { Copy-Item $dll.FullName -Destination $OutDir -Force }
Write-Host "staged $($dll.Name) from $Tag into $OutDir"
