$ErrorActionPreference='Stop'
$adb='E:/platform-tools/adb.exe'
$device='HLA50582'
$root=(Resolve-Path (Join-Path $PSScriptRoot '../../../..')).Path
$log=Join-Path $root 'ALVR_Qualcomm_Spaces/build/alvr_streamer_windows/session_log.txt'
$state=Join-Path $root 'ALVR_Qualcomm_Spaces/build/alvr_streamer_windows/session.json'
$initial=(& $adb -s $device shell getprop debug.alvr.fail_stage | Out-String).Trim()
if($initial){throw 'Existing fault selection must be preserved'}
function Reconnect {
    Invoke-RestMethod -Method Post -Uri http://127.0.0.1:8082/api/session/client-connections -Headers @{'X-ALVR'='1'} -ContentType application/json -Body '["client.wired",{"SetConnectionState":"Disconnecting"}]' | Out-Null
    Start-Sleep -Seconds 6
}
function Read-Window {
    $start=(Get-Item $log).Length
    Start-Sleep -Seconds 2
    $file=[IO.File]::Open($log,'Open','Read','ReadWrite')
    try { $null=$file.Seek($start,'Begin'); $reader=[IO.StreamReader]::new($file); $reader.ReadToEnd() } finally { $file.Dispose() }
}
$rows=@()
try {
    & "$PSScriptRoot/Switch-Trial.ps1" diagnostic -FaultBuild
    foreach($fault in 'vrx_phase_frozen_counter','vrx_phase_feedback_loss','vrx_phase_service') {
        & $adb -s $device shell setprop debug.alvr.fail_stage $fault
        if($fault -eq 'vrx_phase_service'){Reconnect}
        Start-Sleep -Seconds 2
        $failed=Read-Window
        & $adb -s $device shell "setprop debug.alvr.fail_stage ''"
        if($fault -eq 'vrx_phase_service'){Reconnect}
        Start-Sleep -Seconds 2
        $recovered=Read-Window
        $wired=(Get-Content $state -Raw|ConvertFrom-Json).client_connections.'client.wired'.connection_state
        $row=[pscustomobject]@{fault=$fault; fallback=($failed -notmatch 'event=phase_wait_target' -and $failed -match '\[GRAPH\]'); recovered=($recovered -match 'event=phase_wait_target' -and $recovered -match '\[GRAPH\]'); wired=$wired}
        $rows+=$row
        $rows|ConvertTo-Json|Set-Content "$PSScriptRoot/results/faults.json"
        if(!$row.fallback -or !$row.recovered -or $wired -ne 'Streaming'){throw "Fault gate failed for $fault"}
    }
} finally {
    & $adb -s $device shell "setprop debug.alvr.fail_stage ''"
    & "$PSScriptRoot/Switch-Trial.ps1" baseline
}
