param(
    [string]$SdkRoot = "E:\Gao_Ji\Ni_Xiang\android-sdk",
    [string]$DebugKeystorePassword = $env:ALVR_DEBUG_KEYSTORE_PASSWORD,
    [string]$DebugKeystoreAlias = "androiddebugkey"
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrEmpty($DebugKeystorePassword)) {
    throw "Set ALVR_DEBUG_KEYSTORE_PASSWORD to the password of the existing VRX debug keystore."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$buildRoot = Join-Path $repoRoot "build\vrx_apk"
$classesDir = Join-Path $buildRoot "classes"
$dexDir = Join-Path $buildRoot "dex"
$baseApk = Join-Path $repoRoot "build\alvr_client_android\alvr_client_android.apk"
$unsignedApk = Join-Path $buildRoot "alvr_client_android_unsigned.apk"
$alignedApk = Join-Path $buildRoot "alvr_client_android_aligned.apk"
$finalApk = Join-Path $repoRoot "build\alvr_client_android\alvr_client_android_qxr.apk"
$keystore = Join-Path $repoRoot "build\alvr_client_android\debug.keystore"
$javaSource = Join-Path $repoRoot "alvr\client_openxr\java"
$androidJar = Join-Path $SdkRoot "platforms\android-32\android.jar"
$buildTools = Join-Path $SdkRoot "build-tools\34.0.0"
$cargoHome = Join-Path $SdkRoot "cargo"
$rustupHome = Join-Path $SdkRoot "rustup"
$javaHome = Join-Path $SdkRoot "jdk17tmp\jdk-17.0.20.1+1"
$ndkHome = Join-Path $SdkRoot "ndk\25.2.9519653"

$requiredPaths = @(
    $androidJar,
    $javaHome,
    $ndkHome,
    (Join-Path $buildTools "d8.bat"),
    (Join-Path $buildTools "zipalign.exe"),
    (Join-Path $buildTools "apksigner.bat"),
    (Join-Path $cargoHome "bin\cargo.exe")
)
foreach ($path in $requiredPaths) {
    if (-not (Test-Path -LiteralPath $path)) {
        throw "Required build tool is missing: $path"
    }
}

$env:RUSTUP_HOME = $rustupHome
$env:CARGO_HOME = $cargoHome
$env:JAVA_HOME = $javaHome
$env:ANDROID_HOME = $SdkRoot
$env:ANDROID_SDK_ROOT = $SdkRoot
$env:ANDROID_NDK_HOME = $ndkHome
$env:ANDROID_NDK_ROOT = $ndkHome
$env:Path = "$cargoHome\bin;$javaHome\bin;$buildTools;$SdkRoot\platform-tools;$SdkRoot\cmake\3.22.1\bin;C:\Program Files\Git\usr\bin;$env:Path"

New-Item -ItemType Directory -Force -Path $buildRoot, $classesDir, $dexDir | Out-Null

Push-Location $repoRoot
try {
    & (Join-Path $cargoHome "bin\cargo.exe") xtask build-client --release
    if ($LASTEXITCODE -ne 0) {
        throw "cargo xtask build-client failed with exit code $LASTEXITCODE"
    }
    if (-not (Test-Path -LiteralPath $baseApk)) {
        throw "The client build did not produce $baseApk"
    }
    if (-not (Test-Path -LiteralPath $keystore)) {
        throw "The existing VRX debug keystore is missing: $keystore"
    }

    $javaFiles = @(
        (Join-Path $javaSource "com\qualcomm\qti\qxrsocketfetcher\AlvrVrxActivity.java"),
        (Join-Path $javaSource "com\qualcomm\qti\qxrsocketfetcher\QXRSocketFetcher.java")
    )
    & (Join-Path $javaHome "bin\javac.exe") --release 8 `
        -classpath $androidJar -d $classesDir $javaFiles
    if ($LASTEXITCODE -ne 0) {
        throw "javac failed with exit code $LASTEXITCODE"
    }

    $classFiles = @(
        Get-ChildItem -LiteralPath $classesDir -Recurse -Filter "*.class" |
            ForEach-Object FullName
    )
    & (Join-Path $buildTools "d8.bat") --min-api 32 --output $dexDir $classFiles
    if ($LASTEXITCODE -ne 0) {
        throw "d8 failed with exit code $LASTEXITCODE"
    }

    Copy-Item -LiteralPath $baseApk -Destination $unsignedApk -Force
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::Open(
        $unsignedApk,
        [System.IO.Compression.ZipArchiveMode]::Update
    )
    try {
        $existingEntry = $zip.GetEntry("classes.dex")
        if ($null -ne $existingEntry) {
            $existingEntry.Delete()
        }
        $entry = $zip.CreateEntry(
            "classes.dex",
            [System.IO.Compression.CompressionLevel]::Optimal
        )
        $sourceStream = [System.IO.File]::OpenRead((Join-Path $dexDir "classes.dex"))
        $destinationStream = $entry.Open()
        try {
            $sourceStream.CopyTo($destinationStream)
        }
        finally {
            $destinationStream.Dispose()
            $sourceStream.Dispose()
        }
    }
    finally {
        $zip.Dispose()
    }

    & (Join-Path $buildTools "zipalign.exe") -p -f 4 $unsignedApk $alignedApk
    if ($LASTEXITCODE -ne 0) {
        throw "zipalign failed with exit code $LASTEXITCODE"
    }

    & (Join-Path $buildTools "apksigner.bat") sign `
        --ks $keystore `
        --ks-key-alias $DebugKeystoreAlias `
        --ks-pass "pass:$DebugKeystorePassword" `
        --key-pass "pass:$DebugKeystorePassword" `
        --out $finalApk $alignedApk
    if ($LASTEXITCODE -ne 0) {
        throw "apksigner failed with exit code $LASTEXITCODE"
    }

    & (Join-Path $buildTools "apksigner.bat") verify --verbose $finalApk
    if ($LASTEXITCODE -ne 0) {
        throw "The final APK failed signature verification"
    }

    $hash = Get-FileHash -Algorithm SHA256 -LiteralPath $finalApk
    Write-Host "VRX APK: $finalApk"
    Write-Host "SHA-256: $($hash.Hash)"
}
finally {
    Pop-Location
}
