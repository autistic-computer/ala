"""Summarize published gzip captures; retain per-run results and trace limits."""

import gzip
from collections import Counter
import json
from pathlib import Path
import re
import statistics


root = Path(__file__).resolve().parent


def stats(values):
    if not values:
        return None
    ordered = sorted(values)
    return {"median": statistics.median(ordered), "p95": ordered[round((len(ordered) - 1) * .95)], "max": max(ordered)}


def gpu_frames(raw):
    # Mesa 26.0.8 leaves an unfinished frame at shutdown. Keep complete frames
    # without changing the raw trace or treating the missing suffix as a pass.
    decoder = json.JSONDecoder()
    pos, frames = 1, []
    while pos < len(raw):
        while pos < len(raw) and raw[pos] in " \r\n\t,":
            pos += 1
        if pos < len(raw) and raw[pos] == "]":
            return frames, 0
        try:
            value, end = decoder.raw_decode(raw, pos)
        except json.JSONDecodeError:
            return frames, len(raw) - pos
        frames.append(value)
        pos = end
    return frames, 0


output = {"exclude_startup_seconds": 1, "runs": [], "modes": {}}
for folder in ("live", "gpu", "gpu-reverse"):
    manifest_path = root / folder / "manifest.json"
    if not manifest_path.exists():
        continue
    manifest = json.loads(manifest_path.read_text())
    for run in manifest["runs"]:
        name = run["name"]
        path = root / folder / f"{name}.ndjson.gz"
        frames = [json.loads(line) for line in gzip.open(path, "rt")]
        start = frames[0]["timestamp_unix_ns"]
        steady = [f for f in frames if f["timestamp_unix_ns"] >= start + 1e9]
        mode = name.split("-", 1)[1]
        entry = {"capture": folder, "name": name, "mode": mode,
                 "total_frames": len(frames), "steady_frames": len(steady),
                 "startup_draw_wall_ms": frames[0]["draw"]["wall_ns"] / 1e6,
                 "startup_counts": frames[0]["counts"],
                 "last_draw_seconds": (frames[-1]["timestamp_unix_ns"] - start) / 1e9,
                 "last_window_settings": {"viewport_size": frames[-1]["viewport_size"], "scale_factor": frames[-1]["scale_factor"], "accessibility_active": frames[-1]["accessibility_active"]},
                 "present_submit_samples": sum(f["phases"]["present_submit"] is not None for f in steady),
                 "steady_window_settings": sorted({(f["viewport_size"]["width"], f["viewport_size"]["height"], f["scale_factor"], f["accessibility_active"]) for f in steady})}
        for phase in ("draw", "request_layout", "prepaint", "paint", "scene_finish", "present_submit"):
            times = [f["draw"] if phase == "draw" else f["phases"][phase] for f in steady]
            entry[phase + "_wall_ms"] = stats([t["wall_ns"] / 1e6 for t in times if t is not None])
            entry[phase + "_thread_cpu_ms"] = stats([t["thread_cpu_ns"] / 1e6 for t in times if t is not None and t["thread_cpu_ns"] is not None])
        entry["counts"] = {k: stats([f["counts"][k] for f in steady]) for k in frames[0]["counts"]}
        intervals = [(b["timestamp_unix_ns"] - a["timestamp_unix_ns"]) / 1e6 for a, b in zip(steady, steady[1:])]
        entry["draw_interval_ms"] = stats(intervals)
        entry["draw_hz"] = 1000 / statistics.mean(intervals) if intervals else None
        entry["draw_intervals_over_25ms"] = sum(t > 25 for t in intervals)
        text = (root / folder / f"{name}.time.txt").read_text()
        for label, key in (("User time (seconds)", "process_user_seconds"), ("System time (seconds)", "process_system_seconds"), ("Maximum resident set size (kbytes)", "peak_rss_kib")):
            entry[key] = float(re.search(re.escape(label) + r": (\S+)", text)[1])
        entry["process_cpu_seconds"] = entry["process_user_seconds"] + entry["process_system_seconds"]
        entry["gpu"] = next(json.loads(line)["gpu"] for line in (root / folder / f"{name}.log").read_text().splitlines() if line.startswith('{"live_mode"'))
        frequency_path = root / folder / f"{name}.frequency.json.gz"
        if frequency_path.exists():
            samples = json.load(gzip.open(frequency_path, "rt"))
            samples = [s for s in samples if s["monotonic_ns"] >= samples[0]["monotonic_ns"] + 1e9]
            entry["frequency"] = {"samples": len(samples), "histograms": {key: dict(Counter(s[key] for s in samples)) for key in ("requested_mhz", "actual_mhz", "punit_requested_mhz", "throttle_status")},
                "requested_mhz_median": statistics.median(s["requested_mhz"] for s in samples),
                "actual_mhz_median": statistics.median(s["actual_mhz"] for s in samples),
                "throttle_status_nonzero_samples": sum(s["throttle_status"] != 0 for s in samples)}
        gpu_path = root / folder / f"{name}-gpu.json.gz"
        if gpu_path.exists():
            complete, tail = gpu_frames(gzip.open(gpu_path, "rt").read())
            assert [f["frame"] for f in complete] == list(range(len(complete))), "GPU contexts or frame records overlap"
            assert tail < 2048, "GPU trace has more than one small unfinished final record"
            durations, first, last, total = [], None, None, 0
            for frame in complete:
                intervals = []
                for batch in frame["batches"]:
                    timestamps = [int(e["time_ns"]) for e in batch["events"]]
                    if timestamps:
                        intervals.append((min(timestamps), max(timestamps)))
                if not intervals:
                    continue
                intervals.sort()
                first = intervals[0][0] if first is None else first
                last = max(b for a, b in intervals)
                # Union batch spans; nested draw/render-pass events are never summed.
                low, high = intervals[0]
                ns = 0
                for a, b in intervals[1:]:
                    if a > high:
                        ns += high - low
                        low, high = a, b
                    else:
                        high = max(high, b)
                ns += high - low
                total += ns
                if intervals[0][0] >= first + 1e9:
                    durations.append(ns / 1e6)
            entry["gpu_timestamps"] = {"complete_frames": len(complete), "incomplete_tail_bytes": tail,
                "steady_frames": len(durations), "batch_span_union_ms": stats(durations),
                "total_batch_span_union_ms": total / 1e6, "last_activity_seconds": (last - first) / 1e9 if first is not None else None}
        output["runs"].append(entry)
for mode in ("baseline", "replay", "static"):
    runs = [r for r in output["runs"] if r["capture"] == "live" and r["mode"] == mode]
    if not runs:
        continue
    output["modes"][mode] = {"runs": len(runs), "total_frames": [r["total_frames"] for r in runs],
        "steady_frames": [r["steady_frames"] for r in runs],
        "startup_draw_wall_ms": stats([r["startup_draw_wall_ms"] for r in runs]),
        "process_cpu_seconds": stats([r["process_cpu_seconds"] for r in runs]),
        "peak_rss_kib": stats([r["peak_rss_kib"] for r in runs]),
        "draw_hz": stats([r["draw_hz"] for r in runs if r["draw_hz"] is not None])}
    for phase in ("draw", "request_layout", "prepaint", "paint", "scene_finish", "present_submit"):
        output["modes"][mode][phase + "_wall_ms"] = {key: statistics.median(r[phase + "_wall_ms"][key] for r in runs if r[phase + "_wall_ms"] is not None) for key in ("median", "p95")} if any(r[phase + "_wall_ms"] for r in runs) else None
(root / "live.summary.json").write_text(json.dumps(output, indent=2) + "\n")
print(json.dumps(output["modes"], indent=2))
