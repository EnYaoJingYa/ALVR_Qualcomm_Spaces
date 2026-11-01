"""Summarize only completed wired natural-scene samples with both measurement files."""
import hashlib
import json
import statistics
from pathlib import Path

OUT = Path(__file__).resolve().parent / "results"


def main() -> None:
    rows = {}
    for name in ("A1", "observe", "frequency", "B1", "A2", "B2"):
        prefix = OUT / ("static_" + name)
        summary = json.loads(Path(f"{prefix}-summary.json").read_text())
        frames = json.loads(Path(f"{prefix}-frames.json").read_text())
        if not summary["valid"]:
            raise RuntimeError(f"Invalid {name}")
        video = hashlib.sha256(json.dumps(summary["video"], sort_keys=True).encode()).hexdigest()
        events = [json.loads(line) for line in prefix.with_suffix(".jsonl").read_text(encoding="utf-8").splitlines()]
        graphs = [e["data"] for e in events if e["phase"] == "sample" and e["kind"] == "graph"]
        deltas = [e["data"]["content"].rsplit("correction_ns=",1)[-1] for e in events
                  if e["phase"] == "sample" and e["kind"] == "server_log"
                  and "event=vrx_lenovo_candidate" in e["data"]["content"]]
        rows[name] = dict(mtp=summary["latency_ms"], **frames, video_hash=video,
                          dashboard=summary["dashboard"], reports=summary["reports"],
                          diagnostics=dict(candidate_count=len(deltas), nonzero=sum(int(v)!=0 for v in deltas)),
                          breakdown_ms={k:statistics.median(g[k] for g in graphs)*1000
                                        for k in ("encoder_s","network_s","decoder_s","decoder_queue_s","vsync_queue_s")})
    assert len({r["video_hash"] for r in rows.values()}) == 1
    assert len({r["dashboard"] for r in rows.values()}) == 1
    (OUT/"comparison.json").write_text(json.dumps(rows,indent=2))
    for name,r in rows.items():
        print(name, r["mtp"], r["new_fps"], r["reused"], r["diagnostics"])


if __name__ == "__main__":
    main()
