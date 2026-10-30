param([int]$ProcId, [string]$Channel, [string]$Payload, [string]$Exe)
# OP36 Helper update helper (after agwinterm's): runs detached, waits for the helper to exit, applies
# the downloaded release, relaunches the same exe path. Installed: silent setup re-run. Portable:
# rename the running exe aside (allowed while it runs), move the new one in, roll back on failure.
# Whatever goes wrong, the user gets their helper back: every failure path relaunches the old exe.
# Paths are passed with -LiteralPath so folders with [ ] are not read as wildcards.
function Log([string]$m) { try { Add-Content -LiteralPath ($Payload + '.log') -Value ("{0:HH:mm:ss.fff} {1}" -f (Get-Date), $m) } catch { } }
function Fail([string]$why) {
  Log ("FAILED: " + $why + "; relaunching the current helper")
  try { Start-Process -FilePath $Exe } catch { Log ("relaunch failed too: " + $_.Exception.Message) }
  exit 1
}
Log "wait pid=$ProcId channel=$Channel"
try { Wait-Process -Id $ProcId -Timeout 120 -ErrorAction SilentlyContinue } catch { }
if (Get-Process -Id $ProcId -ErrorAction SilentlyContinue) { Fail 'helper never exited' }
Start-Sleep -Milliseconds 500
Log 'helper exited; applying'
if ($Channel -eq 'installed') {
  $setup = Start-Process -FilePath $Payload -ArgumentList '/VERYSILENT','/NORESTART','/SUPPRESSMSGBOXES' -Wait -PassThru
  Log ("setup finished, exit code " + $setup.ExitCode)
  if ($setup.ExitCode -ne 0) { Fail ("setup exit code " + $setup.ExitCode) }
} else {
  $old = $Exe + '.old'
  Remove-Item -LiteralPath $old -Force -ErrorAction SilentlyContinue
  $moved = $false
  for ($i = 0; $i -lt 20 -and -not $moved; $i++) {
    try { Move-Item -LiteralPath $Exe -Destination $old -Force -ErrorAction Stop; $moved = $true }
    catch { Log ("rename-aside failed: " + $_.Exception.Message); Start-Sleep -Milliseconds 500 }
  }
  if (-not $moved) { Fail 'exe still locked after 10 s' }
  try { Move-Item -LiteralPath $Payload -Destination $Exe -Force -ErrorAction Stop; Log 'swap done' }
  catch {
    Log ("swap-in failed, rolling back: " + $_.Exception.Message)
    Move-Item -LiteralPath $old -Destination $Exe -Force
    Fail 'could not move the new exe into place'
  }
}
Start-Process -FilePath $Exe
Log 'relaunched'
