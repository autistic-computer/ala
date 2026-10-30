#!/usr/bin/env python3
"""Reproduce this independent ALA experiment on Linux, macOS or Windows.

Run from the repository root with Python 3.10+ and the pinned Rust toolchain:
  python3 experiments/paint-replay/run.py --build --verify-only
  python3 experiments/paint-replay/run.py --build --repetitions 5 --samples 120

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
    parser.add_argument("--live", choices=("baseline","replay","static"))
    parser.add_argument("--seconds",type=int,default=45)
    parser.add_argument("--rows",type=int,default=480)
    parser.add_argument("--output", type=Path, default=Path("experiments/paint-replay/real-machine.json"))
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
        build_command = ["cargo", "bench", "--locked", "-p", "ala-platform", "--no-default-features", "--features", "bench-support,perf-experiments", "--bench", "paint_replay", "--no-run", "--message-format=json"]
        if args.live and platform.system() == "Linux":
            build_command[build_command.index("--features")+1] += ",wayland,x11"
        built = subprocess.run(build_command, text=True, capture_output=True)
        if built.returncode:
            raise RuntimeError(built.stderr + built.stdout)
        artifacts = [json.loads(line) for line in built.stdout.splitlines() if line.startswith("{")]
        executable = next(a["executable"] for a in artifacts if a.get("reason") == "compiler-artifact" and a.get("target",{}).get("name") == "paint_replay" and a.get("executable"))
        binary = Path(executable).resolve()
    elif args.binary:
        binary = args.binary.resolve()
    else:
        parser.error("use --build or --binary")
    if args.live:
        subprocess.run([str(binary),"--live","--mode",args.live,"--seconds",str(args.seconds),"--rows",str(args.rows)],check=True)
        return
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
        "measurements": "Main-thread elapsed update + draw + arena clear, without GPU submission; thread CPU time also recorded. Allocation passes are separate.",
        "allocation_note": "Separate instrumented processes count allocation requests, not retained memory or RSS. Counting disabled in timing runs.",
        "paint_scope": "Manually driven opacity replay, not the donor animation scheduler or a production animation API",
        "donor_commits": {
            "paint_records": "mesa-hills-research/gpui@eb63578acb9637ba72146dfafc07e4e1cb03d16b",
        },
        "source_sha256": {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in [root / "crates/ala-platform/benches/paint_replay.rs",
                         root / "crates/ala/src/scene.rs", root / "crates/ala/src/window.rs",
                         root / "crates/ala/src/window/perf_experiments.rs",
                         root / "Cargo.lock", Path(__file__).resolve()]},
    }

    env = {key: os.environ[key] for key in (
        "VK_DRIVER_FILES", "VK_ICD_FILENAMES", "LP_NUM_THREADS",
        "CARGO_PROFILE_RELEASE_DEBUG", "CARGO_PROFILE_RELEASE_LTO",
    ) if key in os.environ}
    metadata["relevant_environment"] = env
    def run(flags):
        started = time.monotonic()
        result = subprocess.run([str(binary), *flags], text=True, capture_output=True, check=True)
        value = json.loads(result.stdout)
        value["wall_seconds"] = time.monotonic() - started
        value["command"] = [str(binary), *flags]
        if result.stderr:
            value["stderr"] = result.stderr
        return value
    print("Verifying this experiment...", flush=True)
    verification = run(["--verify"])
    output = {"metadata": metadata, "verification": verification, "runs": [], "summary": []}
    if args.verify_only:
        args.output.parent.mkdir(parents=True,exist_ok=True)
        args.output.write_text(json.dumps(output,indent=2)+"\n")
        print(json.dumps(verification,indent=2))
        return
    cases = []
    for rows in (30, 120, 480):
        cases.append((f"animation/{rows}", ["--case", "animation", "--rows", str(rows)],
                      ("baseline", "replay")))
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
