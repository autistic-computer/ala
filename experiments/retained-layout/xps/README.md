# XPS native validation, 2026-10-29

**Verdict: the steady Taffy kernel result passes native validation. Keep draft as an integration-planning experiment. No ALA application frame-time improvement is established.**

Tested source `ccd4ea0bc09eaa13f3b4b24fe7969f9bd716cf6f`, Rust 1.99.0, separate clean worktree. XPS 13 9300, Ubuntu 26.04.1, i7-1065G7, AC/performance mode. Display remained Wayland 1920×1200 at 59.950 Hz/125%. [Machine snapshot](machine.json) links the shared full hardware audit. This experiment exercises no renderer, text backend or GPU.

The verifier passes 499,328 exact relative node rectangles: four dirty cases × 32 frames × 3,901 nodes (3,840 leaves, 60 panels and one root), with rounding disabled. [Verification](verify-xps.json) records the native binary/source hashes. All three modes also produce matching final geometry checksums in every process; [control checks](control-checks.json) are an additional aggregate check, weaker than exact per-node comparison.

Five alternating process repetitions × 120 samples per mode/case, CPU 2, with separate allocation passes:

| Dirty leaves | Rebuild median | Retained median | Ratio | Rewrite-all median | Rewrite-all ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| 0 | 2.4663 ms | 0.1251 ms | 19.72× | 2.1768 ms | 1.13× |
| 1 | 2.5084 ms | 0.2126 ms | 11.80× | 2.2047 ms | 1.14× |
| 64 | 2.5199 ms | 0.2255 ms | 11.18× | 2.2256 ms | 1.13× |
| 3,840 | 2.5566 ms | 2.2695 ms | 1.13× | 2.2760 ms | 1.12× |

The unchanged retained case requests zero allocation bytes/sample. One/64 dirty leaves request 286,584 bytes; all-dirty and rewrite-all request 5,750,680 bytes, versus 5,782,360 for rebuild. These are separate allocation-request measurements, without a retained-memory/RSS claim.

The baseline clears and reuses Taffy tree storage. Retained modes use fixture-provided stable IDs and still construct/compare every leaf style. Only the first N leaf heights alternate; the 64-leaf case is localized to one panel. Rewrite-all calls `set_style` on every leaf, which marks Taffy nodes dirty even when the style is unchanged. Its small improvement confirms that persistent storage alone does not provide the sparse-update benefit.

Timed work includes style rebuilding/updating, layout computation and one root lookup. Full geometry extraction/checksum happens after timing. Initial tree creation/compute and 30 warmup frames are excluded, so these results do not measure cold mounting. No implementation fix was required.

[Raw samples](xps-retained-layout.json), [summary](xps-retained-layout.summary.json), [commands](commands.txt). Each JSON records exact source/compiler/binary hashes and child commands. All builds and workloads ran sequentially.

ALA reconciliation, real stable identities, node removal/reordering, lifetime bounds, style conversion, text/constraint invalidation, measurement closures, input and rendering remain outside this experiment. Measure those costs and unchanged focus/input behavior in an ALA window before proposing a production integration. Windows/MSVC and Apple native CPU validation remain outstanding.
