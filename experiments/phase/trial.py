"""Use identical headset render probes and existing ALVR statistics on A and B."""
import json
import ctypes
import re
import shlex
import struct
import subprocess
import sys
import time
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT / "analysis_official_streaming"))
import vrx_config_trial as trial
import vrx_stability_monitor as monitor

OUT = HERE / "results"
OUT.mkdir(exist_ok=True)
monitor.OUTPUT = trial.OUTPUT = OUT
ADB = ["E:/platform-tools/adb.exe", "-s", "HLA50582"]
TOOLS = Path("E:/Gao_Ji/Ni_Xiang/android-sdk/ndk/25.2.9519653/toolchains/llvm/prebuilt/windows-x86_64/bin")
TRACE = "/sys/kernel/tracing"


def shell(command: str) -> str:
    return subprocess.check_output([*ADB, "shell", command], encoding="utf-8", errors="replace")


def file_offset(apk: Path) -> int:
    with zipfile.ZipFile(apk) as archive:
        data = archive.read("lib/arm64-v8a/libalvr_client_openxr.so")
    lib = OUT / "symbols.so"
    lib.write_bytes(data)
    symbols = subprocess.check_output([TOOLS/"llvm-nm.exe", "-C", lib], encoding="utf-8")
    match = re.search(r"^([0-9a-f]+) t <alvr_graphics::stream::StreamRenderer>::render$", symbols, re.M)
    if not match:
        raise RuntimeError("Missing render symbol")
    va = int(match[1], 16)
    ph = struct.unpack_from("<Q", data, 32)[0]
    size, count = struct.unpack_from("<HH", data, 54)
    for index in range(count):
        typ, _, offset, base, _, length = struct.unpack_from("<IIQQQQ", data, ph + index*size)
        if typ == 1 and base <= va < base+length:
            return offset + va-base
    raise RuntimeError("Render entry not mapped")


class SampleMonitor(monitor.Monitor):
    def observe(self, seconds: float, require_stream: bool = True) -> None:
        super().observe(seconds, require_stream)
        if self.phase == "warmup":
            shell(f"echo 1 > {instance}/events/{event}/render/enable; echo start > {instance}/trace_marker")

    def live_session(self):
        if getattr(self,"phase",None) == "sample":
            shell(f"echo end > {instance}/trace_marker; echo 0 > {instance}/events/{event}/render/enable")
        return super().live_session()


def stationary_sample(label: str) -> None:
    observer = SampleMonitor(label)
    try:
        observer.await_stream(45)
        session = observer.live_session()
        video = session["session_settings"]["video"]
        assert video["preferred_codec"]["variant"] == "H264" and video["bitrate"]["mode"]["ConstantMbps"] == 100
        err = ctypes.c_int()
        table = observer.vr.VR_GetGenericInterface(b"FnTable:IVRSystem_026",ctypes.byref(err))
        if err.value or not table:
            raise RuntimeError("OpenVR pose unavailable")
        get_pose = ctypes.WINFUNCTYPE(None,ctypes.c_int,ctypes.c_float,ctypes.POINTER(trial.Pose),ctypes.c_uint32)(
            ctypes.cast(table, ctypes.POINTER(ctypes.c_void_p))[12])
        observer.phase = "warmup"
        observer.observe(15)
        observer.rows.clear()
        observer.phase = "sample"
        offsets = {n:Path(str(observer.prefix)+f"-{n}.log").stat().st_size for n in ("android","system")}
        visible = bool(observer.dashboard())
        start = time.monotonic()
        poses = []
        while time.monotonic()-start < 30:
            observer.observe(1)
            pose = trial.Pose()
            get_pose(1,0,ctypes.byref(pose),1)
            poses.append(list(pose.matrix))
            if not pose.valid or not pose.connected or bool(observer.dashboard()) != visible:
                raise RuntimeError("Pose or dashboard state changed validity")
        end = observer.live_session()
        same = end["session_settings"]["video"] == video
        if not same or end["client_connections"]["client.wired"]["connection_state"] != "Streaming":
            raise RuntimeError("Wired configuration changed")
        log = ""
        for n,off in offsets.items():
            with Path(str(observer.prefix)+f"-{n}.log").open("rb") as f:
                f.seek(off)
                log += f.read().decode(errors="replace")
        critical = [k for k in ("Fatal decoder error","HEAD_TRACKING_LOST","panicked at","decoder_admission_timeout","Session Lost") if k in log]
        if critical:
            raise RuntimeError(str(critical))
        result = {"valid":True,"scene":"stationary","head_motion_accepted":False,"dashboard":visible,
                  "video":video,"reports":len(observer.rows),"unique_poses":len(set(map(tuple,poses))),
                  "latency_ms":monitor.describe([r["total_pipeline_latency_s"]*1000 for r in observer.rows])}
        (OUT/f"{label}-summary.json").write_text(json.dumps(result))
        print(result["latency_ms"],flush=True)
    finally:
        observer.close()


def summarize_trace(trace: str, label: str) -> None:
    stamps = re.findall(r"(\d+\.\d+): tracing_mark_write: (start|end)", trace)
    start, end = (float(next(t for t, marker in stamps if marker == key)) for key in ("start","end"))
    values = [int(buf,0) for t,buf in re.findall(r"(\d+\.\d+): render:.*?buf=(0x[0-9a-f]+|[0-9]+)", trace)
              if start <= float(t) <= end]
    if not values or not 29 <= end-start <= 32:
        raise RuntimeError(f"Invalid render trace: {len(values)} records in {end-start}s")
    result = dict(seconds=end-start, draws=len(values), reused=values.count(0),
                  new_fps=(len(values)-values.count(0))/(end-start),
                  repeat_fraction=values.count(0)/len(values))
    (OUT/f"{label}-frames.json").write_text(json.dumps(result))
    print(result)

if __name__ == "__main__":
    label, apk = sys.argv[1:3]
    if not re.fullmatch(r"[A-Za-z0-9_]+", label):
        raise ValueError(label)
    event = "alvr_" + label
    instance = f"{TRACE}/instances/{event}"
    path = shell("cat /proc/$(pidof alvr.client.dev)/maps").splitlines()
    so = next(line.split()[-1] for line in path if line.endswith("/libalvr_client_openxr.so"))
    offset = file_offset(Path(apk))
    register = shlex.quote(f"p:{event}/render {so}:0x{offset:x} buf=%x1:u64")
    shell(f"mkdir {instance}")
    try:
        shell(f"echo {register} >> {TRACE}/uprobe_events")
        stationary_sample(label)
        summary = json.loads((OUT/f"{label}-summary.json").read_text())
        if not summary["valid"]:
            raise SystemExit(f"Invalid trial {label}: {summary.get('error', 'health/configuration gate')}")
        trace = shell(f"cat {instance}/trace")
        (OUT/f"{label}-trace.txt").write_text(trace)
        summarize_trace(trace, label)
    finally:
        shell(f"rmdir {instance}")
        shell(f"echo '-:{event}/render' >> {TRACE}/uprobe_events")
