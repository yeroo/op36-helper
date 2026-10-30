param([int]$ProcId, [string]$Channel, [string]$Payload, [string]$Exe)
# OP36 Helper update helper (after agwinterm's): runs detached, waits for the helper to exit, applies
# the downloaded release, relaunches the same exe path. Installed: silent setup re-run. Portable:
# rename the running exe aside (allowed while it runs), move the new one in, roll back on failure.
function Log([string]$m) { try { Add-Content -Path ($Payload + '.log') -Value ("{0:HH:mm:ss.fff} {1}" -f (Get-Date), $m) } catch { } }
Log "wait pid=$ProcId channel=$Channel"
try { Wait-Process -Id $ProcId -Timeout 120 -ErrorAction SilentlyContinue } catch { }
if (Get-Process -Id $ProcId -ErrorAction SilentlyContinue) { Log 'ABORT: helper never exited'; exit 1 }
Start-Sleep -Milliseconds 500
Log 'helper exited; applying'
if ($Channel -eq 'installed') {
  Start-Process $Payload -ArgumentList '/VERYSILENT','/NORESTART','/SUPPRESSMSGBOXES' -Wait
  Log 'setup finished'
} else {
  $old = $Exe + '.old'
  Remove-Item $old -Force -ErrorAction SilentlyContinue
  $moved = $false
  for ($i = 0; $i -lt 20 -and -not $moved; $i++) {
    try { Move-Item $Exe $old -Force -ErrorAction Stop; $moved = $true } catch { Log ("rename-aside failed: " + $_.Exception.Message); Start-Sleep -Milliseconds 500 }
  }
  if (-not $moved) { Log 'GIVING UP: exe still locked'; exit 1 }
  try { Move-Item $Payload $Exe -Force -ErrorAction Stop; Log 'swap done' }
  catch { Log ("swap-in failed, rolling back: " + $_.Exception.Message); Move-Item $old $Exe -Force; exit 1 }
}
Start-Process $Exe
Log 'relaunched'
