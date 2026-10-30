# Performance tooling assessment — 2026-10-29

The repository now has a CPU benchmark suite, an optimized profiling build, and
a runner for saved baselines, comparisons, phase statistics, and Linux sampling.
The usage guide is [performance.md](performance.md). This report establishes the
initial baseline; [committed evidence](../experiments/cpu-pipeline/README.md)
contains the raw Criterion samples, metadata, profiles, and validation logs.
The follow-up [style-borrowing experiment](performance-experiment.md) records
the first retained optimization and its three paired comparisons.

## Host and available measurements

The current host is an x86-64 Linux VM on an AMD EPYC 9V74. It exposes five
logical CPUs with a cgroup quota equivalent to four CPUs. Rust 1.99.0, Linux
perf 6.12.107, Valgrind 3.24.0, Inferno 0.12.8, and native build dependencies
were installed into the user's directories without system package privileges.
That environment was local to the cloud session. Fresh checkouts should follow
[performance.md](performance.md) and [native validation](performance-validation.md).

| Measurement | Current capability |
| --- | --- |
| Optimized CPU benchmarks | Available; grids, warm/changing text, and idle work |
| CPU stack sampling | Available through `cpu-clock:u`; DWARF stack capture |
| Per-frame phases and thread CPU time | Available through `ALA_FRAME_STATS` |
| Rendered-pixel validation | Software Vulkan available through llvmpipe |
| Instruction counting | Callgrind executed the actual 100-node ala CPU workload successfully |
| Hardware cycles/cache-miss counters | Unavailable; hardware event probe returns `ENOENT` |
| Hardware GPU measurements | Unavailable; no exposed render device |
| Display pacing/input-to-photon/power | Requires a target machine and additional instrumentation |

`vulkaninfo` identifies `llvmpipe (LLVM 19.1.7, 256 bits)`, Mesa
25.0.7, device type CPU. The local environment script selects that driver
explicitly and limits its workers to four. Its rendering times cannot establish
hardware GPU gains. Software rendering, CPU-only UI work, and target-GPU results
must be treated as separate measurements.

## Validation and results

The optimized `ui_cpu` benchmark built successfully with release optimization,
debug symbols, and frame pointers. All seven cases passed a one-iteration smoke
run, including the assertion that each update renders once and idle work does
not rerender.

The software Vulkan renderer passed four unit tests covering colored pixels,
capture sizes, target reuse, and invalid sizes. The headless application
screenshot test also passed (one separate manual glyph benchmark remains
ignored). The new benchmark passed Clippy with warnings denied, formatting
passed, and the four Python tool tests cover result parsing, frame filtering,
rejection of incompatible comparisons, and frozen-build identity checks.

### CPU timing and repeatability

Both runs used the same binary and source fingerprint, affinity pinned to CPU
0, one second of warmup, three seconds of measurement, and 30 Criterion samples.
Frame JSON recording was disabled. These are synthetic CPU UI workloads with
`bench-support`/ala's built-in profiler enabled, without a GPU renderer.

| Case | Baseline mean | Repeat mean |
| --- | --- | --- |
| 100-node grid | 0.169 ms | 0.171 ms |
| 1,000-node grid | 1.777 ms | 1.753 ms |
| 10,000-node grid | 25.710 ms | 26.535 ms |
| 50 warm text rows | 0.528 ms | 0.528 ms |
| 500 warm text rows | 2.372 ms | 2.386 ms |
| 50 changing text rows | 1.652 ms | 1.665 ms |
| Idle executor work, no redraw | 94.6 ns | 92.8 ns |

The largest difference was **3.21% in the same 10,000-node case**, despite
non-overlapping within-run mean confidence intervals. This demonstrates why
those intervals alone do not establish an optimization gain. Other cases varied
by less than 2%. Idle measures a synthetic executor turn, not application idle
power or CPU usage over time.

Raw results, confidence intervals, and metadata are retained in
[`baseline`](../experiments/cpu-pipeline/baseline/runs/baseline.json) and
[`repeat`](../experiments/cpu-pipeline/baseline/runs/repeat.json). Reproduce with:

```sh
taskset -c 0 python3 tools/perf.py run --label baseline-next
taskset -c 0 python3 tools/perf.py run --label repeat-next
python3 tools/perf.py compare target/perf-results/baseline-next target/perf-results/repeat-next
```

### CPU hotspots

Ten-second user-space CPU-clock recordings captured 5,013 samples for the grid
and 5,359 for changing text, with zero lost samples reported. The captures
include process setup, so these approximate self-time shares are investigation
leads. They are not predicted speedups or allocation counts.

For the grid, memory copying accounted for about **17.9%** of sampled CPU,
flexbox layout **8.4%**, style refinement **7.4%**, and bounds-tree insertion
**4.0%**. Memory-copy callers include layout-tree clearing, child-vector
construction, layout requests, and element arena construction. This makes
repeated layout/style construction and copying useful first experiments.

For changing text, memory copying accounted for **8.4%**, bidi character lookup
**4.8%**, script lookup **3.8%**, sprite sorting **3.3%**, and shaping fallback
**3.0%**. Investigate avoidable shaping/copying and scene sorting with realistic
Latin and mixed-script workloads before considering text fast paths.

The [grid profile](../experiments/cpu-pipeline/baseline/profiles/grid-profile.json)
and [text profile](../experiments/cpu-pipeline/baseline/profiles/text-profile.json)
retain all sampled leaf weights, metadata, and capture health. Self-time reports
are committed alongside them. Full captures, folded stacks, and flame graphs
remain local; the profiling commands regenerate them on a fresh checkout.
Rust v0 demangling and slow external inline-symbolizer behavior were handled in
the runner. Matching libc debug symbols were added to the user's perf cache to
resolve memory-copy and allocator routines.

Callgrind also ran `ui_cpu/grid/100` in Criterion test mode successfully. Its
validation log is [committed here](../experiments/cpu-pipeline/baseline/validation/callgrind-smoke.log).
That instruction count includes process/font initialization and is a tool
validation, not an isolated per-frame baseline.

### Frame phases and rendering path

A separate instrumented grid run recorded 219 populated frames, each containing
10,102 elements, 10,101 layout nodes, and 10,000 quads. Empty fixture frames were
excluded with `--min-elements 10000`. Mean thread CPU draw time was **26.25 ms**:
layout request/computation accounted for **14.98 ms**, prepaint **3.91 ms**,
paint **6.14 ms**, and scene finalization **0.088 ms**. Wall time and thread CPU
time were close in this recording. These diagnostic timings include recorder
overhead and Criterion warmup/calibration; compare ordinary timing runs
separately. See the [phase summary](../experiments/cpu-pipeline/baseline/grid-phases.json).

The existing `keyboard_focus_pane` invalidation benchmark also ran through the
headless renderer on software Vulkan, averaging **220.4 ms** per iteration.
The process and llvmpipe workers were pinned to one CPU. This demonstrates that
the rendering benchmark is operational; it does not establish hardware GPU
performance or GPU completion time. Its work counters reported five rendered
views and two rendered terminals per iteration. See the
[software-render result](../experiments/cpu-pipeline/baseline/runs/software-render.json).

## What this enables next

CPU experiments can now be tested against saved, comparable workloads and
inspected through call stacks and per-phase work counts. Relevant candidates
include invalidation, layout construction/computation, text shaping, temporary
allocation, and scene building. A candidate should change one cause at a time,
preserve the workload assertions and rendered pixels, and survive repeated
interleaved A/B runs before a gain is claimed.

The existing headless benchmarks can exercise the rendering path here using
software Vulkan. Final shader, batching, atlas-upload, presentation, and power
claims require a real target GPU. GPU completion timestamp collection and
display/input latency measurement are not implemented by this runner.
These EPYC VM results also do not predict Apple Silicon or other CPU/backend
performance. Validate promising changes in the actual application on its target
hardware; full synthetic grid invalidation can differ substantially from the
application's normal update pattern.
