# Segmented text shaping

**Blocked on correctness.** This benchmark-only decorator caches independently shaped segments using [Mesa Hills Research's segment helpers](https://github.com/mesa-hills-research/gpui/tree/30d883237d5f913f2d11cfcb938bd7697da88aa6/crates/gpui/src). It changes no production text implementation.

Linux verification finds 5 differing wrap results in 400 comparisons and up to 0.318359375 px glyph-position drift. Glyph identities match in this corpus, but that is insufficient for layout equivalence. `--verify-only` reports `layout_equivalent: false`; successful process completion means the experiment completed, not that segmentation is safe to ship. CoreText and DirectWrite may expose additional differences.

The cache is bounded by 262,144 glyphs, compares full keys after hashing and clears on font generation changes. Detected RTL ranges, tabs, bidi controls, newlines and other controls fall back to full-line shaping. The filter does not reject every mixed-script input. This is not a complete typography conformance corpus.

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
python3 experiments/segmented-shaping/run.py --build --verify-only --output verify.json
python3 experiments/segmented-shaping/run.py --build --repetitions 5 --samples 120 --output real-machine-segmented-shaping.json
```

Windows PowerShell:

```powershell
py -3 experiments/segmented-shaping/run.py --build --verify-only --output verify.json
py -3 experiments/segmented-shaping/run.py --build --repetitions 5 --samples 120 --output real-machine-segmented-shaping.json
```

The runner builds the selected benchmark, records source/compiler/binary hashes and native backend, runs correctness checks, alternates baseline/candidate order, and saves raw samples plus a readable `.summary.json`. Allocation requests are counted in separate processes with instrumentation disabled for timing passes; they do not measure retained memory or RSS. Save both result files and the machine details with your report.

Inspect `verification.text` before considering speedups. The runner also saves `correctness_blocked` and prints a warning before diagnostic timings when exact layout differs. Successful execution still means the experiment completed. The verifier records the first unequal glyph/line, segment-cut widths, wrap boundaries, maximum-drift location and a repeatable ASCII probe, and asserts exact fallback results and direct font invalidation. External shared-backend generation changes remain untested; raw native font loading does not necessarily advance the platform generation. Repeat the correctness corpus on all three native text systems. Add real application fonts, ligatures, combining marks, emoji, selection/caret positions, mixed fonts, edits around segment boundaries and adversarial wrap widths before designing a production cache. Cold paragraphs and unchanged text are negative controls; include their regressions in any report. A production proposal must eliminate differences or define and accept a new layout contract first.

## Cloud evidence

`cloud.json` contains raw process/sample results; `cloud.summary.json` contains the summary and verification. These are Linux VM measurements against the in-process baseline from commit `b9b22210d37bd7157161a238f602136a65095723`, not physical GPU benchmarks. See the PR for measured results and validation status.
