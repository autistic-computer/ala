# Opacity paint replay

An opt-in prototype replays cached paint operations with a new opacity scalar, avoiding reconstruction of an unchanged pane. It is adapted from [Mesa Hills Research's paint-effect records](https://github.com/mesa-hills-research/gpui/commit/eb63578acb9637ba72146dfafc07e4e1cb03d16b). Production behavior is unchanged without the `perf-experiments` feature.

This is a manually driven opacity experiment, not an animation scheduler. It does not support native surfaces, shader layers or glass inside an opacity scope (these reject the operation). General transforms, lifetime handling and a production animation API remain integration work. The fixture deliberately uses an unvirtualized pane; a virtualized application's benefit may be much smaller.

## Machines and prerequisites

Use the pinned Rust 1.99 toolchain and Python 3.10+. Run from this checkout's root.

| Machine | Native backend | Prerequisites |
| --- | --- | --- |
| Apple Silicon Mac | CoreText / Metal | Full Xcode with the Metal toolchain, command line tools selected with `xcode-select`, Rust, Python 3 |
| Windows GPU desktop | DirectWrite / Direct3D 11 | Visual Studio 2022 C++ Build Tools, Windows SDK, Rust MSVC toolchain, Python 3 (`py -3`) |
| Linux GPU desktop | cosmic-text / Vulkan | Fontconfig, FreeType, xkbcommon, Wayland and X11 development packages; Vulkan loader and vendor driver |

For example on Debian/Ubuntu, install `build-essential pkg-config libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libxcb1-dev libxcb-xkb-dev libvulkan1`. Package names can vary with the distribution. Core CPU fixtures use bundled IBM Plex Sans/Lilex; native fallback fonts can differ across machines.

Keep the machine plugged in with a fixed power mode, close background workloads, use the same build options, and record OS, CPU, GPU, driver, display scale and refresh rate. Do not compare wall times across different machines as an optimization ratio: each run measures baseline and candidate locally, alternating order. On Linux, optionally add `--cpu N` for an available logical CPU. Omit this flag on macOS and Windows.

## Correctness and CPU measurements

macOS / Linux:

```sh
python3 experiments/paint-replay/run.py --build --verify-only --output verify.json
python3 experiments/paint-replay/run.py --build --repetitions 5 --samples 120 --output real-machine-paint-replay.json
```

Windows PowerShell:

```powershell
py -3 experiments/paint-replay/run.py --build --verify-only --output verify.json
py -3 experiments/paint-replay/run.py --build --repetitions 5 --samples 120 --output real-machine-paint-replay.json
```

The runner builds the selected benchmark, records source/compiler/binary hashes and native backend, runs correctness checks, alternates baseline/candidate order, and saves raw samples plus a readable `.summary.json`. Allocation requests are counted in separate processes with instrumentation disabled for timing passes; they do not measure retained memory or RSS. Save both result files and the machine details with your report.

Correctness checks compare 24 changing scenes and hitboxes, then dispatch two cached click events. macOS and Linux also compare all 46,080,000 rendered pixels byte-for-byte and require distinct animation images. Windows has no headless screenshot renderer in ALA: `pixel_checks_available: false` is expected there and must not be reported as a pixel pass. Inspect/capture the native window on Windows.

## Native window and GPU profiling

Use the same 800 x 600 logical window, display, scale and refresh rate for each mode. The pane contains 480 rows by default; `--rows` changes that load. Run baseline and replay at least five times in alternating order. Run static once as an idle control; it should stop requesting animation frames after initialization.

macOS / Linux (repeat with `replay` and `static`, changing the output filename):

```sh
ALA_FRAME_STATS=baseline.ndjson python3 experiments/paint-replay/run.py --build --live baseline --seconds 60
```

Windows PowerShell:

```powershell
$env:ALA_FRAME_STATS = "baseline.ndjson"
py -3 experiments/paint-replay/run.py --build --live baseline --seconds 60
# Repeat for replay and static with matching filenames.
Remove-Item Env:ALA_FRAME_STATS
```

Linux live mode enables both X11 and Wayland; set `XDG_SESSION_TYPE=x11` or `wayland` to match the session if necessary. Review the fade, text, borders and click behavior. Exclude the first second of trace data, compare median/p95 `draw.wall_ns`, phase times, `views_rendered`, `layout_nodes` and frame counts. The root still renders each animation frame; the cached pane should rebuild only in baseline. `present_submit` is CPU submission/blocking time, not GPU execution duration.

Capture actual GPU work separately with Instruments Metal System Trace (Mac), PIX/GPUView (Windows), or a vendor-supported Vulkan profiler on Linux. Record process CPU usage and frame pacing under the same refresh rate. Do not infer GPU/power improvements from the CPU-only fixture. The cloud live-window check uses Xvfb + llvmpipe and establishes execution, not hardware performance.

## Cloud evidence

`cloud.json` contains raw process/sample results; `cloud.summary.json` contains the summary and verification. These are Linux VM measurements against the in-process baseline from commit `b9b22210d37bd7157161a238f602136a65095723`, not physical GPU benchmarks. See the PR for measured results and validation status.

[Apple Silicon validation](macos/README.md) covers native CoreText/Metal correctness, CPU fixture/live pairs, static idle behavior and target-process GPU diagnostics. Representative application and physical display/input/power gates remain open.
