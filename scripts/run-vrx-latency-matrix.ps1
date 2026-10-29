param(
    [ValidateSet("Wired", "Wireless", "All")]
    [string]$LinkMode = "All",
    [int[]]$BitratesMbps = @(200, 400, 600, 800),
    [int]$SampleSeconds = 30,
    [string]$Serial = "HLA50582",
    [string]$ApiBase = "http://127.0.0.1:8082",
    [ValidateSet("Off", "On")]
    [string]$FoveatedEncoding = "Off"
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$sessionPath = Join-Path $repoRoot "build\alvr_streamer_windows\session.json"
$tracePath = Join-Path $env:TEMP "alvr-latency-breakdown.log"
$resultRoot = Join-Path $repoRoot "build\vrx_validation\latency-matrix"
$clientComponent = "alvr.client.dev/com.qualcomm.qti.qxrsocketfetcher.AlvrVrxActivity"
$wiredHostname = "client.wired"
$advertisedHostname = "8368.client.local."

$wirelessMaxLatencyMs = @{
    200 = 45.0
    400 = 48.0
    600 = 90.0
    800 = 400.0
}
$wiredMaxLatencyMs = @{
    200 = 38.0
    400 = 38.0
    600 = 38.0
    800 = 48.0
}

function Invoke-AlvrPost {
    param(
        [string]$Path,
        [object]$Body
    )

    Invoke-RestMethod `
        -Uri "$ApiBase$Path" `
        -Method Post `
        -Headers @{ "X-ALVR" = "true" } `
        -ContentType "application/json" `
        -Body (ConvertTo-Json -InputObject $Body -Depth 12 -Compress)
}

function Get-VrxSession {
    Get-Content -LiteralPath $sessionPath -Raw | ConvertFrom-Json
}

function Get-ClientEntry {
    param([string]$Hostname)

    $session = Get-VrxSession
    $property = $session.client_connections.PSObject.Properties[$Hostname]
    if ($null -eq $property) {
        return $null
    }
    $property.Value
}

function Send-ClientAction {
    param(
        [string]$Hostname,
        [object]$Action
    )

    Invoke-AlvrPost `
        -Path "/api/session/client-connections" `
        -Body @($Hostname, $Action)
}

function Set-Bitrate {
    param([int]$Mbps)

    $mode = (Get-VrxSession).session_settings.video.bitrate.mode
    $mode.ConstantMbps = $Mbps
    $mode.variant = "ConstantMbps"

    Invoke-AlvrPost `
        -Path "/api/session/values" `
        -Body @(
            @{
                path = @(
                    @{ Name = "session_settings" },
                    @{ Name = "video" },
                    @{ Name = "bitrate" },
                    @{ Name = "mode" }
                )
                value = $mode
            }
        )
}

function Ensure-AppStarted {
    $deviceLine = & adb devices | Select-String -Pattern "^$([regex]::Escape($Serial))\s+device$"
    if (-not $deviceLine) {
        throw "ADB device $Serial is not online."
    }
    $appPid = (& adb -s $Serial shell pidof alvr.client.dev).Trim()
    if ([string]::IsNullOrWhiteSpace($appPid)) {
        & adb -s $Serial shell am start -n $clientComponent | Out-Null
    }
}

function Wait-Streaming {
    param(
        [string]$ExpectedHostname,
        [int]$TimeoutSeconds = 45
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $session = Get-VrxSession
        $entry = $session.client_connections.PSObject.Properties[$ExpectedHostname]
        if ($null -ne $entry -and $entry.Value.connection_state -eq "Streaming") {
            return $entry.Value
        }
        Start-Sleep -Milliseconds 500
    }

    throw "Timed out waiting for $ExpectedHostname to reach Streaming."
}

function Wait-Disconnected {
    param(
        [string]$Hostname,
        [int]$TimeoutSeconds = 20
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $entry = Get-ClientEntry -Hostname $Hostname
        if ($null -eq $entry -or $entry.connection_state -eq "Disconnected") {
            return
        }
        Start-Sleep -Milliseconds 500
    }

    throw "Timed out waiting for $Hostname to disconnect."
}

function Switch-LinkMode {
    param([ValidateSet("Wired", "Wireless")][string]$Mode)

    $clients = (Get-VrxSession).client_connections.PSObject.Properties

    if ($Mode -eq "Wired") {
        Ensure-AppStarted
        Send-ClientAction `
            -Hostname $wiredHostname `
            -Action @{ AddIfMissing = @{ trusted = $true; manual_ips = @() } }

        foreach ($client in $clients) {
            if ($client.Name -ne $wiredHostname -and $client.Value.connection_state -ne "Disconnected") {
                Send-ClientAction -Hostname $client.Name -Action "RemoveEntry"
            }
        }
        Wait-Disconnected -Hostname $advertisedHostname | Out-Null
        Wait-Streaming -ExpectedHostname $wiredHostname | Out-Null
    }
    else {
        $advertised = Get-ClientEntry -Hostname $advertisedHostname
        if ($null -eq $advertised -or $advertised.connection_state -ne "Streaming") {
            Ensure-AppStarted
        }

        $wired = Get-ClientEntry -Hostname $wiredHostname
        if ($null -ne $wired) {
            Send-ClientAction -Hostname $wiredHostname -Action "RemoveEntry"
            Wait-Disconnected -Hostname $wiredHostname | Out-Null
        }

        Wait-Streaming -ExpectedHostname $advertisedHostname | Out-Null
    }
}

function Get-AppendedTrace {
    param(
        [long]$StartOffset,
        [string]$Destination
    )

    $stream = [System.IO.File]::Open(
        $tracePath,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::ReadWrite
    )
    try {
        $stream.Seek($StartOffset, [System.IO.SeekOrigin]::Begin) | Out-Null
        $reader = [System.IO.StreamReader]::new($stream)
        try {
            $text = $reader.ReadToEnd()
        }
        finally {
            $reader.Dispose()
        }
    }
    finally {
        if (-not $stream.SafeFileHandle.IsClosed) {
            $stream.Dispose()
        }
    }

    $text | Out-File -LiteralPath $Destination -Encoding utf8
    $text
}

function Get-Median {
    param([double[]]$Values)

    if ($Values.Count -eq 0) {
        return [double]::NaN
    }
    $sorted = @($Values | Sort-Object)
    if ($sorted.Count % 2 -eq 1) {
        return $sorted[[int](($sorted.Count - 1) / 2)]
    }
    return ($sorted[$sorted.Count / 2 - 1] + $sorted[$sorted.Count / 2]) / 2
}

function Measure-Bitrate {
    param(
        [ValidateSet("Wired", "Wireless")][string]$Mode,
        [int]$Mbps
    )

    Set-Bitrate -Mbps $Mbps
    Start-Sleep -Seconds 5

    $expectedHostname = if ($Mode -eq "Wired") { $wiredHostname } else { $advertisedHostname }
    Wait-Streaming -ExpectedHostname $expectedHostname | Out-Null
    Start-Sleep -Seconds 3

    $startOffset = (Get-Item -LiteralPath $tracePath).Length
    Start-Sleep -Seconds $SampleSeconds
    $fileSuffix = "{0}-{1}-{2}" -f $Mode.ToLowerInvariant(), $Mbps, (Get-Date -Format "yyyyMMdd-HHmmss")
    $rawPath = Join-Path $resultRoot "$fileSuffix.log"
    $trace = Get-AppendedTrace -StartOffset $startOffset -Destination $rawPath
    $lines = @($trace -split "`r?`n" | Where-Object { $_ -match "^total=" })

    $total = @($lines | ForEach-Object {
        [double]([regex]::Match($_, "total=([0-9.]+)").Groups[1].Value)
    })
    $vsync = @($lines | ForEach-Object {
        [double]([regex]::Match($_, "vsync_queue=([0-9.]+)").Groups[1].Value)
    })
    $transport = @($lines | ForEach-Object {
        [double]([regex]::Match($_, "transport=([0-9.]+)").Groups[1].Value)
    })
    $decoderQueue = @($lines | ForEach-Object {
        [double]([regex]::Match($_, "decoder_queue=([0-9.]+)").Groups[1].Value)
    })

    if ($total.Count -eq 0) {
        throw "No latency samples were collected for $Mode at $Mbps Mbps."
    }

    $median = Get-Median -Values $total
    $p90Values = @($total | Sort-Object)
    $p90 = $p90Values[[int][Math]::Floor(($p90Values.Count - 1) * 0.9)]
    $targetMax = if ($Mode -eq "Wired") {
        $wiredMaxLatencyMs[$Mbps]
    }
    else {
        $wirelessMaxLatencyMs[$Mbps]
    }

    [pscustomobject]@{
        link = $Mode
        bitrate_mbps = $Mbps
        samples = $total.Count
        total_median_ms = [Math]::Round($median, 2)
        total_p90_ms = [Math]::Round($p90, 2)
        total_min_ms = [Math]::Round((@($total | Sort-Object)[0]), 2)
        total_max_ms = [Math]::Round((@($total | Sort-Object)[-1]), 2)
        vsync_median_ms = [Math]::Round((Get-Median -Values $vsync), 2)
        transport_median_ms = [Math]::Round((Get-Median -Values $transport), 2)
        decoder_queue_median_ms = [Math]::Round((Get-Median -Values $decoderQueue), 2)
        target_max_ms = $targetMax
        passes_target = $median -le $targetMax
        trace_file = $rawPath
    }
}

New-Item -ItemType Directory -Force -Path $resultRoot | Out-Null
Invoke-WebRequest -Uri "$ApiBase/api/ping" -Headers @{ "X-ALVR" = "true" } | Out-Null

$session = Get-VrxSession
$video = $session.session_settings.video
$expectedFoveated = $FoveatedEncoding -eq "On"
$qualityChecks = @(
    ($video.preferred_codec.variant -ne "H264"),
    ($video.transcoding_view_resolution.variant -ne "Absolute"),
    ($video.transcoding_view_resolution.Absolute.width -ne 2144),
    ($video.preferred_fps -ne 90),
    ($video.foveated_encoding.enabled -ne $expectedFoveated),
    ($video.max_buffering_frames -ne 0.1)
)
if ($qualityChecks -contains $true) {
    throw "The session no longer matches the validated image-quality configuration."
}

$modes = if ($LinkMode -eq "All") { @("Wireless", "Wired") } else { @($LinkMode) }
$results = @()
foreach ($mode in $modes) {
    Switch-LinkMode -Mode $mode
    foreach ($mbps in $BitratesMbps) {
        $result = Measure-Bitrate -Mode $mode -Mbps $mbps
        $results += $result
        $result | Format-List
    }
}

$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$jsonPath = Join-Path $resultRoot "latency-matrix-$timestamp.json"
$csvPath = Join-Path $resultRoot "latency-matrix-$timestamp.csv"
$results | ConvertTo-Json -Depth 5 | Out-File -LiteralPath $jsonPath -Encoding utf8
$results | Export-Csv -LiteralPath $csvPath -NoTypeInformation -Encoding utf8BOM

Write-Host "JSON: $jsonPath"
Write-Host "CSV:  $csvPath"
if (@($results | Where-Object { -not $_.passes_target }).Count -gt 0) {
    throw "One or more bitrate/link combinations missed the requested latency target."
}
