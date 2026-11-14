#Requires -Version 5.1
<#
.SYNOPSIS
  Start the framerize daemon on Windows with the Camoufox (stealth Firefox) engine.

.DESCRIPTION
  Camoufox is the engine that passes Framer's Castle device checks from a
  desktop machine. Desktop Chrome/Edge (full-chrome) and headless-shell are
  risk-refused with HTTP 403 "request blocked by our security checks" on a
  fresh device, so this launcher always selects --engine camoufox.

  Prerequisites (one time):
    py -3.12 -m pip install -r runtime\requirements.txt   # camoufox==0.5.6, playwright==1.62.0
    python -m camoufox fetch                              # download the Camoufox browser

  The daemon stores its database, session cookies and per-account browser
  state under -DataDir (default: <repo>\.framerize\runtime, gitignored), so
  imported accounts survive restarts.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\windows\run-daemon.ps1

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\windows\run-daemon.ps1 -Listen 127.0.0.1:9000
#>
param(
  [string]$Listen = "127.0.0.1:8787",
  [string]$DataDir = "",
  [string]$Python = "",
  [string]$Exe = "",
  [string]$WebDir = ""
)

$ErrorActionPreference = "Stop"

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path

if (-not $Python) {
  $Python = (Get-Command python -ErrorAction SilentlyContinue).Source
  if (-not $Python) {
    throw "python not found on PATH; pass -Python C:\path\to\python.exe"
  }
}
if (-not $DataDir) { $DataDir = Join-Path $repo ".framerize\runtime" }
if (-not $Exe) { $Exe = Join-Path $repo "target\debug\framerize.exe" }
if (-not $WebDir) { $WebDir = Join-Path $repo "web\dist" }

if (-not (Test-Path $Exe)) {
  throw "daemon binary not found at $Exe; build it first: cargo build -p framerize"
}
if (-not (Test-Path $WebDir)) {
  throw "web UI not found at $WebDir; build it first: (cd web; npm ci; npm run build)"
}

$sidecar = Join-Path $repo "scripts\camoufox_host.py"
if (-not (Test-Path $sidecar)) {
  throw "cross-platform Camoufox sidecar not found at $sidecar"
}

$DataDir = [System.IO.Path]::GetFullPath($DataDir)
$profiles = Join-Path $DataDir "profiles"
$db = Join-Path $DataDir "framerize.db"
$null = New-Item -ItemType Directory -Force -Path $DataDir, $profiles

# The sidecar resolves its interpreter from FRAMERIZE_PYTHON; on Windows the
# venv path used on Unix does not exist, and `python3` is a Store stub.
$env:FRAMERIZE_PYTHON = $Python

Write-Host "framerize daemon"
Write-Host "  listen:  $Listen"
Write-Host "  data:    $DataDir"
Write-Host "  python:  $Python"
Write-Host "  engine:  camoufox ($sidecar)"

& $Exe daemon `
  --listen $Listen `
  --db $db `
  --profiles $profiles `
  --web-dir $WebDir `
  --engine camoufox `
  --sidecar-script $sidecar
