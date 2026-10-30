# Apple Silicon opacity replay validation, 2026-10-29

Tested source `de64d16db673390499e34eaef4699e15bcdf0915`, clean tracked source, Rust 1.99.0. Evidence commits do not change the measured implementation. M4 Max MacBook Pro (12 performance + 4 efficiency CPU cores, 40 GPU cores), 48 GiB RAM; macOS 27.2, Xcode 27.0, Python 3.14.10; AC/automatic power. Shared workstation with background services active, no task-owned build/profile overlap during timing, no hard CPU affinity. Native window: 800 x 600 logical pixels, scale 1, virtual 1920 x 1080 display configured at 60 Hz.

**Verdict: native CoreText/Metal exactness, cached input and CPU gains pass. Keep draft for production API/invalidation review and remaining platform/application gates.**

All 24 scene/hitbox comparisons, 46,080,000 pixels over 24 images (17 distinct), two cached click checks and 593 release core tests pass. Formatting passes. These automated checks do not constitute broad native typography or manual interaction coverage. The desktop UI tool could not select the unbundled fixture executable, so manual visual/resize/scroll review was not completed.

Five alternating repetitions x 120 samples, separate allocation passes; medians of process medians:

| Rows | Baseline CPU ms | Replay CPU ms | Ratio |
| --- | ---: | ---: | ---: |
| 30 | 0.1156 | 0.0202 | 5.71x |
| 120 | 0.3155 | 0.0317 | 9.95x |
| 480 | 1.1347 | 0.0784 | 14.48x |

Five native one-minute baseline/replay pairs ran in alternating order, followed by a full one-minute static process. First second excluded from animated statistics:

| Native draw statistic | Baseline | Replay |
| --- | ---: | ---: |
| CPU draw median ms | 5.9871 | 0.7258 |
| CPU draw p95 ms | 7.3341 | 0.9260 |
| Thread CPU draw median ms | 5.8618 | 0.7083 |
| Draw-event rate Hz | 23.26 | 23.18 |

Median CPU draw improves 8.25x; paired ratios range 7.79-8.91x. Steady layout nodes fall 967 to 4 and views 2 to 1, with 480 hitboxes preserved. The static process records one startup draw and none for the rest of its 60-second lifetime. Raw per-frame streams and process durations are retained.

The observed roughly 23 Hz draw-event rate is below the configured 60 Hz display refresh. Window activation was not independently verified. These traces do not establish physical 60 Hz pacing, compositor presentation, scanout or input latency.

Separate short Metal System Trace recordings, after all CPU comparisons/builds, identify target-process GPU execution on M4 Max. Only target-PID intervals are published; full system traces remain local because they contain other processes. GPU encoder envelopes exclude the first second of target GPU work and group vertex/fragment intervals by application command-buffer ID:

| Mode | Command buffers | GPU envelope median ms | p95 ms |
| --- | ---: | ---: | ---: |
| baseline | 207 | 0.114708 | 0.122833 |
| replay | 213 | 0.114917 | 0.122792 |

The envelope uses earliest GPU-channel start to latest channel end per command buffer; it does not sum overlapping stages or substitute submission/queue latency for execution. These short captures under automatic clocks are diagnostic; no general GPU speedup or power saving is established. The fixture logs `gpu: null` because macOS `MacWindow::gpu_specs()` returns `None`; the Metal trace independently identifies M4 Max hardware execution.

`cpu.json.gz`, `cpu.summary.json`, `live.summary.json`, the compressed frame streams and target-only Metal samples preserve source/compiler/binary hashes, commands and complete measurements. Full `.trace` bundles are excluded. API/lifetime/invalidation review, representative production callers, broader typography/interaction, physical display/input/power and Windows/MSVC remain pending.
