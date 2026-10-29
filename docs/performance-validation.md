# Real-machine validation for the CPU performance drafts

The committed cloud evidence covers Linux CPU work and software Vulkan pixels.
Apple Silicon, native Linux GPU, Windows, and production application performance
remain pending. Compare each machine with itself using the same fonts, display
scale, compiler, features, and workload. Do not compare numbers across platforms
as evidence for this change.

## Prepare both builds

Use a clean checkout with the pinned Rust toolchain, Python 3.11+, and the
platform's native build prerequisites. The branches form a stack: the toolkit
branch contains the baseline harness, and the borrowing branch adds the
optimization. Fetch and build both before measuring:

```sh
git fetch origin
git switch codex/ala-cpu-perf-toolkit
python3 tools/perf.py doctor
python3 tools/perf.py snapshot --label native-original
git switch codex/ala-borrow-layout-styles
python3 tools/perf.py snapshot --label native-borrowed
```

On Windows use `py -3` instead of `python3`. The snapshot retains its source and
binary hashes, so switching branches later does not relabel the original build
as the candidate. Use fresh labels for repeated experiments.

## Timing protocol

Finish builds and profilers, close other busy applications, and keep power and
thermal conditions steady. Run three pairs in order A1/B1, B2/A2, A3/B3. For
example, the first pair on Linux or macOS is:

```sh
python3 tools/perf.py run --variant target/perf-results/native-original --label native-a1 --warmup 2 --measurement 10 --samples 50
python3 tools/perf.py run --variant target/perf-results/native-borrowed --label native-b1 --warmup 2 --measurement 10 --samples 50
python3 tools/perf.py compare target/perf-results/native-a1 target/perf-results/native-b1
```

Repeat with labels `native-b2`, `native-a2`, `native-a3`, `native-b3` and compare
matching pairs. Keep all runs, including regressions. Examine both per-run
confidence intervals and variation between runs. Do not record frame JSON,
allocations, screenshots, or profiles during timing runs.

On Linux, prefix each timing command with `taskset -c N`, choosing an allowed
CPU from `doctor`. On macOS there is no equivalent hard-affinity setting in this
runner; record machine and power conditions. Native application validation must
also use the application's production feature set: `bench-support` enables
ala's internal profiler.

## Linux desktop with a GPU

Install the dependencies listed in [performance.md](performance.md). Confirm
the adapter with `vulkaninfo --summary`; a CPU/lavapipe adapter validates software
rendering rather than target GPU performance. Do not load the cloud's software
driver environment script. Run the full CPU protocol above, then separately:

```sh
cargo test --locked -p ala --profile perf --no-default-features --features bench-support,test-support,wayland,x11 --lib
cargo test --locked -p ala-platform --profile perf --features test-support,bench-support,font-kit,wayland,x11 --test headless
python3 tools/perf.py run --bench invalidation --label native-invalidation
```

Run a representative application workload on the hardware renderer. Use `perf`
or the toolkit's `profile` command separately for CPU attribution. Use RenderDoc
and supported GPU timestamp queries for GPU completion, and native tracing for
frame pacing. Headless submission timing alone does not establish those metrics.

## Mac with Apple Silicon

Install Xcode Command Line Tools, the pinned Rust toolchain, and Python 3.11+.
Run the full CPU protocol and repository tests. Confirm the native text backend,
font availability, and output scale before interpreting the text cases. This
workflow has not been run on macOS by this chat.

Exercise the actual application on Metal with Time Profiler and Metal System
Trace or Xcode GPU capture. Record scrolling, resizing, text updates, idle
behavior, frame deadlines, and power in separate runs. The toolkit's Linux
`perf` sampling command is not a macOS profiler. Preserve native Instruments
traces with the tested commit and application feature set.

## Windows desktop with a GPU

Install the Rust MSVC toolchain, Visual Studio C++ build tools/Windows SDK,
required native dependencies, and Python 3.11+. Use PowerShell with `py -3`.
The current headless Windows platform supplies `NoopTextSystem`, so use this
filter on every snapshot comparison run:

```powershell
py -3 tools/perf.py run --variant target/perf-results/native-original --label windows-a1 --filter '^ui_cpu/(grid/|idle_no_redraw$)' --warmup 2 --measurement 10 --samples 50
py -3 tools/perf.py run --variant target/perf-results/native-borrowed --label windows-b1 --filter '^ui_cpu/(grid/|idle_no_redraw$)' --warmup 2 --measurement 10 --samples 50
py -3 tools/perf.py compare target/perf-results/windows-a1 target/perf-results/windows-b1
```

Repeat in the same paired order. Windows compilation and execution of this
toolkit are pending. Snapshot files use `.exe`, but this naming support is not
a Windows validation result. Native text validation must use the application's
DirectWrite backend, not these headless text cases.

ALA currently has no Windows headless screenshot renderer. Use the real
DirectX application for pixel checks and representative interaction/scrolling
workloads. Capture CPU traces with Windows Performance Recorder/Analyzer and
GPU/frame pacing evidence with appropriate DirectX tooling/PresentMon. GPU
completion and display latency are separate from the CPU benchmarks.

## Remaining gates before merging the optimization

- CI on the published revisions and supported feature combinations.
- Repeated native CPU comparisons on Linux, Apple Silicon, and Windows; confirm
  text with real native backends where applicable.
- Representative application behavior and timings with production features.
- Investigate the cloud's approximately 2.7 ns synthetic idle regression.
- Native rendered pixels, GPU completion, frame pacing, input latency, and
  power; no claim about those measurements has been established by the cloud.
