# Exact path tessellation cache

Repeated curves are tessellated by Lyon on every `PathBuilder::build()`. This draft adds an explicit caller-owned `PathCache` and `build_with_cache()` method for sites that rebuild identical paths. Existing `build()` calls are unchanged.

```rust
// Keep this with the view/model that owns the repeated geometry.
let mut paths = ala::PathCache::new(8 * 1024 * 1024);
// While painting: create the path as usual, then opt in.
let path = builder.build_with_cache(&mut paths)?;
// Releasing the view drops the cache; paths.clear() also releases its table.
```

The mesh-retention idea and key encoding are adapted from [GPUI Fast](https://github.com/longbridge/gpui-fast/blob/07b42ebf1f43088b8b74a7d4fca6b95e44e04265/crates/gpui/src/fast/path_cache.rs). Coordinates in this version stay absolute. Transforms are applied before key construction, so position/scale/rotation changes miss the cache. This preserves the same tessellation as the unmodified ALA baseline. Fill triangulation and floating-point ties can change when normalizing and translating a path; translation-invariant retention needs separate visual/geometry evaluation.

The full key includes coordinates, fill/stroke options and dashes; hash collisions still compare complete keys. Each returned path owns its vertices and can be colored/clipped/modified independently. The cache counts keys, stored path structures and vertex capacity against its payload budget; hash-table/allocator overhead is additional. Oversized entries are skipped. On exhaustion all entries are released; this simple policy can churn if the working set exceeds the budget. Expose and inspect hit/miss counts before adopting it in a caller.

The new API is opt-in. Cheap shapes, unique shapes and cold workloads can be slower. This draft does not automatically cache every path or change the scene/renderer.

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
python3 experiments/path-cache/run.py --build --verify-only --output verify.json
python3 experiments/path-cache/run.py --build --repetitions 5 --samples 120 --output real-machine-path-cache.json
```

Windows PowerShell:

```powershell
py -3 experiments/path-cache/run.py --build --verify-only --output verify.json
py -3 experiments/path-cache/run.py --build --repetitions 5 --samples 120 --output real-machine-path-cache.json
```

All three machines check exact original/cached mesh representations, including private path state, vertices, bounds and clipping defaults. macOS and Linux also compare twelve 800 x 600 rendered images and require three distinct scenes; Windows has no headless pixel renderer and reports that check unavailable. The CPU benchmark uses no native text or GPU submission.

The runner measures five alternating process repetitions, 120 samples each, and a separate allocation pass. Each sample builds 128 paths, then drops the owned meshes. Cases are repeated curves, repeated dashed curves, cheap triangles, continuously changing curves, and cold curves (cache cleared inside every sample). Check hit/miss counts: repeated warm cases should hit every path, changing/cold cases should miss every path. Report the cheap/cold/unique regressions along with warm improvements. Allocation bytes are requests, not retained memory/RSS. Results contain raw samples, source/compiler/binary hashes and a readable `.summary.json`.

Run the targeted cache tests too:

```sh
cargo test --locked -p ala --no-default-features --lib path_builder::cache::tests
```

These cover independence of returned meshes, fill/stroke/dashes/transforms, payload exhaustion, oversized entries and explicit release. Before adopting the API in a real application, instrument the actual path-heavy view to confirm tessellation is a bottleneck and reuse survives layout changes. Compare its full draw phase and process CPU profile before/after on the same native machine. An identical CPU mesh does not establish a GPU performance improvement.

## Cloud evidence and research

Linux VM medians per batch of 128 paths (5 alternating repetitions × 120 samples; pinned logical CPU 2):

| Workload | Baseline | Cached | Baseline / cached |
| --- | ---: | ---: | ---: |
| Repeated curves | 3.032 ms | 0.205 ms | 14.80× |
| Repeated dashed curves | 2.102 ms | 0.199 ms | 10.59× |
| Repeated triangles | 0.062 ms | 0.056 ms | 1.11× |
| Changing curves | 3.026 ms | 3.339 ms | 0.91× |
| Cold curves | 3.032 ms | 3.377 ms | 0.90× |

Changing/cold cases take 10–11% longer with the cache. Warm-curve allocation requests fall from 15,710,744 to 4,334,232 bytes/batch; they still include path/key construction and returned mesh copies. Verification passes 1,536 exact mesh comparisons and 5,760,000 pixels over twelve rendered images.

`cloud.json` holds raw measurements, `cloud.summary.json` the summary/verification. These are Linux VM CPU measurements with software Vulkan used only for pixel verification. [XPS/Linux native results](xps/README.md) include all workload controls, exactness tests and process-memory measurements. [Apple Silicon results](macos/README.md) also pass exactness and Metal pixels. Native Windows execution remains outstanding.

See [discovery.md](discovery.md) for source findings and why this experiment includes paths and negative controls.
