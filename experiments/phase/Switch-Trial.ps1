param([ValidateSet('baseline','observe','frequency','active','diagnostic')][string]$Mode,
      [switch]$FaultBuild)
$ErrorActionPreference='Stop'
$root=(Resolve-Path (Join-Path $PSScriptRoot '../../../..')).Path
$base=Join-Path $root 'analysis_official_streaming/lenovo_phase_impl_20261101'
$streamer=Join-Path $root 'ALVR_Qualcomm_Spaces/build/alvr_streamer_windows'
$dest=Join-Path $streamer 'bin/win64/driver_alvr_server.dll'
$driver=Join-Path $PSScriptRoot 'V2_driver.dll'
$apk=Join-Path $PSScriptRoot '../../build/alvr_client_android/alvr_client_android.apk'
if($Mode -eq 'baseline') { $driver=Join-Path $base 'A_driver.dll'; $apk=Join-Path $base 'A.apk' }
if($FaultBuild -and $Mode -ne 'baseline') { $apk=Join-Path $PSScriptRoot 'Fault.apk' }
if(!(Test-Path -LiteralPath $apk) -or !(Test-Path -LiteralPath $driver)){throw 'Missing trial artifacts'}
$adb='E:/platform-tools/adb.exe'
$serial='HLA50582'
& $adb -s $serial get-state
if($LASTEXITCODE){throw 'Headset offline'}
& $adb -s $serial shell input keyevent KEYCODE_HOME
$api='http://127.0.0.1:8082/api'
$headers=@{'X-ALVR'='1'}
if(Get-Process vrserver -ErrorAction SilentlyContinue){
    Invoke-RestMethod -Method Post -Uri "$api/steamvr/restart" -Headers $headers | Out-Null
    Wait-Process -Name vrserver -Timeout 25 -ErrorAction SilentlyContinue
    if(Get-Process vrserver -ErrorAction SilentlyContinue){throw 'SteamVR still running'}
}
Copy-Item -LiteralPath $driver -Destination $dest
& $adb -s $serial install -r $apk
if($LASTEXITCODE){throw 'Installation failed'}
& $adb -s $serial shell am start -n alvr.client.dev/android.app.NativeActivity
$env:ALVR_VRX_LENOVO_PHASE=$Mode
Start-Process -FilePath 'C:/Program Files (x86)/Steam/steamapps/common/SteamVR/bin/win64/vrstartup.exe' -WindowStyle Hidden
for($i=0;$i -lt 45;$i++){
    Start-Sleep -Seconds 1
    $s=Get-Content (Join-Path $streamer 'session.json') -Raw|ConvertFrom-Json
    if($s.client_connections.'client.wired'.connection_state -eq 'Streaming'){
        if($Mode -eq 'baseline' -or (Select-String -Quiet -Path (Join-Path $streamer 'session_log.txt') -Pattern 'event=vrx_lenovo_received')){
            Write-Output "$Mode wired; apk=$((Get-FileHash $apk).Hash); driver=$((Get-FileHash $dest).Hash)"
            return
        }
    }
    foreach($client in $s.client_connections.PSObject.Properties){
        if($client.Name -ne 'client.wired' -and $client.Value.connection_state -eq 'Streaming'){
            $body=ConvertTo-Json -Compress -InputObject @($client.Name,@{SetConnectionState='Disconnecting'})
            Invoke-RestMethod -Method Post -Uri "$api/session/client-connections" -Headers $headers -ContentType application/json -Body $body | Out-Null
        }
    }
}
throw "$Mode did not establish verified wired session"
