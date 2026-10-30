import contextlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import perf


class PerfTests(unittest.TestCase):
    def test_frame_summary_preserves_cpu_clock_and_missing_phases(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "frames.jsonl"
            records = [
                {"draw": {"wall_ns": 2_000_000, "thread_cpu_ns": 1_000_000},
                 "phases": {"paint": {"wall_ns": 1_000_000, "thread_cpu_ns": None}, "present_submit": None},
                 "counts": {"elements": 10}},
                {"draw": {"wall_ns": 4_000_000, "thread_cpu_ns": 3_000_000},
                 "phases": {"paint": {"wall_ns": 3_000_000, "thread_cpu_ns": None}, "present_submit": None},
                 "counts": {"elements": 20}},
            ]
            path.write_text("\n".join(json.dumps(record) for record in records))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                perf.frames(SimpleNamespace(path=path, min_elements=0))
            result = json.loads(output.getvalue())
            self.assertEqual(result["frames"], 2)
            self.assertEqual(result["mean_counts_per_frame"]["elements"], 15)
            self.assertEqual(result["timings_ms"]["draw.thread_cpu_ns"]["p50"], 2)
            self.assertEqual(result["timings_ms"]["paint.wall_ns"]["mean"], 2)
            self.assertNotIn("paint.thread_cpu_ns", result["timings_ms"])
            self.assertNotIn("present_submit.wall_ns", result["timings_ms"])
            filtered = io.StringIO()
            with contextlib.redirect_stdout(filtered):
                perf.frames(SimpleNamespace(path=path, min_elements=15))
            selected = json.loads(filtered.getvalue())
            self.assertEqual(selected["frames"], 1)
            self.assertEqual(selected["skipped_frames"], 1)
            self.assertEqual(selected["mean_counts_per_frame"]["elements"], 20)

    def test_result_discovery_uses_only_current_measurements(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            estimate = {"point_estimate": 2000, "confidence_interval": {"lower_bound": 1900, "upper_bound": 2100}}
            for kind in ("new", "base"):
                folder = root / "criterion/grid/1000" / kind
                folder.mkdir(parents=True)
                (folder / "benchmark.json").write_text(json.dumps({"full_id": "ui_cpu/grid/1000"}))
                (folder / "estimates.json").write_text(json.dumps({"mean": estimate, "median": estimate}))
            result = perf.estimates(root)
            self.assertEqual(list(result), ["ui_cpu/grid/1000"])
            self.assertEqual(result["ui_cpu/grid/1000"]["mean_ns"], 2000)

    def test_comparison_rejects_diagnostic_and_timing_runs(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / name for name in ("baseline", "candidate")]
            for path, frames in zip(paths, (False, True)):
                path.mkdir()
                (path / "metadata.json").write_text(json.dumps({"frames": frames}))
            with self.assertRaisesRegex(RuntimeError, "Incompatible runs: frames"):
                perf.compare(SimpleNamespace(baseline=paths[0], candidate=paths[1]))

    def test_frozen_variant_preserves_source_identity_and_rejects_changed_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            variant = Path(directory)
            saved = {"bench": "ui_cpu", "binary_sha256": "original-binary",
                     "commit": "original-commit", "worktree_sha256": "original-source",
                     "dirty": True, "rust": "rust-version", "features": "features", "rustflags": "flags"}
            (variant / "variant.json").write_text(json.dumps(saved))
            current = {**saved, "worktree_sha256": "changed-source", "machine": "measurement-host"}
            args = SimpleNamespace(variant=variant, bench="ui_cpu")
            with patch.object(perf, "metadata", return_value=current.copy()):
                binary, info = perf.prepare(args)
            executable = "benchmark.exe" if perf.sys.platform == "win32" else "benchmark"
            self.assertEqual(binary, str(variant.resolve() / executable))
            self.assertEqual(info["worktree_sha256"], "original-source")
            self.assertEqual(info["machine"], "measurement-host")
            with patch.object(perf, "metadata", return_value={**current, "binary_sha256": "changed-binary"}):
                with self.assertRaisesRegex(RuntimeError, "hash does not match"):
                    perf.prepare(args)


if __name__ == "__main__":
    unittest.main()
