"""Collect sequential process RSS controls, separate from primary CPU timings."""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=Path)
args = parser.parse_args()
binary = args.binary.resolve()
os.sched_setaffinity(0, {2})
root = Path(__file__).resolve().parent
out = root / "memory"
out.mkdir(exist_ok=False)
result = {"source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
          "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "cpu_affinity": [2], "runs": []}
cases = [(workload, mode, 120) for workload in ("curves", "dashed", "triangle", "changing", "cold") for mode in ("baseline", "cached")]
cases.append(("changing", "cached", 1200))
for workload, mode, samples in cases:
    name = f"{workload}-{mode}-{samples}"
    command = ["/usr/bin/time", "-v", "-o", str(out / f"{name}.time.txt"), str(binary),
               "--workload", workload, "--mode", mode, "--paths", "128", "--samples", str(samples)]
    with (out / f"{name}.json").open("w") as log:
        subprocess.run(command, stdout=log, check=True)
    value = json.loads((out / f"{name}.json").read_text())
    expected = samples * 128
    assert value["retained_payload_bytes"] <= 8 * 1024 * 1024
    assert value["hits"] == (expected if mode == "cached" and workload not in ("cold", "changing") else 0)
    assert value["misses"] == (expected if mode == "cached" and workload in ("cold", "changing") else 0)
    time_log = (out / f"{name}.time.txt").read_text()
    result["runs"].append({"name": name, "command": command, "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "peak_rss_kib": int(re.search(r"Maximum resident set size \(kbytes\): (\d+)", time_log)[1]),
        **{key: value[key] for key in ("hits", "misses", "retained_payload_bytes")}})
    (root / "memory.summary.json").write_text(json.dumps(result, indent=2) + "\n")
    print(name, result["runs"][-1]["peak_rss_kib"], "KiB", flush=True)
