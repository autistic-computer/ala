# Apple Silicon validation, 2026-10-29

Tested source: `b083be572dcd2495dc0856ca00d31399d2746ef0`. The evidence commit only adds this report. Both published-source CI jobs passed; see `tested-source-ci.json`.

MacBook Pro Mac16,5, Apple M4 Max (12 performance + 4 efficiency CPU cores, 40 GPU cores), 48 GiB RAM; macOS 27.2 (26B123), Xcode 27.0, Rust 1.99.0, Python 3.14.10. AC power, automatic power mode, no initial thermal/performance warning. Display: 1920 x 1080 at 60 Hz, scale 1 in the native fixture. This is a shared workstation with background services active; no task-owned compilation or profiling overlapped these timing passes. CPU affinity was not pinned on macOS.

Five alternating process repetitions, 120 samples per repetition and workload, with allocation counting in separate processes. Table values are medians of process medians. Raw samples, all controls, source/compiler/binary hashes and verification are in `cpu.json.gz`; `cpu.summary.json` is readable without decompression. These are local baseline/candidate comparisons, not comparisons against Linux wall times.

**Verdict: exact meshes and Metal pixels pass; warm complex paths improve while misses remain slower. Keep opt-in and draft.**

All 1,536 exact complete-mesh comparisons and 5,760,000 pixels across twelve rendered images/three distinct scenes pass using the macOS Metal headless renderer. All three targeted cache tests pass. No implementation fix was needed.

| Workload, 128 paths/sample | Baseline ms | Cached ms | Ratio | Cached time change |
| --- | ---: | ---: | ---: | ---: |
| curves | 2.4983 | 0.2142 | 11.67x | -91.4% |
| dashed | 1.8803 | 0.2085 | 9.02x | -88.9% |
| triangle | 0.0714 | 0.0698 | 1.02x | -2.2% |
| changing | 2.4877 | 2.6975 | 0.92x | +8.4% |
| cold | 2.5002 | 2.6895 | 0.93x | +7.6% |

Every warm cached curves/dashed/triangle process has 15,360 hits and zero misses. Changing/cold processes have zero hits and 15,360 misses. Stored payload stays within 8 MiB; this is not a process-RSS cap. Timing includes construction, keys, tessellation or mesh copy, and drop, with no GPU submission. Pixel equality does not establish GPU speedups. Actual caller adoption/hit rates, application draw-phase performance and native Windows/MSVC execution remain outstanding.
