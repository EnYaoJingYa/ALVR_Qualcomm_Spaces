param(
    [string]$Overlay = "/steam/dashboard",
    [switch]$Hide
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$openVrRoot = Join-Path $repoRoot "openvr"
$openVrBin = Join-Path $openVrRoot "bin\win64"
$openVrBindings = Join-Path $openVrRoot "headers\openvr_api.cs"

if (-not (Test-Path -LiteralPath (Join-Path $openVrBin "openvr_api.dll"))) {
    throw "Missing OpenVR runtime: $openVrBin\openvr_api.dll"
}
if (-not (Test-Path -LiteralPath $openVrBindings)) {
    throw "Missing OpenVR C# bindings: $openVrBindings"
}

$env:PATH = "$openVrBin;$env:PATH"
if ($null -eq ("Valve.VR.OpenVR" -as [type])) {
    Add-Type -Path $openVrBindings
}

$initError = [Valve.VR.EVRInitError]::None
$system = [Valve.VR.OpenVR]::Init(
    [ref]$initError,
    [Valve.VR.EVRApplicationType]::VRApplication_Overlay
)
if ($null -eq $system -or $initError -ne [Valve.VR.EVRInitError]::None) {
    throw "OpenVR initialization failed: $initError"
}

try {
    $visibleBefore = [Valve.VR.OpenVR]::Overlay.IsDashboardVisible()
    if ($Hide) {
        [Valve.VR.OpenVR]::Overlay.ShowDashboard("")
    }
    else {
        [Valve.VR.OpenVR]::Overlay.ShowDashboard($Overlay)
    }
    Start-Sleep -Milliseconds 750
    $visibleAfter = [Valve.VR.OpenVR]::Overlay.IsDashboardVisible()

    [pscustomobject]@{
        Action = if ($Hide) { "Hide" } else { "Show" }
        Overlay = if ($Hide) { $null } else { $Overlay }
        VisibleBefore = $visibleBefore
        VisibleAfter = $visibleAfter
    } | Format-List
}
finally {
    [Valve.VR.OpenVR]::Shutdown()
}
