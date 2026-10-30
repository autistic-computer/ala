# Performance debugging

Use `python3 tools/perf.py doctor` to inspect the host before measuring. The
runner needs Python 3.11+, the pinned Rust toolchain, and ala's native build
dependencies. On Debian/Ubuntu the Linux dependencies are:

```sh
sudo apt-get install libfontconfig-dev libfreetype-dev libwayland-dev \
  libx11-xcb-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev \
  mesa-vulkan-drivers vulkan-tools linux-perf valgrind libc6-dbg binutils fonts-dejavu-core
cargo install inferno --locked
```

The cloud measurements used user-local packages through an uncommitted
`/workspace/.ala-perf/env.sh`. That setup selects software Vulkan. A fresh clone
should install native dependencies and use its own environment; the cloud path
is not needed. See [real-machine validation](performance-validation.md) for
Linux, Apple Silicon, and Windows instructions and remaining gates.

## CPU baselines

```sh
python3 tools/perf.py run --label baseline
python3 tools/perf.py run --label repeat
python3 tools/perf.py compare target/perf-results/baseline target/perf-results/repeat
```

`ui_cpu` runs layout, prepaint, paint, scene construction, and the foreground
executor without a renderer. It exercises grids of 100/1,000/10,000 nodes,
50/500 text rows with warm text, 50 text rows with changing content, and idle
work. Each update must render exactly once; idle work must not rerender.
Only rows within the viewport are painted, so the row-count cases also exercise
layout and clipping. This is a synthetic workload, not a full application replay.
On Windows, `current_platform(true)` supplies `NoopTextSystem`; its headless
text cases do not measure DirectWrite. Limit Windows comparisons to the grid
and idle cases until a native text-backend run has been validated.

The `perf` Cargo profile inherits release optimization and retains symbols.
The runner enables frame pointers for the crate and its dependencies. It builds
with the lockfile and captures the toolchain, commit, worktree fingerprint,
binary hash, CPU affinity/quota, build flags, and selected runtime settings.
Each label creates a new directory and cannot overwrite an existing result.
Criterion JSON, logs, and confidence intervals are retained in that directory.
`bench-support` enables ala's built-in profiler. Keep that feature set identical
when comparing benchmarks, and validate final changes in the application with
its production features too.

Use `--filter '^ui_cpu/grid/10000$'` to select one case. Defaults are one second
of warmup, three seconds of measurement, and 30 samples per case. `--warmup`,
`--measurement`, and `--samples` can increase those durations for final results.

Freeze each build before editing/rebuilding to alternate variants without
compilation between measurements:

```sh
python3 tools/perf.py snapshot --label original-build
# Apply the candidate change, then freeze it too.
python3 tools/perf.py snapshot --label candidate-build
taskset -c 0 python3 tools/perf.py run --variant target/perf-results/original-build --label original-1
taskset -c 0 python3 tools/perf.py run --variant target/perf-results/candidate-build --label candidate-1
```

A snapshot saves the executable and its source/build metadata. `--variant`
verifies its binary hash and preserves that build's source identity while
recording the current measurement host. Repeat in alternating order and use
fresh labels for each run. `profile` also accepts `--variant`.

Before claiming a gain, alternate baseline and candidate builds on the same
otherwise idle host, collect multiple runs, and inspect their variance. Pin
CPU affinity consistently if supported. Compilation and profiling must finish
before timing runs. A VM's quota and host scheduling make small differences
unreliable. The comparison command rejects mismatched machine/build/workload
settings; overlapping mean confidence intervals are called out. These intervals
describe a single run and do not account for variation between runs.

## Frame and phase diagnostics

```sh
python3 tools/perf.py run --label grid-phases --frames --filter '^ui_cpu/grid/10000$'
python3 tools/perf.py frames target/perf-results/grid-phases/frames.jsonl --min-elements 10000
```

`ALA_FRAME_STATS` records layout, prepaint, paint, scene finalization, draw, and
submission wall time and thread CPU time where available, with element, glyph,
layout-node, and primitive counts. The summary reports p50/p95/p99 and mean
counts. Record one case per diagnostic run: this stream includes fixture setup,
Criterion warmup, and calibration. Counters need not describe identical work for
every frame. Diagnostic serialization/timing adds overhead, so compare ordinary
uninstrumented runs separately. The runner clears an inherited
`ALA_FRAME_STATS` unless `--frames` is explicitly supplied.
`--min-elements` can exclude empty fixture frames; the summary reports how many
records were skipped. This does not exclude Criterion warmup/calibration.

`present_submit` measures the CPU-side platform call. It does not establish GPU
completion, compositor presentation, input-to-photon latency, or scanout time.
In `ui_cpu` it is a test-platform call with no GPU work.

## CPU sampling

```sh
python3 tools/perf.py profile --label grid-profile --filter '^ui_cpu/grid/10000$' --seconds 10
```

On Linux this records user-space CPU-clock samples and DWARF call stacks using
`perf`. Hardware cycle counters are unnecessary. Results include `perf.data`
and a self-time report; if Inferno is installed, they also include folded stacks
and an SVG flame graph. Binutils `c++filt`, when available, demangles Rust names
that distro perf versions leave mangled. Stack export preserves physical call
stacks without inline expansion, avoiding slow external symbolizer lookups.
Criterion's profiling mode avoids statistical benchmark
sampling, but the recording still includes process setup, warmup, and teardown.
Use a sufficiently long recording and inspect the workload's call stacks.
Profile runs are diagnostic, not timing baselines. `perf` also supports
`perf report -i target/perf-results/grid-profile/perf.data` interactively.
Use `python3 tools/perf.py report target/perf-results/grid-profile` to regenerate
the text report and flame graph from an existing capture.

If the host disallows `perf_event_open`, Valgrind/Callgrind can count executed
instructions without kernel sampling. Its timings are not representative of
native execution. Valgrind Massif can investigate heap usage; allocations and
leaks are separate questions from CPU sample counts.

## Rendering validation and target hardware

```sh
export RUSTFLAGS='-C force-frame-pointers=yes'
cargo test --locked -p ala-platform --profile perf \
  --features test-support,bench-support,font-kit,wayland,x11 --test headless
python3 tools/perf.py run --bench invalidation --label invalidation
python3 tools/perf.py run --bench inspector_render --label inspector
```

`vulkaninfo --summary` identifies the Vulkan device. A CPU device such as
llvmpipe/lavapipe validates shaders and rendered pixels, but its timing measures
software rendering. Keep those results separate from `ui_cpu` and from hardware
results. The existing headless benchmark path warns when it selects a software
adapter. Headless submission timings alone do not measure GPU completion.

For actual GPU optimization, run on the target backend and hardware with the
same scene sizes and display scale. Use Metal System Trace/GPU captures on
Apple platforms or RenderDoc and supported GPU timestamp queries with wgpu.
Hardware timestamps, frame pacing, missed display deadlines, power, and real
input latency still need target-machine instrumentation; the runner does not
claim to implement those measurements.

## Tool verification

```sh
python3 -m unittest discover -s tools -p 'test_perf.py'
cargo fmt --all -- --check
```

The Python tests cover real frame-stat records, missing CPU clocks, isolated
Criterion result discovery, and rejection of incomparable runs. Rust benchmark
assertions validate that the workload actually renders and that idle work does
not redraw.
