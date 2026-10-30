# Apple Silicon toolkit validation, 2026-10-29

M4 Max MacBook Pro, 48 GiB RAM, macOS 27.2 (26B123), Rust 1.99.0, Xcode 27.0, Python 3.14.10. AC/automatic power, no initial thermal warning. Shared workstation with background services active; no task-owned builds, frame recordings or profiles overlapped these timings. No hard CPU affinity. Native CoreText; no renderer/GPU submission in `ui_cpu`.

Both executables were built and frozen before timing, with the locked dependencies, the `perf` profile and `-C force-frame-pointers=yes`. Three pairs ran A1/B1, B2/A2, A3/B3, using two seconds of warmup, ten seconds requested measurement and 50 Criterion samples for every case. Criterion can extend collection to meet the sample count. All seven workloads keep their render-count/no-idle-redraw assertions. These are synthetic CPU pipeline results, not application FPS, physical display pacing, GPU completion, input latency or power measurements.

Frozen baseline source: `307bee633693f13a67c3afe98c0ac098e978e8f4`. The executable hash and clean source fingerprint are in `variant.json`. The later test/report commits do not change benchmark code.

**Verdict: all seven native CoreText CPU workloads and frozen-run comparison tooling execute successfully. Native Mac testing exposed and fixed a path-resolution assumption that Linux CI did not expose.**

The frozen-path test compared the unresolved macOS `/var/...` temporary path against the resolved path returned by production code (`/private/var/...`). Resolving the expected path fixes the assertion without changing the runner or benchmarks. All four tooling tests pass after commit `172ad2d`; the original failure and corrected output are preserved here. Linux-only CI had run the tooling tests; the original macOS CI covered Rust.

Repeated baseline means:

| Case | A1 us | A2 us | A3 us | Range / mean |
| --- | ---: | ---: | ---: | ---: |
| ui_cpu/grid/100 | 94.6345 | 93.2964 | 93.8579 | 1.42% |
| ui_cpu/grid/1000 | 969.5487 | 958.4711 | 961.0765 | 1.15% |
| ui_cpu/grid/10000 | 10147.5516 | 10020.2293 | 10016.7710 | 1.30% |
| ui_cpu/idle_no_redraw | 0.0517 | 0.0518 | 0.0505 | 2.59% |
| ui_cpu/text_churn/50 | 1032.4079 | 1018.9557 | 1028.7250 | 1.31% |
| ui_cpu/text_warm/50 | 327.2793 | 321.7917 | 322.8840 | 1.69% |
| ui_cpu/text_warm/500 | 1214.4766 | 1199.3329 | 1198.9436 | 1.29% |

The range reflects variation between process runs, not a confidence interval. Each compressed run preserves metadata, complete Criterion samples/estimates/benchmark/Tukey records and console output. The paired style-borrowing result lives in [the stacked optimization report](https://github.com/autistic-computer/ala/blob/codex/ala-borrow-layout-styles/experiments/cpu-pipeline/borrow-styles/macos/README.md). Representative application workloads, native Windows/DirectWrite, Linux desktop repetitions, physical display/GPU latency and power remain separate gates.
