# ala: the fork of GPUI

ala is GPUI, split out of its upstream repository on 2026-10-05 and owned since. It
is not a patch branch: nothing gets rebased onto upstream, and upstream fixes come in by hand when
we want them. [zebrilus][zebrilus] and nidus carry ala as the git subtree `libs/ala` and edit it
there; the penna repo builds on it as a git dependency.

- Forked from: upstream at `933d8d9381`, limited to the gpui crates.
- The 89 commits after that one are the old patch fork's `zz-patches` branch (tip
  `5a00ac89a4`), replayed under new IDs. That fork stays up as an archive and zebrilus's knowledge
  pages cite its commit IDs; the same commits exist here with the same subjects.
- Upstream license: Apache-2.0, retained as `LICENSE-APACHE`. Third-party notices are in `NOTICE.md`.

## Layout

```
crates/
  ala/            gpui core, with upstream's utility crates as modules: collections, util,
                  sum_tree, scheduler, http_client, refineable
  ala-macros/     ala's macros and the Refineable derive
  ala-platform/   every backend as a module built only for its target: apple (Metal),
                  macos, linux, windows, web, wgpu, ios, android
third_party/
  cosmic-text/    cosmic-text with ala's patches
```

The ios and android backends have no upstream counterpart.

## Upstream paths in ala

| Upstream | ala |
|---|---|
| `crates/gpui/src/` | `crates/ala/src/` |
| `crates/gpui_macros/src/` | `crates/ala-macros/src/` |
| `crates/refineable/derive_refineable/src/` | `crates/ala-macros/src/derive_refineable.rs` |
| `crates/gpui_platform/src/` | `crates/ala-platform/src/ala_platform.rs` |
| `crates/gpui_<x>/src/` for `macos`, `linux`, `windows`, `web`, `wgpu` | `crates/ala-platform/src/<x>/`, root file `<x>.rs` (Linux: `linux.rs` and `linux/`) |
| `crates/gpui_apple/src/` (Metal) | `crates/ala-platform/src/apple/` |
| `crates/<x>/src/` for `collections`, `http_client`, `scheduler`, `sum_tree`, `refineable` | `crates/ala/src/<x>/`, root file `<x>.rs` |
| `crates/util/src/` (the parts ala uses) | `crates/ala/src/util.rs` |
| `gpui::`, `gpui_<x>::`, `collections::` | `ala::`, `ala_platform::<x>::`, `ala::collections::` (`crate::` inside the crate) |
| `ZED_*` environment variables: `HEADLESS`, `DEVICE_ID`, `FONTS_GAMMA`, `FONTS_GRAYSCALE_ENHANCED_CONTRAST`, `FONTS_SUBPIXEL_ENHANCED_CONTRAST`, `MEASUREMENTS`, `RESTART_*` | `ALA_*` with the same suffix |
| `tooling/perf`, screen capture (`scap`, ScreenCaptureKit), tracy, `zlog`, `ztracing`, `util_macros` | removed: skip upstream patches that only touch them |

## Pulling a fix from upstream

Export the patch, map its paths and identifiers with the table above, then apply it. A backend's
root file moved down one directory, so a patch to `crates/gpui_wgpu/src/gpui_wgpu.rs` lands in
`crates/ala-platform/src/wgpu.rs`, and `crate::` paths inside a backend become `crate::<x>::`.

```bash
git -C <upstream-checkout> format-patch -1 <sha> --stdout -- crates/gpui crates/gpui_wgpu \
  | sed -e 's#crates/gpui_wgpu/src/#crates/ala-platform/src/wgpu/#g' -e 's#crates/gpui/#crates/ala/#g' \
        -e 's/\bgpui_wgpu::/ala_platform::wgpu::/g' -e 's/\bgpui::/ala::/g' > fix.patch
git am -3 fix.patch
```

From zebrilus, apply the same patch with `git am -3 --directory=libs/ala`. Limit the pathspec to
the crates the fix touches. Never bulk-merge upstream: take what you need, read it, and run the GPU
tests for the renderers it touches.

## Last review

Upstream reviewed through `bc6fc11` (2026-11-02); start the next pass there. Still open from that
pass: `0373366` (CAMetalDisplayLink frame pacing, ported on the `upstream-macos` branch, untested
with zz's underlay), and the perf changes `772a8a9`, `3389d87`, `b4ecc45` and `1dd9bbf`. Skipped
for good: the objc2 migration, upstream's iOS backend, `ScrollWheelEvent` rename, frame telemetry,
hang monitor, benchmarks, and the Low Power Mode 30 fps cap.

## What ala adds

`git log` is the authority, and zebrilus's [ala reference][reference] has the long version. The
core six:

1. `RenderImage::into_frames()`: retired frames hand their pixel buffers back to the caller's pool.
2. `WgpuDeviceContext`: `Window::wgpu_device_context()` exposes the wgpu renderer's `Device` and
   `Queue` (plus an `ala::wgpu` re-export) so an app creates GPU resources on ala's own device.
3. The external-texture element: `ala::external_texture(wgpu::Texture)` paints an app-provided
   texture with normal clipping and HiDPI through the repurposed surface pipeline (a dead YCbCr
   stub upstream).
4. The window corner mask: `Window::set_window_corner_mask()` clips everything the window draws
   except drop shadows to a rounded rect, so scrollbars or surfaces cannot escape a client-drawn
   frame's rounded corners. The wgpu renderer applies it through per-frame globals in every
   fragment shader.
5. Corner shape: `Window::set_default_corner_smoothing()` swaps every quad's circular arc for a
   superellipse (`2.0` circular, `4.0` squircle), and `set_adaptive_corner_fraction()` resolves an
   ordinary radius against the element so one global setting cannot turn a component into a
   pill. Shadows and the corner mask trace the same exponent; a circular mask around a squircle
   frame cuts the frame's own border off over the arc. `CornerRadiusMode::Fixed` bypasses the
   adaptive compression, `Styled::corner_smoothing` overrides only the element's own fill, border
   and shadows, and fully rounded quads stay true circles.
6. Rounded content masks: every primitive carries a `ClipMask`, the rectangle the vertex stage
   clips to plus an optional rounded rect (bounds, per-corner radii, exponent) that the fragment
   stage applies with the quad's own coverage formula, in every renderer and for every primitive
   kind. A `div` with both overflow axes clipped and rounded corners pushes one
   (`Style::overflow_clip_mask`, `Window::with_clip_mask`), inset to its padding box with each
   radius reduced by the wider adjacent border. A rectangle inside a rounded mask keeps the rounded
   shape exactly. Two rounded masks merge into one: the rounded rect is their intersection, and
   each corner takes the largest radius either mask still needs there (its own radius minus how far
   the new corner sits inside it along the farther axis), so nothing escapes either mask.
   Concentric masks (inner radius = outer radius minus the inset) come out exact; corners that
   do not line up get trimmed slightly more than the true intersection. The exponent comes from the
   mask that needs the larger radius. A plain rectangle costs 40 more bytes per instance and one
   branch per fragment. Shader layers under a rounded mask render their custom shader into the
   layer's scratch target and composite it through the mask; glass reads the mask from its uniform.
   Hit testing stays rectangular.

Later additions include terminal glyph render effects, CoreGraphics stroke variants, device-pixel
synthetic bold, BGRA CoreVideo surfaces, `PlatformDisplay::refresh_rate()`, `TestWindow` raw
handles that return `HandleError::Unavailable` instead of panicking, two drag fixes
(pointer-transparent previews, a drag that survives a rejected `can_drop`),
`TextSystem::underline_thickness`, an optional traffic light scale in `TitlebarOptions`,
CSS-correct spread shadows, per-window content zoom (`Window::set_zoom()`), a usable GPU under WSL
(`ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER` only when `/dev/dxg` exists), `KeystrokeEvent::is_held`,
liquid glass, the iOS UIKit and Android backends, and the wgpu subpixel offset kept in device
pixels.

## Gotchas when changing renderers or pulling upstream work

- GPU-facing structs must stay a multiple of 8 bytes (const-asserted in `scene.rs`), or every draw
  fails wgpu's binding-size check while Metal silently reads at a skew.
- ala ships wgpu 30; upstream still pinned 29 at the split. Upstream wgpu code written against 29 needs
  `color_space: SurfaceColorSpace::Auto` on both `SurfaceConfiguration`s and
  `Queue::present(frame)` instead of `frame.present()`. The `apply_limit_buckets` change only
  compiles against 30.
- Upstream split instance loading into `shaders_storage.wgsl` and `shaders_webgl.wgsl` with
  per-record `load_*` functions. The storage variant reads the WGSL structs directly, but the WebGL
  loaders hard-code word strides and read sequences: every change that widens a scene struct
  (Quad, Shadow, PolychromeSprite, SurfaceParams) must also touch them, and nothing catches a miss
  at compile time. `Quad` must stay a multiple of 16 bytes (52 words) because the WebGL quad
  decoder reads whole texels.
- Upstream perf tracking sits behind the `profiler` feature. Leave it off in production builds for
  its collection overhead, and never turn it on for wasm: it uses native timing and crashes on
  action dispatch there. `set_frame_trace_enabled` became `set_trace_enabled`, and collectors
  yield `FrameEvent::{Draw,Present}` instead of bare `FrameTiming`.

## Checking a change

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --exclude ala-platform
cargo test -p ala-platform --all-features
```

ala-platform's tests need a GPU, or Mesa lavapipe on Linux
(`VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`). In zebrilus, `just libs check ala` runs
the first two, and the third takes `--manifest-path libs/ala/Cargo.toml`.

Each backend compiles for its own target only. From macOS:

- Linux: `cargo check -p ala-platform --all-features --target x86_64-unknown-linux-musl`, with a
  `zig cc -target x86_64-linux-musl` wrapper named `x86_64-linux-musl-gcc` on `PATH` (drop
  cc-rs's `--target=` flag; a `zig c++` twin as `x86_64-linux-musl-g++`),
  `RUST_FONTCONFIG_DLOPEN=1` and `FREETYPE2_NO_PKG_CONFIG=1`.
- Windows: `cargo check -p ala-platform --all-features --target x86_64-pc-windows-msvc` with
  `RC_x86_64_pc_windows_msvc` set to Homebrew's `llvm-rc`.
- Windows shaders: the cargo check above never compiles the HLSL. The backend compiles
  `shaders.hlsl` and `color_text_raster.hlsl` with the legacy `fxc` at shader model 4.1 (in
  `build.rs` for release builds, through `D3DCompileFromFile` at runtime for debug builds), and
  `fxc` only runs on Windows. The DirectX Shader Compiler builds on macOS from its source in about
  five minutes (`cmake -G Ninja -C cmake/caches/PredefinedParams.cmake
  -DCMAKE_BUILD_TYPE=Release -DHLSL_INCLUDE_TESTS=OFF -DLLVM_INCLUDE_TESTS=OFF
  -DCLANG_INCLUDE_TESTS=OFF -DSPIRV_BUILD_TESTS=OFF -DENABLE_SPIRV_CODEGEN=OFF`, then
  `ninja dxc`). With `dxc` on `PATH`, compile every entry point the backend loads:

  ```bash
  (cd crates/ala-platform/src/windows &&
  for m in quad shadow path_rasterization path_sprite underline monochrome_sprite \
      subpixel_sprite polychrome_sprite surface emoji_rasterization; do
    f=shaders.hlsl; [ "$m" = emoji_rasterization ] && f=color_text_raster.hlsl
    for s in vs:vertex ps:fragment; do
      dxc -nologo -HV 2018 -WX -T "${s%:*}_6_0" -E "${m}_${s#*:}" -Fo /dev/null "$f" || exit 1
    done
  done)
  ```

  `dxc` has no shader model 4 targets, so this checks the same source at 6.0 with the 2018
  language rules `fxc` follows. It catches syntax, type and missing-symbol errors; what it
  misses is what only shader model 4 rejects, such as gradient sampling inside varying flow
  control.
- iOS: `cargo clippy -p ala-platform --all-features --target aarch64-apple-ios`.
- Android: point `CC_aarch64_linux_android`, `AR_aarch64_linux_android` and
  `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` at the NDK's
  `toolchains/llvm/prebuilt/<host>/bin` clang and `llvm-ar`, then
  `cargo clippy -p ala-platform --all-features --target aarch64-linux-android`. Clippy 1.99
  reports `missing_const_for_thread_local` there for `thread_local!`s that are already `const`,
  so ala allows it on Android.

[zebrilus]: https://github.com/autistic-computer/zebrilus
[reference]: https://github.com/autistic-computer/zebrilus/blob/main/knowledge/references/ala.md
