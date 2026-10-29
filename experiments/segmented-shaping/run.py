#!/usr/bin/env python3
"""Reproduce this independent ALA experiment on Linux, macOS or Windows.

Run from the repository root with Python 3.10+ and the pinned Rust toolchain:
  python3 experiments/segmented-shaping/run.py --build --verify-only
  python3 experiments/segmented-shaping/run.py --build --repetitions 5 --samples 120

Timing excludes GPU submission. On Linux --cpu can pin a logical CPU. Omit it on
macOS and Windows. Keep source, build options, fonts and window state matched.
The native backend and all correctness limitations belong with the saved results.
"""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time


def command(args):
    result = subprocess.run(args, text=True, capture_output=True, check=True)
    return result.stdout.strip()


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[round((len(ordered) - 1) * fraction)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--build", action="store_true")
    parser.add_argument("--verify-only", action="store_true")
    parser.add_argument("--output", type=Path, default=Path("experiments/segmented-shaping/real-machine.json"))
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--samples", type=int, default=120)
    parser.add_argument("--cpu", type=int)
    args = parser.parse_args()
    if args.cpu is not None:
        if not hasattr(os, "sched_setaffinity"):
            parser.error("--cpu is supported only on Linux; omit it on macOS/Windows")
        os.sched_setaffinity(0, {args.cpu})
    root = Path(__file__).resolve().parents[2]
    os.chdir(root)
    build_command = None
    if args.build:
        build_command = ["cargo", "bench", "--locked", "-p", "ala-platform", "--no-default-features", "--features", "bench-support", "--bench", "segmented_shaping", "--no-run", "--message-format=json"]
        built = subprocess.run(build_command, text=True, capture_output=True)
        if built.returncode:
            raise RuntimeError(built.stderr + built.stdout)
        artifacts = [json.loads(line) for line in built.stdout.splitlines() if line.startswith("{")]
        executable = next(a["executable"] for a in artifacts if a.get("reason") == "compiler-artifact" and a.get("target",{}).get("name") == "segmented_shaping" and a.get("executable"))
        binary = Path(executable).resolve()
    elif args.binary:
        binary = args.binary.resolve()
    else:
        parser.error("use --build or --binary")
    metadata = {
        "experimental":True,
        "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "platform": platform.platform(),
        "text_backend": {"Linux":"cosmic-text","Darwin":"CoreText","Windows":"DirectWrite"}.get(platform.system()),
        "cpuinfo": next((line.split(":", 1)[1].strip()
            for line in Path("/proc/cpuinfo").read_text().splitlines()
            if line.startswith("model name")), None) if Path("/proc/cpuinfo").exists() else None,
        "cpu_affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "source_commit": command(["git", "rev-parse", "HEAD"]),
        "baseline_commit": "b9b22210d37bd7157161a238f602136a65095723",
        "build_command":build_command,
        "tracked_diff_sha256": hashlib.sha256(command(["git", "diff", "HEAD"]).encode()).hexdigest(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "rustc": command(["rustc", "--version"]),
        "measurements": "Main-thread text edit + layout + draw + arena clear; no GPU submission. Separate allocation passes.",
        "allocation_note": "Separate instrumented processes count allocation requests, not retained memory or RSS. Counting disabled in timing runs.",
        "shaping_scope": "Benchmark-only decorator around the native platform text system; semantic differences block merging",
        "donor_commits": {
            "segment_helpers": "mesa-hills-research/gpui@30d883237d5f913f2d11cfcb938bd7697da88aa6",
        },
        "source_sha256": {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in [root / "crates/ala-platform/benches/segmented_shaping.rs",
                         *sorted((root / "crates/ala-platform/benches/segmented_shaping").glob("*.rs")),
                         root / "Cargo.lock", Path(__file__).resolve()]},
    }

    env = {key: os.environ[key] for key in (
        "VK_DRIVER_FILES", "VK_ICD_FILENAMES", "LP_NUM_THREADS",
        "CARGO_PROFILE_RELEASE_DEBUG", "CARGO_PROFILE_RELEASE_LTO",
    ) if key in os.environ}
    metadata["relevant_environment"] = env
    def run(flags):
        started = time.monotonic()
        result = subprocess.run([str(binary), *flags], text=True, capture_output=True)
        if result.returncode:
            raise RuntimeError(f"{binary.name} exited {result.returncode}:\n{result.stderr}\n{result.stdout}")
        value = json.loads(result.stdout)
        value["wall_seconds"] = time.monotonic() - started
        value["command"] = [str(binary), *flags]
        if result.stderr:
            value["stderr"] = result.stderr
        return value
    print("Verifying this experiment...", flush=True)
    verification = run(["--verify"])
    blocked = not verification["text"]["layout_equivalent"]
    output = {"metadata": metadata, "verification": verification, "correctness_blocked": blocked, "runs": [], "summary": []}
    if blocked:
        print("CORRECTNESS BLOCKED: native line layouts differ; timing results are diagnostic only.", flush=True)
    if args.verify_only:
        args.output.parent.mkdir(parents=True,exist_ok=True)
        args.output.write_text(json.dumps(output,indent=2)+"\n")
        print(json.dumps(verification,indent=2))
        return
    cases = []
    for size in (2000, 20000, 100000):
        for workload in ("append", "middle", "cold", "unchanged"):
            cases.append((f"text/{size}/{workload}",
                          ["--case", "text", "--bytes", str(size), "--workload", workload],
                          ("baseline", "segmented")))
    def save():
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(output, separators=(",",":")) + "\n")
        args.output.with_suffix(".summary.json").write_text(json.dumps({k:v for k,v in output.items() if k != "runs"},indent=2)+"\n")
    save()
    for name, flags, modes in cases:
        medians = {mode: [] for mode in modes}
        allocations = {mode: [] for mode in modes}
        tails = {mode: [] for mode in modes}
        cpu_medians = {mode: [] for mode in modes}
        for repetition in range(args.repetitions):
            # Reverse the order on alternate repetitions to expose warm-up/order bias.
            order = modes if repetition % 2 == 0 else tuple(reversed(modes))
            for mode in order:
                result = run([*flags, "--mode", mode, "--samples", str(args.samples)])
                assert result["retained_glyphs"] <= 262144, result
                if result["case"] == "animation":
                    expected = 0 if mode == "replay" else args.samples
                    assert result["pane_renders"] == expected, result
                result.update(name=name, repetition=repetition)
                medians[mode].append(statistics.median(s["ns"] for s in result["samples"]))
                tails[mode].append(percentile([s["ns"] for s in result["samples"]], 0.95))
                cpu_medians[mode].append(statistics.median(s["cpu_ns"] for s in result["samples"]))
                output["runs"].append(result)
        for mode in modes:
            result = run([*flags, "--mode", mode, "--samples", str(args.samples), "--count-allocations"])
            assert result["retained_glyphs"] <= 262144, result
            result.update(name=name, allocation_only=True)
            allocations[mode].append(statistics.median(s["allocated_bytes"] for s in result["samples"]))
            output["runs"].append(result)
        baseline = statistics.median(medians[modes[0]])
        summary = {
            "name": name,
            "modes": {
                mode: {
                    "median_ms": statistics.median(medians[mode]) / 1e6,
                    "p95_ms": statistics.median(tails[mode]) / 1e6,
                    "run_medians_ms": [value / 1e6 for value in medians[mode]],
                    "thread_cpu_median_ms": statistics.median(cpu_medians[mode]) / 1e6,
                    "paired_speedup_range": [
                        min(a / b for a, b in zip(medians[modes[0]], medians[mode])),
                        max(a / b for a, b in zip(medians[modes[0]], medians[mode])),
                    ],
                    "median_allocated_bytes": statistics.median(allocations[mode]),
                    "speedup_vs_baseline": baseline / statistics.median(medians[mode]),
                } for mode in modes
            },
        }
        output["summary"].append(summary)
        print(name + ": " + ", ".join(
            f"{mode}={value['median_ms']:.4f}ms (p95 {value['p95_ms']:.4f}, {value['speedup_vs_baseline']:.2f}x)"
            for mode, value in summary["modes"].items()), flush=True)
        save()
    print(f"Saved {len(output['runs'])} runs to {args.output}", flush=True)


if __name__ == "__main__":
    main()
