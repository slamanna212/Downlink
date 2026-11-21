# UNTESTED on Windows (the pin was verified from Linux: download, SHA-256 and archive layout).
# Fetch mpv's official pinned libmpv build (GitHub release asset) into <OutDir>.
# Usage: tools\mpv\windows.ps1 -OutDir <dir>
param([Parameter(Mandatory = $true)][string]$OutDir)
$ErrorActionPreference = 'Stop'

# Read pins.env (KEY=VALUE lines).
$pins = @{}
Get-Content (Join-Path $PSScriptRoot 'pins.env') | ForEach-Object {
  if ($_ -match '^\s*([A-Z0-9_]+)=(.*)$') { $pins[$Matches[1]] = $Matches[2].Trim() }
}
$Ver = $pins['MPV_VERSION']; $Asset = $pins['MPV_WIN_ASSET']; $Sha = $pins['MPV_WIN_SHA256']
if (-not $Ver -or -not $Asset -or -not $Sha) { throw 'pins.env is missing MPV_VERSION / MPV_WIN_* entries' }

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$Archive = Join-Path $env:TEMP $Asset
Invoke-WebRequest -Uri "https://github.com/mpv-player/mpv/releases/download/v$Ver/$Asset" -OutFile $Archive
$actual = (Get-FileHash $Archive -Algorithm SHA256).Hash.ToLower()
if ($actual -ne $Sha.ToLower()) { throw "SHA-256 mismatch for $Asset (got $actual)" }

# Keep only what ships: the DLL and its licence (the zip also carries headers, an import lib and Java jars).
$Tmp = Join-Path $env:TEMP "libmpv-$Ver"
if (Test-Path $Tmp) { Remove-Item $Tmp -Recurse -Force }
Expand-Archive -Path $Archive -DestinationPath $Tmp -Force
foreach ($f in 'libmpv-2.dll', 'LICENSE.GPL') {
  $src = Get-ChildItem -Path $Tmp -Recurse -Filter $f | Select-Object -First 1
  if (-not $src) { throw "$f not found in $Asset" }
  Copy-Item $src.FullName -Destination $OutDir -Force
}
Write-Host "staged libmpv-2.dll (mpv $Ver) into $OutDir"
