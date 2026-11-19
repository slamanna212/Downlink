# UNTESTED on Windows. Fetch the pinned zhongfly libmpv build into tools\mpv\out\windows\mpv.
# Fill in $Tag and $Sha256 from PINNED.md before use.
$ErrorActionPreference = 'Stop'
$Tag    = $env:MPV_TAG    # e.g. a zhongfly release tag, pinned in PINNED.md
$Sha256 = $env:MPV_SHA256
if (-not $Tag -or -not $Sha256) { throw 'Set MPV_TAG and MPV_SHA256 (see PINNED.md)' }
$Out = Join-Path $PSScriptRoot 'out\windows\mpv'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Url = "https://github.com/zhongfly/mpv-winbuild/releases/download/$Tag/mpv-dev-x86_64-$Tag.7z"  # TODO: confirm asset name
$Archive = Join-Path $env:TEMP 'mpv-dev.7z'
Invoke-WebRequest -Uri $Url -OutFile $Archive
if ((Get-FileHash $Archive -Algorithm SHA256).Hash -ne $Sha256.ToUpper()) { throw 'SHA-256 mismatch' }
7z x $Archive "-o$Out" -y | Out-Null
Get-ChildItem $Out -Filter 'libmpv-2.dll' | Out-Null
Write-Host "staged libmpv into $Out"
