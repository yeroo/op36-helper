# Copy the release exe to installer\Output\op36-helper-portable-<ver>.exe. The portable exe looks
# for its firmware in a `firmware\` folder next to it; without one the Flash buttons stay disabled.
param([switch]$SkipCargo)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Definition
$root = Split-Path -Parent $here
$helper = Join-Path $root "helper"
$cargo = Get-Content (Join-Path $helper "Cargo.toml") -Raw
if ($cargo -notmatch '(?m)^version\s*=\s*"([^"]+)"') { throw "version not found in helper\Cargo.toml" }
$ver = $Matches[1]
if (-not $SkipCargo) {
  Push-Location $helper
  try { & cargo build --release; if ($LASTEXITCODE -ne 0) { throw "cargo build failed" } } finally { Pop-Location }
}
New-Item -ItemType Directory -Force (Join-Path $here "Output") | Out-Null
$out = Join-Path $here "Output\op36-helper-portable-$ver.exe"
Copy-Item (Join-Path $helper "target\release\op36-helper.exe") $out -Force
Write-Host ("== done: {0} ({1:N1} MB) ==" -f $out, ((Get-Item $out).Length / 1MB)) -ForegroundColor Green
