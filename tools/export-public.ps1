# Copy the working tree into a fresh directory for the public repository: no git history, none of
# the vendor or personal binaries, no scratch. The firmware submodule pointer travels as .gitmodules
# plus an empty rmk/ directory; `git submodule update --init` fills it after the first clone.
param([Parameter(Mandatory = $true)][string]$Dest)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Definition)
$exclude = @(
  ".git", "_scratch", "_ref", ".superpowers", ".venv", "rmk",
  "helper\target", "installer\Output", "installer\stage",
  "firmware\op36-v0.2.0", "firmware\op36-v0.2.0-stock", "firmware\zmk-rollback",
  "firmware\backup-zmk-left", "firmware\backup-zmk-right", "firmware\settings-reset"
)
$excludeFiles = @("1.png", "2.png", "3.png", "4.png", "5.png", "firmware\op36-v0.2.0.zip", "firmware\settings-reset-v0.1.4.zip")

if (Test-Path $Dest) { throw "destination exists: $Dest" }
New-Item -ItemType Directory -Force $Dest | Out-Null
$count = 0
Get-ChildItem -Path $root -Recurse -File -Force | ForEach-Object {
  $rel = $_.FullName.Substring($root.Length).TrimStart('\')
  $top = $rel.Split('\')[0]
  foreach ($e in $exclude) { if ($rel -eq $e -or $rel.StartsWith($e + '\')) { return } }
  if ($excludeFiles -contains $rel) { return }
  if ($_.Name -like "*.pyc") { return }
  $target = Join-Path $Dest $rel
  New-Item -ItemType Directory -Force (Split-Path -Parent $target) | Out-Null
  Copy-Item $_.FullName $target
  $script:count++
}
New-Item -ItemType Directory -Force (Join-Path $Dest "rmk") | Out-Null
"exported $count files to $Dest"
