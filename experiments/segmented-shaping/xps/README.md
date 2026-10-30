# XPS native validation, 2026-10-29

**Verdict: blocked on correctness. Native cosmic-text reproduces the wrap/layout failure, and a simple ASCII probe also differs. All timing results are diagnostic and unusable as an adoption claim. Keep draft.**

Original source `ab1bc309a8bd85c02b2ab978fda90bc1875eba41`; expanded verifier/timing source `c34cd002d05e3153cdb1ad34c29c8645d20cf0b7`, Rust 1.99.0. This PR has its own worktree. XPS 13 9300, Ubuntu 26.04.1, i7-1065G7, AC/performance mode; Wayland 1920×1200 at 59.950 Hz/125%. [Machine snapshot](machine.json) links the shared hardware audit. The benchmark uses the actual Linux cosmic-text backend with bundled IBM Plex Sans/Lilex; no GPU submission occurs.

## Correctness and numerical evidence

- The original 80-paragraph corpus has five differing wraps out of 400 comparisons, maximum horizontal glyph drift 0.318359375 px, matching glyph identities, and unequal complete line layouts. Both original and expanded native runs reproduce this result.
- At the first differing glyph, `paragraph(2037, 2)`, IBM Plex Sans 16 px, byte 219/glyph ordinal 214: baseline x is **1565.02490234375**, segmented x is **1565.02392578125**. Font and glyph identities match. The separately shaped prefix width is exactly the segmented value. The float bit patterns are 1153671372 and 1153671364.
- At the next cut, byte 428, the full-line pen is 3054.491455078125 px, the independently shaped prefix width is 3054.49267578125 px, and the sum of segment widths is 3054.495849609375 px. The joined glyph uses the last value.
- The first wrap mismatch is `paragraph(2666, 19)` at width 200 px: boundary ordinal 68 chooses byte **1754** in baseline and **1759** in segmented output. Full text, boundaries and cut points are saved in the verifier JSON.
- The repeatable ASCII input `"hello ".repeat(700)` has cuts at bytes 2052 and 4104 and maximum glyph drift **0.458984375 px**. Its widths are 27462.044921875 px baseline and 27462.494140625 px segmented. The overall correctness result includes this probe.

[Original verifier](verify-original-xps.json), [expanded verifier](verify-xps.json). `correctness_blocked` is true and `layout_equivalent` is false. Pixel equivalence is not established. Successful verifier execution establishes completion and the individual assertions, not semantic equivalence.

## Cause and fixes made

cosmic-text computes [word widths](https://github.com/autistic-computer/ala/blob/c34cd002d05e3153cdb1ad34c29c8645d20cf0b7/third_party/cosmic-text/src/shape.rs#L810) separately from its [sequential glyph pen](https://github.com/autistic-computer/ala/blob/c34cd002d05e3153cdb1ad34c29c8645d20cf0b7/third_party/cosmic-text/src/shape.rs#L2709). ALA copies the native positions and line width. The experimental [join](https://github.com/autistic-computer/ala/blob/c34cd002d05e3153cdb1ad34c29c8645d20cf0b7/crates/ala-platform/benches/segmented_shaping/segments.rs#L205) instead adds local positions to accumulated segment widths. This substitutes a different prefix value and changes `f32` addition order. The numeric cut diagnostics above demonstrate the substitution. ALA's [wrap comparison](https://github.com/autistic-computer/ala/blob/c34cd002d05e3153cdb1ad34c29c8645d20cf0b7/crates/ala/src/text_system/line_layout.rs#L219) uses the resulting positions, so small drift can select a different word boundary.

`LineLayout` retains positions rather than original shaping advances/offsets/context. Joining in higher precision or adding a tolerance would not reproduce the native accumulation contract. Exact reuse needs a lower-level shaping representation and native accumulation order; whole-line fallback removes the measured append benefit. No production shaping implementation was changed, and the algorithmic blocker remains.

The verifier now saves first glyph/line/cut/wrap differences and the maximum-drift location, checks all ten injected fallback cases for exact layouts, verifies direct font-cache release and warm-cache hits, and adds the ASCII probe to the overall result. The runner emits `correctness_blocked`, warns before timings, enforces the 262,144-glyph bound in timing/allocation processes, and includes native stderr on execution failures. The guide and helper comments now describe the mixed-script filter accurately.

An intermediate test assumed raw font loading advanced the backend generation; that is not the native contract and the check was corrected before measurement. Its failure logs are preserved. Direct invalidation through the decorator passes; external shared-backend generation changes remain untested. The cut filter rejects detected RTL ranges and controls, rather than every mixed-script input.

## Diagnostic CPU measurements

Five alternating process repetitions × 120 samples per mode/case, CPU 2, with separate allocation passes. Timed work assigns prepared input and runs headless ALA text layout/draw/arena clear. Input strings are prepared before timing. These measurements exclude GPU submission and native-window frame pacing, and compare unequal layouts.

| Input | Workload | Baseline | Segmented | Ratio |
| --- | --- | ---: | ---: | ---: |
| 2 KB | Append | 1.6425 ms | 0.6547 ms | 2.51× |
| 2 KB | Middle | 1.3555 ms | 0.6648 ms | 2.04× |
| 2 KB | Cold | 1.3604 ms | 1.4394 ms | 0.95× |
| 2 KB | Unchanged | 0.5109 ms | 0.5026 ms | 1.02× |
| 20 KB | Append | 8.9370 ms | 1.0361 ms | 8.63× |
| 20 KB | Middle | 8.7493 ms | 1.2308 ms | 7.11× |
| 20 KB | Cold | 8.8695 ms | 9.3854 ms | 0.95× |
| 20 KB | Unchanged | 0.7421 ms | 0.7300 ms | 1.02× |
| 100 KB | Append | 46.7726 ms | 3.0895 ms | 15.14× |
| 100 KB | Middle | 46.6381 ms | 3.2364 ms | 14.41× |
| 100 KB | Cold | 46.7922 ms | 47.1455 ms | 0.99× |
| 100 KB | Unchanged | 1.5302 ms | 1.5217 ms | 1.01× |

Cold regressions are 5.8%, 5.8% and 0.8%; unchanged text is roughly equal. Maximum recorded retained glyph count across all 144 processes is 254,305, below the limit. Allocation requests and glyph counts do not measure process RSS.

[Raw data](xps-segmented-shaping.json), [summary](xps-segmented-shaping.summary.json), [commands](commands.txt), [Clippy](clippy.log.gz), [format check](format.log.gz). Clippy passes with warnings denied and formatting passes. The expanded verifier's fallback/cache assertions pass while its exact-layout result remains false. All workloads/builds ran sequentially.

Recommend blocking adoption and preserving the existing typography contract. Broader fonts/scripts, ligatures, combining marks, emoji, caret/selection behavior, edits around cuts, larger correctness inputs and pixel equivalence need evaluation after a backend-level design. Windows/MSVC/DirectWrite and Apple/CoreText native execution remain outstanding on this Ubuntu machine.
