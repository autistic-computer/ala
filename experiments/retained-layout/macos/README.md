# Apple Silicon validation, 2026-10-29

Tested source: `19c3b3f6695212f95a9a38a67dc3e3707bd68461`. The evidence commit only adds this report. Both published-source CI jobs passed; see `tested-source-ci.json`.

MacBook Pro Mac16,5, Apple M4 Max (12 performance + 4 efficiency CPU cores, 40 GPU cores), 48 GiB RAM; macOS 27.2 (26B123), Xcode 27.0, Rust 1.99.0, Python 3.14.10. AC power, automatic power mode, no initial thermal/performance warning. Display: 1920 x 1080 at 60 Hz, scale 1 in the native fixture. This is a shared workstation with background services active; no task-owned compilation or profiling overlapped these timing passes. CPU affinity was not pinned on macOS.

Five alternating process repetitions, 120 samples per repetition and workload, with allocation counting in separate processes. Table values are medians of process medians. Raw samples, all controls, source/compiler/binary hashes and verification are in `cpu.json.gz`; `cpu.summary.json` is readable without decompression. These are local baseline/candidate comparisons, not comparisons against Linux wall times.

**Verdict: the Taffy kernel exactness and sparse-update CPU result reproduce on Apple Silicon. Keep draft pending ALA integration work.**

All 499,328 exact relative node rectangles across four cases and 128 frames match. Aggregate geometry checksums also match across all three modes in every one of the 72 timing/allocation processes. The checksum check supplements the stronger per-node verifier.

| Dirty leaves | Rebuild ms | Retained ms | Ratio | Rewrite all ms | Ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| 0 | 1.4830 | 0.0745 | 19.91x | 1.3921 | 1.07x |
| 1 | 1.4921 | 0.1321 | 11.30x | 1.4311 | 1.04x |
| 64 | 1.5656 | 0.1375 | 11.39x | 1.4405 | 1.09x |
| 3840 | 1.5379 | 1.4194 | 1.08x | 1.4134 | 1.09x |

Stable identities come from the fixture. Initial creation, initial compute and 30 warmup frames are excluded. Full geometry extraction/checksum is outside timing. No ALA reconciliation, text, renderer or GPU work is exercised. All-dirty and rewrite-all remain modest gains. Production stable-ID reconciliation, invalidation, lifetime, removal/reordering, measurement closures and focus/input behavior remain unvalidated. Windows/MSVC execution remains outstanding.
