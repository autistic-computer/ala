# Borrow styles during layout construction

The retained change reduces style copying during layout construction. Across
three confirmation pairs, CPU time fell by 5.0% for the 100-node grid, 4.6% for
1,000 nodes, and 6.4% for 10,000 nodes. Every grid case was faster in all three
pairs. Rendered-work counters stayed identical, and correctness checks passed.

These are measured gains in synthetic CPU workloads on this VM. They do not
establish application-wide or hardware GPU improvements.

## Change

`Div` now passes a borrowed `Style` into an internal window layout helper.
The layout engine also borrows the style while converting its layout
properties for Taffy. This avoids moving the full paint/text style through those
internal calls and through the child-layout closure.

The public `Window::request_layout` and `request_measured_layout` signatures
remain available. Layout rules, invalidation behavior, and the work performed
by the benchmark are preserved. The production diff is limited to:

- [Div layout requests](../crates/ala/src/elements/div.rs).
- [Window layout construction](../crates/ala/src/window.rs).
- [Taffy style conversion boundary](../crates/ala/src/taffy.rs).

Two preceding inlining experiments produced mixed or inconclusive results and
were removed. Their screening data is committed in
[`borrow-styles/rejected`](../experiments/cpu-pipeline/borrow-styles/rejected).
The retained change contains
no macro or inlining changes.

## Confirmation protocol

The original and candidate executables were frozen before confirmation using
`tools/perf.py snapshot`. Every run verified the binary hash and retained its
original source/build metadata. The confirmation order was **A1, B1, B2, A2,
A3, B3**, giving paired orders A/B, B/A, A/B.

All runs used the same Linux EPYC VM, CPU affinity pinned to CPU 0, the same
Rust 1.99.0 toolchain, optimized `perf` profile, frame pointers, and benchmark
features. Each case used one second of warmup, three seconds of measurement,
and 30 Criterion samples. Frame JSON recording was disabled, and compilation
and profiling did not overlap timing runs.

The table averages each case's mean over its three runs. Paired ranges show all
three candidate-versus-original changes; negative means less time.

| Case | Original mean | Candidate mean | Change | Three paired changes |
| --- | --- | --- | --- | --- |
| Grid, 100 nodes | 0.1740 ms | 0.1653 ms | **-5.02%** | -2.73%, -7.38%, -4.84% |
| Grid, 1,000 nodes | 1.8236 ms | 1.7395 ms | **-4.61%** | -5.80%, -3.07%, -4.91% |
| Grid, 10,000 nodes | 26.7257 ms | 25.0169 ms | **-6.39%** | -9.23%, -6.87%, -3.03% |
| Warm text, 50 rows | 0.5358 ms | 0.5238 ms | -2.24% | -1.84%, -1.06%, -3.74% |
| Warm text, 500 rows | 2.4134 ms | 2.3638 ms | -2.05% | -2.21%, -2.35%, -1.59% |
| Changing text, 50 rows | 1.7041 ms | 1.6894 ms | -0.86% | +1.48%, -2.86%, -1.08% |
| Synthetic idle turn | 92.37 ns | 95.07 ns | +2.92% | +1.95%, +0.52%, +6.32% |

The grid improvements were consistent across run orders. Warm-text changes
were smaller. Changing-text results were mixed and do not establish a gain.
The idle control showed an observed regression of about 2.7 ns per synthetic
turn; its cause was not isolated. Its no-redraw assertion still passed.

This experiment has three independent run pairs, not a dedicated hardware
runner or a guarantee about every UI. The earlier same-binary checks showed
several percent of VM variation. The consistent paired grid changes, supported
by the copying profile below, justify keeping this small change, while broader
performance claims need target-machine testing.

The 10,000-node case still takes about **25 ms**, above a 16.7 ms frame budget
even before GPU work. This is a modest improvement rather than a solution to
the cost of rebuilding that entire grid.

## Profile and equivalent work

A separate ten-second candidate capture recorded 4,813 CPU-clock samples with
zero lost samples. The original profile's binary hash matches the frozen
original executable. Memory copying fell from about **17.9% to 14.8% of sampled
CPU time**. The former copy hotspot under the div layout-request closure no
longer appeared among the leading copy callers. These sampling shares support
the hypothesis; they are not counts of copied bytes or an isolated instruction
measurement.

A separate frame-stat recording contained 249 populated candidate frames. Each
had one rendered view, 10,102 elements, 10,101 layout nodes, and 10,000 quads,
matching the original recording exactly. No rendering work was removed to
obtain the timing result. Instrumented timings were excluded from confirmation.

## Validation

- 605 core tests passed, including layout, style, text, and interaction tests.
- The headless rendered-pixel test and four renderer tests passed on llvmpipe.
- All seven benchmark cases retained their render-count/idle assertions.
- Four Python tool tests passed, including frozen-build provenance/hash checks.
- Clippy with warnings denied and formatting checks passed.

No real GPU was exposed to this VM. The rendering checks validate pixels on
software Vulkan; GPU execution, display pacing, power, and real application
frame rates remain unmeasured.

## Pending before merge

Follow [real-machine validation](performance-validation.md) on Linux, Apple
Silicon, and Windows. The published branch still needs CI, native application
timings with production features, native renderer checks, and investigation of
the small synthetic idle regression. Windows headless text uses a no-op backend;
native DirectWrite text performance remains unvalidated.

The parallel fork survey covers opacity replay, segmented shaping, and retained
layout experiments. This change reduces copies in ALA's existing rebuild path;
it does not include those prototypes or claim their speedups. Recheck the
combined behavior if retained layout later replaces that path.

## Saved evidence

- [Aggregate and all per-run means](../experiments/cpu-pipeline/borrow-styles/summary.json).
- [Exact historical confirmation commands](../experiments/cpu-pipeline/borrow-styles/commands.sh)
  (use fresh labels for another run).
- [Pair 1](../experiments/cpu-pipeline/borrow-styles/pair-1.txt),
  [pair 2](../experiments/cpu-pipeline/borrow-styles/pair-2.txt), and
  [pair 3](../experiments/cpu-pipeline/borrow-styles/pair-3.txt).
- [Original build metadata](../experiments/cpu-pipeline/borrow-styles/style-original.json)
  and [candidate build metadata](../experiments/cpu-pipeline/borrow-styles/style-borrowed.json).
- [Candidate profile](../experiments/cpu-pipeline/borrow-styles/profiles/borrow-profile.json)
  and [candidate work counts](../experiments/cpu-pipeline/borrow-styles/phases.json).

Raw Criterion samples, confidence intervals, commands, and measurement metadata
are committed in `experiments/cpu-pipeline/borrow-styles/runs/borrow-a1.json`
through `borrow-b3.json`. Large traces and flame graphs remain local and can be
regenerated with the toolkit; they are not required to read the committed data.
The general [performance guide](performance.md) documents snapshots, comparisons,
phase recording, and profiling.
