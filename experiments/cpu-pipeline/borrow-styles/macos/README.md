# Apple Silicon style-borrowing validation, 2026-10-29

M4 Max MacBook Pro, 48 GiB RAM, macOS 27.2 (26B123), Rust 1.99.0, Xcode 27.0, Python 3.14.10. AC/automatic power, no initial thermal warning. Shared workstation with background services active; no task-owned builds, frame recordings or profiles overlapped these timings. No hard CPU affinity. Native CoreText; no renderer/GPU submission in `ui_cpu`.

Both executables were built and frozen before timing, with the locked dependencies, the `perf` profile and `-C force-frame-pointers=yes`. Three pairs ran A1/B1, B2/A2, A3/B3, using two seconds of warmup, ten seconds requested measurement and 50 Criterion samples for every case. Criterion can extend collection to meet the sample count. All seven workloads keep their render-count/no-idle-redraw assertions. These are synthetic CPU pipeline results, not application FPS, physical display pacing, GPU completion, input latency or power measurements.

**Verdict: small-grid gains reproduce, while large-grid and text results are mixed.** The 100/1,000-node grids improve about 4% in every pair. The 10,000-node changes are +3.24%, -4.97% and +3.36% reductions, so its 0.55% aggregate reduction does not establish a repeatable gain. Warm 50-row text averages 2.46% slower; changing text is roughly equal.

Frozen sources: baseline `307bee633693f13a67c3afe98c0ac098e978e8f4`, borrowed `b9607d8057dacf1a11b2842ddb5096146411e686`. Later commits fix a tooling assertion and add evidence; they do not change the measured Rust code. [Baseline metadata and raw A runs](../../macos/README.md) are inherited from the toolkit branch.

All 594 core tests, four workspace integration tests, 66 macOS platform tests and five iOS input tests pass on merged source `17361dded4d501b0a9c3dc40a2ba5bd147c673b0`; four platform tests are already ignored by the repository. Formatting and all four corrected Python tooling tests pass. Native platform tests include Metal renderer/atlas and CoreText cases; this does not establish complete application pixel equivalence.

Means averaged across three process runs; positive reduction is faster:

| Case | Baseline ms | Borrowed ms | Mean reduction | Pair 1 / 2 / 3 reductions |
| --- | ---: | ---: | ---: | --- |
| ui_cpu/grid/100 | 0.093930 | 0.090192 | +3.98% | +4.35% / +3.45% / +4.13% |
| ui_cpu/grid/1000 | 0.963032 | 0.924543 | +4.00% | +4.34% / +3.72% / +3.92% |
| ui_cpu/grid/10000 | 10.061517 | 10.005791 | +0.55% | +3.24% / -4.97% / +3.36% |
| ui_cpu/idle_no_redraw | 0.000051 | 0.000050 | +2.99% | +4.14% / +3.86% / +0.94% |
| ui_cpu/text_churn/50 | 1.026696 | 1.025148 | +0.15% | +0.96% / -0.59% / +0.07% |
| ui_cpu/text_warm/50 | 0.323985 | 0.331951 | -2.46% | +2.21% / -0.91% / -8.73% |
| ui_cpu/text_warm/500 | 1.204251 | 1.186719 | +1.46% | +2.79% / +0.91% / +0.65% |

The idle control changes from 51.325 to 49.789 ns, a -1.536 ns difference. Per-run confidence intervals and raw samples remain in the compressed run records; they do not replace repeated-process controls. No samples or regressions were dropped.

Separate short instrumented 10,000-node runs check phase counters outside timing. Both variants record exactly one view, 10,102 elements, 10,101 layout nodes and 10,000 quads per populated frame. See the two `*-phases.summary.json` files. They are diagnostic and include warmup/calibration, not another performance comparison.

Keep draft. Repeated native CPU results do not validate production application behavior, hardware GPU completion, physical display pacing, input latency or power. Native Windows/DirectWrite and Linux desktop comparisons remain pending.
