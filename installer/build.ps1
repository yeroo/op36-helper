# Build installer\Output\op36-helper-setup-<ver>.exe: release build of the helper, staged with its
# icons, the firmware pair and the licence, compiled by Inno Setup 6 (ISCC on PATH, or the usual
# per-user / machine-wide locations). The version is read from helper\Cargo.toml.
param(
  [string]$FirmwareDir = "",   # folder with op36_left.uf2, op36_right.uf2, BUILD_COMMIT.txt
  [switch]$SkipCargo           # reuse an existing helper\target\release\op36-helper.exe
)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Definition
$root = Split-Path -Parent $here
$helper = Join-Path $root "helper"
$stage = Join-Path $here "stage"
if (-not $FirmwareDir) { $FirmwareDir = Join-Path $root "firmware\op36-layer-notify" }

$cargo = Get-Content (Join-Path $helper "Cargo.toml") -Raw
if ($cargo -notmatch '(?m)^version\s*=\s*"([^"]+)"') { throw "version not found in helper\Cargo.toml" }
$ver = $Matches[1]

$iscc = (Get-Command iscc.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
  $iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
            "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { throw "ISCC (Inno Setup 6 compiler) not found" }

if (-not $SkipCargo) {
  Write-Host "== cargo build --release ==" -ForegroundColor Cyan
  Push-Location $helper
  try { & cargo build --release; if ($LASTEXITCODE -ne 0) { throw "cargo build failed" } } finally { Pop-Location }
}

Write-Host "== stage ==" -ForegroundColor Cyan
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force (Join-Path $stage "assets") | Out-Null
New-Item -ItemType Directory -Force (Join-Path $stage "firmware") | Out-Null
Copy-Item (Join-Path $helper "target\release\op36-helper.exe") $stage
Copy-Item (Join-Path $helper "assets\icon_on.ico"), (Join-Path $helper "assets\icon_off.ico") (Join-Path $stage "assets")
foreach ($f in @("op36_left.uf2", "op36_right.uf2", "BUILD_COMMIT.txt")) {
  $src = Join-Path $FirmwareDir $f
  if (-not (Test-Path $src)) { throw "firmware file missing: $src" }
  Copy-Item $src (Join-Path $stage "firmware")
}
foreach ($f in @("LICENSE", "README.md")) {
  $src = Join-Path $root $f
  if (Test-Path $src) { Copy-Item $src $stage } else { Write-Warning "$f not found at the repository root; the installer ships without it" }
}
if (-not (Test-Path (Join-Path $stage "LICENSE"))) { Set-Content (Join-Path $stage "LICENSE") "MIT License - see https://github.com/yeroo/op36-helper" }
$exeVer = (Get-Item (Join-Path $stage "op36-helper.exe")).VersionInfo.FileVersion
if ($exeVer -and ($exeVer -notlike "$ver*")) { throw "exe version $exeVer does not match Cargo version $ver" }
Write-Host ("stage OK: {0} files, version {1}" -f (Get-ChildItem $stage -Recurse -File).Count, $ver) -ForegroundColor Green

Write-Host "== ISCC ==" -ForegroundColor Cyan
& $iscc "/DAppVersion=$ver" (Join-Path $here "op36-helper.iss")
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
$out = Join-Path $here "Output\op36-helper-setup-$ver.exe"
if (-not (Test-Path $out)) { throw "installer not produced: $out" }
Write-Host ("== done: {0} ({1:N1} MB) ==" -f $out, ((Get-Item $out).Length / 1MB)) -ForegroundColor Green
