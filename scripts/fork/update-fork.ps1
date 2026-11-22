<#
.SYNOPSIS
  Install the newest fork build of jcode-desktop.exe from this fork's GitHub releases.

.DESCRIPTION
  Downloads jcode-desktop.exe from the latest release of the fork, checks its
  SHA-256, and replaces the exe in your Jcode Desktop folder. The original
  official exe is kept once as jcode-desktop.official.exe, and the previous
  build as jcode-desktop.prev.exe, so either can be restored by renaming.
  Requires the GitHub CLI (gh) to be signed in.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File update-fork.ps1 -InstallDir F:\jcode\Jcode-0.6.5-windows-x86_64
#>
param(
  [string]$InstallDir,
  [string]$Repo = 'mdea4420-svg/jcode-desktop',
  [string]$Tag = '',
  [switch]$Force
)
$ErrorActionPreference = 'Stop'

if (-not $InstallDir) {
  $running = Get-Process jcode-desktop -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($running -and $running.Path) {
    $InstallDir = Split-Path $running.Path
  } else {
    $hint = Join-Path $env:USERPROFILE '.jcode\hotkey\last_dir'
    if (Test-Path $hint) { $InstallDir = (Get-Content $hint -Raw).Trim() }
  }
}
if (-not $InstallDir -or -not (Test-Path (Join-Path $InstallDir 'jcode-desktop.exe'))) {
  throw "Could not find jcode-desktop.exe. Pass -InstallDir <folder containing jcode-desktop.exe>."
}
$exe = Join-Path $InstallDir 'jcode-desktop.exe'
$marker = Join-Path $InstallDir '.fork-release'

$release = if ($Tag) { $Tag } else {
  gh release view --repo $Repo --json tagName --jq .tagName
}
if ($LASTEXITCODE -or -not $release) { throw "Could not read the latest release of $Repo (is gh signed in?)" }
if (-not $Force -and (Test-Path $marker) -and ((Get-Content $marker -Raw).Trim() -eq $release)) {
  Write-Host "Already on $release."
  exit 0
}

$download = Join-Path ([IO.Path]::GetTempPath()) "jcode-fork-$release"
Remove-Item $download -Recurse -Force -ErrorAction SilentlyContinue
gh release download $release --repo $Repo --dir $download --pattern jcode-desktop.exe --pattern SHA256SUMS
if ($LASTEXITCODE) { throw "Download of $release failed" }
$expected = ((Get-Content (Join-Path $download 'SHA256SUMS') -Raw).Trim() -split '\s+')[0].ToLower()
$actual = (Get-FileHash (Join-Path $download 'jcode-desktop.exe') -Algorithm SHA256).Hash.ToLower()
if ($expected -ne $actual) { throw "Checksum mismatch for $release" }

$running = Get-Process jcode-desktop -ErrorAction SilentlyContinue |
  Where-Object { $_.Path -and ((Resolve-Path $_.Path).Path -eq (Resolve-Path $exe).Path) }
if ($running) {
  $answer = Read-Host "Jcode Desktop is running. Close it now to update? [y/N]"
  if ($answer -notmatch '^[yY]') { Write-Host 'Update postponed.'; exit 1 }
  $running | ForEach-Object { $_.CloseMainWindow() | Out-Null }
  Start-Sleep -Seconds 3
  $running | Where-Object { -not $_.HasExited } | Stop-Process -Force
  Start-Sleep -Seconds 1
}

$official = Join-Path $InstallDir 'jcode-desktop.official.exe'
if (-not (Test-Path $official) -and -not (Test-Path $marker)) { Copy-Item $exe $official }
Copy-Item $exe (Join-Path $InstallDir 'jcode-desktop.prev.exe') -Force
Copy-Item (Join-Path $download 'jcode-desktop.exe') $exe -Force
Set-Content $marker $release
Remove-Item $download -Recurse -Force -ErrorAction SilentlyContinue
Write-Host "Installed $release into $InstallDir"
if ($running) { Start-Process $exe }
