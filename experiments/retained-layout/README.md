# Retained layout kernel

A pure Taffy experiment compares rebuilding a 3,901-node tree against retaining it and updating only changed styles. The fixture supplies stable identities. **This is not an integration into ALA's layout engine.** It measures the kernel ceiling before paying for reconciliation and dirty tracking.

Keeping a tree while rewriting every style is a negative control: retaining nodes alone is insufficient. A production implementation still needs stable IDs, removal/reordering, bounded lifetime, invalidation for text/constraints/style changes, focus/input correctness and an end-to-end ALA benchmark that includes reconciliation costs.

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
python3 experiments/retained-layout/run.py --build --verify-only --output verify.json
python3 experiments/retained-layout/run.py --build --repetitions 5 --samples 120 --output real-machine-retained-layout.json
```

Windows PowerShell:

```powershell
py -3 experiments/retained-layout/run.py --build --verify-only --output verify.json
py -3 experiments/retained-layout/run.py --build --repetitions 5 --samples 120 --output real-machine-retained-layout.json
```

The runner builds the selected benchmark, records source/compiler/binary hashes and native backend, runs correctness checks, alternates baseline/candidate order, and saves raw samples plus a readable `.summary.json`. Allocation requests are counted in separate processes with instrumentation disabled for timing passes; they do not measure retained memory or RSS. Save both result files and the machine details with your report.

The correctness gate compares 499,328 exact node rectangles over 128 frames. Compare unchanged, one dirty leaf, 64 dirty leaves and all-dirty cases. The all-dirty and rewrite-all modes are negative controls. No graphics driver or native renderer is exercised by this experiment, so a GPU profile of it has no meaning. Native-machine runs check whether the CPU result survives different CPUs and operating systems.

## Cloud evidence

`cloud.json` contains raw process/sample results; `cloud.summary.json` contains the summary and verification. These are Linux VM measurements against the in-process baseline from commit `b9b22210d37bd7157161a238f602136a65095723`, not physical GPU benchmarks. See the PR for measured results and validation status.

[XPS/Linux native results](xps/README.md) validate the kernel and all dirty/rewrite-all controls on an i7-1065G7. They establish no ALA application frame-time improvement; [Apple Silicon results](macos/README.md) also validate the kernel. Windows/MSVC native validation remains outstanding.
