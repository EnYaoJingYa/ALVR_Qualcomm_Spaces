"""Exercise the installed active Lenovo build across wired session boundaries."""

import json
import re
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT / "analysis_official_streaming"))
import vrx_stability_monitor as monitor

STREAMER = ROOT / "ALVR_Qualcomm_Spaces/build/alvr_streamer_windows"
LOG = STREAMER / "session_log.txt"
SESSION = STREAMER / "session.json"
PHASE = re.compile(r"event=vrx_lenovo_received session=(\d+).*? vc=(\d+).*? active=true")
OUT = HERE / "results" / ("endurance-" + time.strftime("%Y%m%d-%H%M%S") + ".json")


def tail():
    with LOG.open("rb") as stream:
        stream.seek(max(0, LOG.stat().st_size - 262144))
        return stream.read().decode(errors="replace")


def wired():
    clients = json.loads(SESSION.read_bytes())["client_connections"]
    return clients["client.wired"]["connection_state"] == "Streaming"


def phase():
    records = PHASE.findall(tail())
    return tuple(map(int, records[-1])) if records else (0, 0)


def recover(kind):
    old, _ = phase()
    start = time.monotonic()
    if kind == "cold_start":
        monitor.adb("shell", "am", "force-stop", "alvr.client.dev")
        time.sleep(2)
        monitor.adb("shell", "am", "start", "-n", "alvr.client.dev/android.app.NativeActivity")
    else:
        monitor.request("session/client-connections", ["client.wired", {"SetConnectionState": "Disconnecting"}])
    while time.monotonic() - start < 60:
        time.sleep(1)
        current, first = phase()
        if current > old and wired():
            time.sleep(2)
            confirmed, second = phase()
            if confirmed == current and second > first and wired() and "[GRAPH]" in tail():
                return {"session": current, "seconds": round(time.monotonic() - start, 2)}
    raise RuntimeError(f"{kind} failed to re-establish advancing wired feedback after session {old}")


def soak(result, results):
    started = time.monotonic()
    last = next_health = started
    latencies = []
    session, vc = phase()
    ws = monitor.websocket.create_connection("ws://127.0.0.1:8082/api/events", header=["X-ALVR: 1"], timeout=1)
    try:
        while time.monotonic() - started < 1800:
            if (HERE / "results" / "stop-endurance").exists():
                result["ended_by"] = "user"
                break
            try:
                event = json.loads(ws.recv())["event_type"]
            except monitor.websocket.WebSocketTimeoutException:
                event = {}
            now = time.monotonic()
            if event.get("id") == "GraphStatistics":
                latencies.append(event["data"]["total_pipeline_latency_s"] * 1000)
                last = now
            if now - last > 5:
                raise RuntimeError("Video statistics stopped for five seconds")
            if now >= next_health:
                current, count = phase()
                if not wired() or current != session or (result["health"] and count <= vc):
                    raise RuntimeError("Wired session changed or phase counter stalled")
                vc = count
                battery = int(monitor.adb("shell", "cat /sys/class/power_supply/battery/capacity"))
                result["health"].append({"seconds": round(now - started), "battery": battery, "vc": vc})
                result.update(seconds=round(now - started, 2), graphs=len(latencies),
                              mtp_ms=monitor.describe(latencies))
                OUT.write_text(json.dumps(results, indent=2))
                if battery <= 10:
                    raise RuntimeError("Battery reserve reached")
                next_health = now + 30
    finally:
        ws.close()
        result.update(seconds=round(time.monotonic() - started, 2), graphs=len(latencies),
                      mtp_ms=monitor.describe(latencies))


def main():
    results = {"cold_start": [], "reconnect": [], "soak": [], "passed": False}
    try:
        if not wired() or not phase()[0]:
            raise RuntimeError("Active Lenovo wired feedback required")
        for kind in ("cold_start", "reconnect"):
            for _ in range(10):
                results[kind].append(recover(kind))
                OUT.write_text(json.dumps(results, indent=2))
        for _ in range(2):
            result = {"health": []}
            results["soak"].append(result)
            soak(result, results)
            OUT.write_text(json.dumps(results, indent=2))
            if result.get("ended_by") == "user":
                return
        results["passed"] = True
    except Exception as error:
        results["failure"] = str(error)
        raise
    finally:
        OUT.write_text(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
