"""Reproduce sequential XPS live captures after building the native benchmark."""

import argparse
import datetime
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=Path)
parser.add_argument("--profile", action="store_true")
parser.add_argument("--reverse", action="store_true")
parser.add_argument("--sample-frequency", action="store_true")
args = parser.parse_args()
binary = args.binary.resolve()
root = Path(__file__).resolve().parents[3]
os.chdir(root)
out = Path(__file__).resolve().parent / ("gpu-reverse" if args.reverse else "gpu" if args.profile else "live")
out.mkdir(exist_ok=False)
modes = ["baseline", "replay"] * 5 + ["static"]
if args.profile:
    modes = ["baseline", "replay", "static"]
if args.reverse:
    assert args.profile
    modes = ["replay", "baseline"]
manifest = {
    "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
    "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    "gpu_timing_documentation": "https://docs.mesa3d.org/u_trace.html",
    "runs": [],
}
for index, mode in enumerate(modes):
    name = f"{index + 1:02d}-{mode}"
    env = os.environ.copy()
    env["ALA_FRAME_STATS"] = str(out / f"{name}.ndjson")
    if args.profile:
        env.update(MESA_GPU_TRACES="print_json", MESA_GPU_TRACEFILE=str(out / f"{name}-gpu.json"))
    command = ["/usr/bin/time", "-v", "-o", str(out / f"{name}.time.txt"),
               "python3", "experiments/paint-replay/run.py", "--binary", str(binary),
               "--live", mode, "--seconds", "60"]
    run = {
        "name": name,
        "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "command": command,
        "environment": {key: env[key] for key in ("ALA_FRAME_STATS", "MESA_GPU_TRACES", "MESA_GPU_TRACEFILE") if key in env},
        "power_mode": subprocess.check_output(["powerprofilesctl", "get"], text=True).strip(),
        "thermal_millicelsius": {str(path): int(path.read_text()) for path in Path("/sys/class/thermal").glob("thermal_zone*/temp")},
    }
    print(f"Starting {name} ({'GPU timestamps' if args.profile else 'CPU frame statistics'})", flush=True)
    with (out / f"{name}.log").open("w") as log:
        if args.sample_frequency:
            child = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT)
            frequencies = []
            paths = {"requested_mhz": "/sys/class/drm/card1/gt_cur_freq_mhz", "actual_mhz": "/sys/class/drm/card1/gt_act_freq_mhz", "punit_requested_mhz": "/sys/class/drm/card1/gt/gt0/punit_req_freq_mhz", "throttle_status": "/sys/class/drm/card1/gt/gt0/throttle_reason_status"}
            while child.poll() is None:
                sample = {"monotonic_ns": time.monotonic_ns(), "unix_ns": time.time_ns()}
                for key, path in paths.items():
                    try:
                        sample[key] = int(Path(path).read_text())
                    except OSError as error:
                        sample[key] = str(error)
                frequencies.append(sample)
                time.sleep(.047)
            run["returncode"] = child.wait()
            (out / f"{name}.frequency.json").write_text(json.dumps(frequencies) + "\n")
            run["frequency_sample_interval_seconds"] = .047
        else:
            run["returncode"] = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
    for trace in [out / f"{name}.ndjson", *out.glob(f"{name}-gpu*.json"), *out.glob(f"{name}.frequency.json")]:
        if trace.exists():
            with gzip.open(str(trace) + ".gz", "wb") as compressed:
                compressed.write(trace.read_bytes())
    manifest["runs"].append(run)
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    if run["returncode"]:
        raise SystemExit(f"{name} failed; inspect its log")
