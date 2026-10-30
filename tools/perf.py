#!/usr/bin/env python3
"""Reproducible ala CPU benchmarks, frame diagnostics, and Linux sampling.

Only Python's standard library is required. See docs/performance.md.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
FEATURES = "bench-support,font-kit,wayland,x11"


def capture(command):
    result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    return result.stdout.strip() if result.returncode == 0 else None


def read(path):
    try:
        return Path(path).read_text().strip()
    except OSError:
        return None


def cargo():
    found = shutil.which("cargo") or Path.home() / ".cargo/bin/cargo"
    if not Path(found).exists():
        raise RuntimeError("Install the Rust toolchain pinned in rust-toolchain.toml")
    return str(found)


def build_env():
    env = os.environ.copy()
    flags = env.get("RUSTFLAGS", "")
    if not re.search(r"(?:^|\s)-C\s*force-frame-pointers=yes(?:\s|$)", flags):
        flags = (flags + " -C force-frame-pointers=yes").strip()
    env["RUSTFLAGS"] = flags
    return env


def event_probe(kind, config):
    syscall = {"x86_64": 298, "aarch64": 241}.get(platform.machine())
    if sys.platform != "linux" or syscall is None:
        return "probe unsupported on this platform"
    attr = bytearray(128)
    struct.pack_into("IIQ", attr, 0, kind, 128, config)
    struct.pack_into("Q", attr, 40, (1 << 5) | (1 << 6))  # user space only
    libc = ctypes.CDLL(None, use_errno=True)
    buf = ctypes.create_string_buffer(bytes(attr))
    fd = libc.syscall(syscall, ctypes.byref(buf), 0, -1, -1, 0)
    if fd < 0:
        return os.strerror(ctypes.get_errno())
    os.close(fd)
    return "available"


def machine():
    cpuinfo = read("/proc/cpuinfo") or ""
    model = re.search(r"^model name\s*:\s*(.+)$", cpuinfo, re.M)
    return {
        "system": platform.platform(),
        "host": platform.node(),
        "cpu": model.group(1) if model else platform.processor(),
        "logical_cpus": os.cpu_count(),
        "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "cpu_quota": read("/sys/fs/cgroup/cpu.max"),
        "governor": read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
    }


def doctor():
    rustc = Path(cargo()).with_name("rustc")
    return {
        "machine": machine(),
        "rust": capture([str(rustc), "--version"]),
        "tools": {name: shutil.which(name) for name in ("perf", "valgrind", "vulkaninfo", "c++filt", "inferno-collapse-perf", "inferno-flamegraph")},
        "perf_user_cpu_clock": event_probe(1, 0),
        "perf_user_cycles": event_probe(0, 0),
        "gpu_devices": [str(p) for p in Path("/dev/dri").glob("render*")],
        "display": os.environ.get("WAYLAND_DISPLAY") or os.environ.get("DISPLAY"),
        "perf_event_paranoid": read("/proc/sys/kernel/perf_event_paranoid"),
    }


def build(bench):
    command = [cargo(), "bench", "--locked", "-p", "ala-platform", "--profile", "perf",
               "--features", FEATURES, "--bench", bench, "--no-run", "--message-format=json"]
    process = subprocess.Popen(command, cwd=ROOT, env=build_env(), text=True, stdout=subprocess.PIPE)
    executable = None
    for line in process.stdout:
        try:
            message = json.loads(line)
        except ValueError:
            print(line, end="", file=sys.stderr)
            continue
        if message.get("reason") == "compiler-message":
            print(message["message"].get("rendered", ""), end="", file=sys.stderr)
        if message.get("reason") == "compiler-artifact" and message.get("executable") and message["target"]["name"] == bench:
            executable = message["executable"]
    if process.wait() or executable is None:
        raise RuntimeError("Benchmark build failed")
    return executable


def output_dir(label):
    if not re.fullmatch(r"[a-zA-Z0-9_.-]+", label) or label in (".", ".."):
        raise RuntimeError("Use a label containing letters, numbers, underscores, dots, or hyphens")
    path = ROOT / "target/perf-results" / label
    path.mkdir(parents=True, exist_ok=False)
    return path


def write_json(path, data):
    path.write_text(json.dumps(data, indent=2) + "\n")


def execute(command, env, log):
    with log.open("w") as output:
        process = subprocess.Popen(command, cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        for line in process.stdout:
            output.write(line)
            print(line, end="", flush=True)
        status = process.wait()
    if status:
        raise RuntimeError(f"Command failed ({status}); see {log}")


def metadata(args, binary):
    diff = subprocess.check_output(["git", "diff", "HEAD", "--", "."], cwd=ROOT)
    # Include untracked source files in the worktree fingerprint without storing their content.
    digest = hashlib.sha256(diff)
    for name in subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard"], cwd=ROOT, text=True).splitlines():
        digest.update(name.encode())
        digest.update((ROOT / name).read_bytes())
    with open(binary, "rb") as source:
        binary_hash = hashlib.file_digest(source, "sha256").hexdigest()
    return {
        "schema_version": 1,
        "commit": capture(["git", "rev-parse", "HEAD"]),
        "worktree_sha256": digest.hexdigest(),
        "dirty": bool(capture(["git", "status", "--porcelain"])),
        "machine": machine(),
        "rust": capture([str(Path(cargo()).with_name("rustc")), "--version"]),
        "features": FEATURES,
        "rustflags": build_env()["RUSTFLAGS"],
        "bench": args.bench,
        "filter": args.filter,
        "binary": binary,
        "binary_sha256": binary_hash,
        "renderer": "none (CPU UI pipeline only)" if args.bench == "ui_cpu" else "headless (verify adapter separately)",
        "runtime": {key: os.environ.get(key) for key in ("VK_DRIVER_FILES", "VK_ICD_FILENAMES", "LP_NUM_THREADS", "ALA_PERF_SYSROOT")},
    }


def estimates(path):
    result = {}
    for entry in sorted((path / "criterion").rglob("new/estimates.json")):
        identity = json.loads(entry.with_name("benchmark.json").read_text())["full_id"]
        values = json.loads(entry.read_text())
        mean = values["mean"]
        result[identity] = {"mean_ns": mean["point_estimate"], "lower_ns": mean["confidence_interval"]["lower_bound"],
                            "upper_ns": mean["confidence_interval"]["upper_bound"], "median_ns": values["median"]["point_estimate"]}
    return result


def snapshot(args):
    binary = build(args.bench)
    path = output_dir(args.label)
    saved = path / ("benchmark.exe" if sys.platform == "win32" else "benchmark")
    shutil.copy2(binary, saved)
    write_json(path / "variant.json", metadata(args, str(saved)))
    print(f"Frozen benchmark and build metadata: {path}")


def prepare(args):
    if not args.variant:
        binary = build(args.bench)
        return binary, metadata(args, binary)
    variant = Path(args.variant).resolve()
    saved = json.loads((variant / "variant.json").read_text())
    binary = str(variant / ("benchmark.exe" if sys.platform == "win32" else "benchmark"))
    if saved["bench"] != args.bench:
        raise RuntimeError("Frozen variant belongs to a different benchmark")
    info = metadata(args, binary)
    if info["binary_sha256"] != saved["binary_sha256"]:
        raise RuntimeError("Frozen benchmark hash does not match its build metadata")
    for key in ("commit", "worktree_sha256", "dirty", "rust", "features", "rustflags"):
        info[key] = saved[key]
    info["variant"] = str(variant)
    return binary, info


def run(args):
    binary, info = prepare(args)
    path = output_dir(args.label)
    env = os.environ.copy()
    # Instrumentation changes timings; phase recordings are separate diagnostic runs.
    env.pop("ALA_FRAME_STATS", None)
    if args.frames:
        env["ALA_FRAME_STATS"] = str(path / "frames.jsonl")
    env["CRITERION_HOME"] = str(path / "criterion")
    command = [binary, "--bench", "--noplot", "--warm-up-time", str(args.warmup),
               "--measurement-time", str(args.measurement), "--sample-size", str(args.samples)]
    if args.filter:
        command.append(args.filter)
    info.update({"frames": args.frames, "warmup": args.warmup, "measurement": args.measurement, "samples": args.samples, "command": command})
    write_json(path / "metadata.json", info)
    execute(command, env, path / "run.log")
    values = estimates(path)
    if not values:
        raise RuntimeError("No Criterion measurements found; check the benchmark filter")
    write_json(path / "summary.json", values)
    for name, value in values.items():
        print(f"{name}: {value['mean_ns'] / 1000:.2f} us (95% CI {value['lower_ns'] / 1000:.2f}–{value['upper_ns'] / 1000:.2f})")
    print(f"Results: {path}")


def compare(args):
    base, candidate = [Path(value).resolve() for value in (args.baseline, args.candidate)]
    old_meta, new_meta = [json.loads((path / "metadata.json").read_text()) for path in (base, candidate)]
    keys = ("machine", "rust", "features", "rustflags", "bench", "filter", "renderer", "runtime", "frames", "warmup", "measurement", "samples")
    mismatch = [key for key in keys if old_meta.get(key) != new_meta.get(key)]
    if mismatch:
        raise RuntimeError("Incompatible runs: " + ", ".join(mismatch))
    old, new = [json.loads((path / "summary.json").read_text()) for path in (base, candidate)]
    if old.keys() != new.keys():
        raise RuntimeError("Runs contain different benchmark cases")
    print("Observed mean changes; negative is faster. Repeat interleaved A/B runs before claiming a gain.")
    for name in old:
        delta = (new[name]["mean_ns"] / old[name]["mean_ns"] - 1) * 100
        overlap = new[name]["lower_ns"] <= old[name]["upper_ns"] and old[name]["lower_ns"] <= new[name]["upper_ns"]
        print(f"{name}: {delta:+.2f}%" + (" (mean confidence intervals overlap)" if overlap else ""))


def quantiles(values):
    values = sorted(values)
    def percentile(q):
        index = (len(values) - 1) * q
        lo = int(index)
        return values[lo] + (values[min(lo + 1, len(values) - 1)] - values[lo]) * (index - lo)
    return {"samples": len(values), "mean": sum(values) / len(values), **{f"p{q}": percentile(q / 100) for q in (50, 95, 99)}}


def frames(args):
    series, count_sums, n, skipped = {}, {}, 0, 0
    with Path(args.path).open() as source:
        for line in source:
            frame = json.loads(line)
            if frame["counts"]["elements"] < args.min_elements:
                skipped += 1
                continue
            n += 1
            timings = {"draw": frame["draw"], **frame["phases"]}
            for phase, timing in timings.items():
                if timing:
                    for clock, value in timing.items():
                        if value is not None:
                            series.setdefault(f"{phase}.{clock}", []).append(value / 1e6)
            for key, value in frame["counts"].items():
                count_sums[key] = count_sums.get(key, 0) + value
    if not n:
        raise RuntimeError("No frames recorded")
    report = {"frames": n, "skipped_frames": skipped, "note": "Diagnostic timings in ms, including setup and Criterion warmup/calibration. Submission is not GPU completion or scanout.",
              "timings_ms": {key: quantiles(values) for key, values in series.items()},
              "mean_counts_per_frame": {key: value / n for key, value in count_sums.items()}}
    print(json.dumps(report, indent=2))


def profile(args):
    if event_probe(1, 0) != "available" or not shutil.which("perf"):
        raise RuntimeError("User-space perf CPU sampling is unavailable; run doctor for details")
    binary, info = prepare(args)
    path = output_dir(args.label)
    env = os.environ.copy()
    env.pop("ALA_FRAME_STATS", None)
    env["CRITERION_HOME"] = str(path / "criterion")
    command = ["perf", "record", "-e", "cpu-clock:u", "-F", "499", "--call-graph", "dwarf,16384", "-o", str(path / "perf.data"),
               "--", binary, "--bench", "--profile-time", str(args.seconds), args.filter]
    info.update({"command": command, "seconds": args.seconds, "event": "cpu-clock:u", "perf": capture(["perf", "version"])})
    write_json(path / "metadata.json", info)
    execute(command, env, path / "run.log")
    if ": Profiling for " not in (path / "run.log").read_text():
        raise RuntimeError("No profiling workload matched; check the benchmark filter")
    profile_report(path)
    print(f"CPU sample profile: {path}")


def profile_report(path):
    with (path / "report.txt").open("w") as output:
        subprocess.run(["perf", "report", "--stdio", "--no-children", "--call-graph", "none", "--sort", "symbol", "--percent-limit", "0.5", "-i", str(path / "perf.data")], stdout=output, check=True)
    demangle(path / "report.txt")
    collapse, flamegraph = [shutil.which(name) for name in ("inferno-collapse-perf", "inferno-flamegraph")]
    if collapse and flamegraph:
        with (path / "stacks.perf").open("w") as output:
            # Inline expansion can launch a slow external addr2line per symbol.
            # Keep the captured physical stacks; interactive perf can expand them.
            subprocess.run(["perf", "script", "--no-inline", "-i", str(path / "perf.data")], stdout=output, check=True)
        demangle(path / "stacks.perf")
        with (path / "stacks.perf").open() as source, (path / "stacks.folded").open("w") as output:
            subprocess.run([collapse], stdin=source, stdout=output, check=True)
        with (path / "stacks.folded").open() as source, (path / "flamegraph.svg").open("w") as output:
            title = json.loads((path / "metadata.json").read_text())["filter"]
            subprocess.run([flamegraph, "--title", title], stdin=source, stdout=output, check=True)


def demangle(path):
    # Some distro perf builds leave Rust v0 names mangled. Binutils can resolve
    # those names before Inferno consumes the stack stream.
    executable = shutil.which("c++filt")
    if executable:
        temporary = path.with_name(path.name + ".tmp")
        with path.open() as source, temporary.open("w") as output:
            subprocess.run([executable, "--format=rust"], stdin=source, stdout=output, check=True)
        temporary.replace(path)


def report(args):
    profile_report(Path(args.path).resolve())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("doctor")
    command = commands.add_parser("snapshot")
    command.add_argument("--bench", choices=("ui_cpu", "invalidation", "inspector_render"), default="ui_cpu")
    command.add_argument("--label", required=True)
    command.set_defaults(filter="")
    for name in ("run", "profile"):
        command = commands.add_parser(name)
        command.add_argument("--bench", choices=("ui_cpu", "invalidation", "inspector_render"), default="ui_cpu")
        command.add_argument("--label", required=True)
        command.add_argument("--variant", help="Run a frozen snapshot without rebuilding")
        command.add_argument("--filter", default="" if name == "run" else "ui_cpu/grid/10000")
        if name == "run":
            command.add_argument("--frames", action="store_true")
            command.add_argument("--warmup", type=float, default=1)
            command.add_argument("--measurement", type=float, default=3)
            command.add_argument("--samples", type=int, default=30)
        else:
            command.add_argument("--seconds", type=int, default=10)
    command = commands.add_parser("compare")
    command.add_argument("baseline")
    command.add_argument("candidate")
    command = commands.add_parser("frames")
    command.add_argument("path")
    command.add_argument("--min-elements", type=int, default=0, help="Skip empty/small fixture frames")
    command = commands.add_parser("report")
    command.add_argument("path")
    args = parser.parse_args()
    if args.command == "run" and (args.samples < 10 or args.warmup <= 0 or args.measurement <= 0):
        parser.error("samples must be at least 10; warmup and measurement must be positive")
    if args.command == "profile" and (args.seconds < 1 or not args.filter):
        parser.error("profiling requires a filter and at least one second")
    try:
        if args.command == "doctor":
            print(json.dumps(doctor(), indent=2))
        else:
            globals()[args.command](args)
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
