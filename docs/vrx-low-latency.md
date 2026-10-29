# ThinkReality VRX low-latency validation

Date: 2026-10-29

Device: `HLA50582`, Lenovo ThinkReality VRX, firmware
`VRX_user_S1154001_2406262352_kona`.

## Root cause

With the original AIO settings, the VRX OpenXR runtime reported a VSync queue
of about `35-37 ms` per frame. The other ALVR pipeline stages normally totaled
only `8-13 ms`.

The runtime has a debug property:

```text
debug.openxr.cond.wait.enable
```

Setting it to `1` reduces the measured VSync queue to about `25-26.5 ms`.
`debug.openxr.wait.fence.enable=1` preserves the normal GPU fence wait and did
not negate the improvement.

The decoder's configured buffering target also directly adds whole frames of
latency. On a 90 Hz display:

- `max_buffering_frames=1.5`: roughly one to one-and-a-half extra frames.
- `max_buffering_frames=1.0`: alternating approximately `37.7/48.8 ms`.
- `max_buffering_frames=0.1`: approximately `37-38.5 ms`.

The current session therefore uses:

```text
debug.openxr.cond.wait.enable = 1
debug.openxr.wait.fence.enable = 1
max_buffering_frames = 0.1
```

## Persistence

The Android client sets the two runtime properties in its native `run` path,
before loading the OpenXR loader or creating an OpenXR session. This ordering
matters because `NativeActivity` starts `android_main` while the Java
Activity's `onCreate` is still running. This was verified after setting both
properties to `0` and launching the normal APK: both changed back to `1`.

The PC wired auto-launch path also sets the same properties whenever an online
VRX is selected, so a wired stream recovers the low-latency configuration
without a manual ADB command.

## Build

The VRX APK needs both the native client and the Java Activity/`classes.dex`.
Use the checked-in wrapper so the release build, Java compilation, dexing,
alignment, signing, and signature verification happen in one reproducible step:

```powershell
$env:ALVR_DEBUG_KEYSTORE_PASSWORD = '<existing local debug keystore password>'
powershell -NoProfile -ExecutionPolicy Bypass `
  -File scripts/build-vrx-apk.ps1
```

This writes `build/alvr_client_android/alvr_client_android_qxr.apk`.

The full bitrate matrix can be driven through the local ALVR HTTP API:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass `
  -File scripts/run-vrx-latency-matrix.ps1
```

The script checks the validated H.264 High, 2144-pixel, 90 Hz,
`max_buffering_frames=0.1`, and foveated-encoding mode. It runs 200, 400, 600,
and 800 Mbps for both wired and wireless links, saves each raw latency window,
and writes JSON/CSV summaries under `build/vrx_validation/latency-matrix`.

SteamVR's automatic per-eye render target is `2624x2624`; this is a supersampled
game render target and is separate from ALVR's transport resolution. The VRX
runtime recommends a `1520x1520` stream view, while ALVR's stable configured
stream resolution is `2144x2144`. Reducing the stream to 1520 caused visible
softness and was reverted. `2624x2624` transport was tested but caused the
current SteamVR/ALVR driver process to exit, so 2144 remains the stable setting.
Server-side foveated encoding is disabled to avoid reduced peripheral text
resolution.

## Cold-start verification

On 2026-10-29, the fixed APK was installed on `HLA50582` and tested with a
forced cold start:

1. Set both runtime properties to `0`.
2. Force-stop `alvr.client.dev`.
3. Launch `com.qualcomm.qti.qxrsocketfetcher.AlvrVrxActivity`.

The launch completed cold in `97 ms`; both properties were `1` immediately
after launch. Native logs confirm the ordering:

```text
10:48:10.730 VRX runtime low-latency properties set before OpenXR initialization
10:48:10.738 OpenXR available extensions:
10:48:10.790 Detected platform: Lenovo ThinkReality VRX
```

The client reconnected as `Streaming` over `192.168.1.10`, the panel remained
at its single 90 Hz mode, and the headset screenshot showed a normal stereo
SteamVR image. A 30-second steady-state sample at 800 Mbps produced 425 total
latency samples with a `38.20 ms` median, `36.06-39.48 ms` range, and
`25.52 ms` median VSync queue.

The first launch produced one transient compositor depth warning while the
SteamVR scene initialized. The repeated controlled launch and subsequent
30-second window had no `Duplicate display time`, `Too many frames are queued`,
`xrEndFrame`, fatal exception, or process crash messages.

Evidence:

```text
build/vrx_validation/cold-start-fixed-device-20261029.log
build/vrx_validation/cold-start-fixed-latency-20261029.log
build/vrx_validation/cold-start-fixed-final-20261029.png
build/vrx_validation/cold-start-dashboard-20261029.png
```

## Measured latency

The values below are `total` motion-to-photon latency samples from the ALVR
latency breakdown log. Wireless samples were verified against the real client
IP `192.168.1.10`, not the USB loopback address.

| Link | Mbps | Median | Typical range |
| --- | ---: | ---: | ---: |
| Wired | 200 | 37.83 ms | 36.57-38.52 ms |
| Wired | 400 | 37.84 ms | 36.71-38.16 ms |
| Wired | 600 | 37.64 ms | 36.72-38.46 ms |
| Wired | 800 | 37.69 ms | 36.01-38.19 ms |
| Wireless | 200 | 37.82 ms | 36.96-38.71 ms |
| Wireless | 400 | 37.78 ms | 36.85-38.53 ms |
| Wireless | 600 | 37.81 ms | 36.69-38.59 ms |
| Wireless | 800 | 37.69 ms | 36.73-38.52 ms |

### Current 2026-10-29 matrix

The automated 30-second matrix was rerun after the cold-start and boundary
fixes. Wireless results were:

| Mbps | Median | P90 | VSync median | Transport median | Result |
| ---: | ---: | ---: | ---: | ---: | --- |
| 200 | 47.02 ms | 48.69 ms | 25.10 ms | 5.95 ms | Above 45 ms target |
| 400 | 47.43 ms | 49.69 ms | 25.15 ms | 5.81 ms | Pass |
| 600 | 47.79 ms | 49.23 ms | 25.28 ms | 4.78 ms | Pass |
| 800 | 47.51 ms | 50.01 ms | 25.71 ms | 4.71 ms | Pass |

A 60-second 200 Mbps retest produced a `47.62 ms` median. Reducing
`buffering_history_weight` from `0.9` to `0.5` reduced the decoder queue median
from `4.94 ms` to `3.61 ms` but left total latency at `47.66 ms`; the original
`0.9` value was restored.

Wired testing exposed a transport limit rather than a rendering regression:

- The VRX UDC negotiated `high-speed` (USB 2.0) even though it supports
  `super-speed-plus`.
- Measured ADB transfer throughput was only `23-32 MB/s`
  (`184-256 Mbps`).
- Wired medians were `48.22/92.14/102.14/101.25 ms` at
  `200/400/600/800 Mbps`.
- TCP transport medians were `9.78/43.51/54.08/51.66 ms`, while VSync stayed
  near `25.5 ms`.

The USB link also disconnected repeatedly during the run. A stable USB 3
SuperSpeed port and data cable are required before the wired target can be
re-evaluated meaningfully. No image-quality setting was reduced for these
tests.

After moving the VRX to a USB 3 port, the UDC reported `super-speed-plus` and
ADB push throughput increased to about `54 MB/s` (about 430 Mbps). Wired results
were:

| Mbps | Median | P90 | VSync median | Transport median | Result |
| ---: | ---: | ---: | ---: | ---: | --- |
| 200 | 36.85 ms | 37.57 ms | 25.16 ms | 2.09 ms | Pass |
| 400 | 37.32 ms | 38.86 ms | 25.64 ms | 2.09 ms | Pass |

The 600 and 800 Mbps runs were stopped when head tracking became untracked.
OpenXR reported `location_flags=3` and `view_flags=3` (valid but not tracked)
with a constant head position, and the QVR camera service continued reporting
`invalid frame cams`. The last known good position was around
`(-0.014, 1.072, -0.22)` and the lost-tracking pose was exactly
`(0.0, 1.613, 0.0)`. This is a runtime tracking-camera/boundary availability
problem, not an ALVR transport regression.

Wi-Fi link during wireless validation:

```text
SSID: CMCC-HpAU-5G-FAST
Frequency: 5240 MHz
RSSI: -17 dBm
Link speed: 2401 Mbps
Client IP: 192.168.1.10
```

## QXR findings

- `QXRCoreClient_Create` succeeds and initialization returns `2`.
- QXRCore and XRCBCore binder connections succeed.
- `device-mode` is read-only through QVR `SetParam` and returns `-15`.
- `openxr-device-config-file` is also read-only through QVR `SetParam` and
  returns `-4`.
- The client no longer contains compile-time `host_smart` or tuned-config write
  paths. Those historical probe APKs could not switch the device mode and must
  not be presented as successful `host_smart` tests.
- Direct-render mode alone did not reduce the VSync queue.
- The low-latency improvement comes from the runtime condition-wait behavior
  and lower decoder buffering target.

## Evidence

Raw logs are stored under `build/vrx_validation`:

```text
latency-wired-200-buffer0.1-20261029.log
latency-wired-400-buffer0.1-20261029.log
latency-wired-600-buffer0.1-retest-20261029.log
latency-wired-800-buffer0.1-20261029.log
latency-wireless-200-buffer0.1-20261029.log
latency-wireless-400-buffer0.1-20261029.log
latency-wireless-600-buffer0.1-20261029.log
latency-wireless-800-buffer0.1-20261029.log
```
