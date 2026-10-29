# XPS native validation, 2026-10-29

**Verdict: exactness passes and repeated complex paths improve substantially. Keep the API opt-in; caller adoption lacks application-level evidence. Cold/changing paths regress, and cheap triangles gain nothing. Keep draft.**

Tested source `5e0a214dbb12cda890f62be68d4e3a852547acc2`, Rust 1.99.0. This PR uses its own clean worktree. The XPS 13 9300 runs Ubuntu 26.04.1 with an i7-1065G7 and Intel Iris Plus G7/i915/Mesa 26.0.8. AC/performance mode, GNOME Wayland, 1920×1200 at 59.950 Hz and 125% scale stayed matched. [Machine snapshot](machine.json) links the complete hardware audit, including the confirmed hardware-accelerated native renderer.

## Correctness

All 1,536 exact complete-mesh comparisons and all 5,760,000 pixels over twelve images pass, with three distinct scenes. Linux reports pixel checks available. All three targeted cache tests pass, covering independent returned meshes, style/dash/transform separation, payload exhaustion, oversized entries and explicit release. No implementation fix was required.

[Verification](verify-xps.json), [test log](cache-tests.log.gz), [commands](commands.txt).

## CPU controls

Five alternating process repetitions × 120 samples, pinned CPU 2. Each sample builds and drops 128 paths. Allocation passes run separately. These timings cover path construction, keying, tessellation or mesh cloning, and drop; they exclude GPU submission and application reconciliation.

| Workload | Baseline median | Cached median | Baseline/cached |
| --- | ---: | ---: | ---: |
| Repeated curves | 4.3952 ms | 0.3064 ms | 14.34× |
| Repeated dashed curves | 2.7809 ms | 0.2834 ms | 9.81× |
| Repeated triangles | 0.0862 ms | 0.0872 ms | 0.99× |
| Changing curves | 4.4602 ms | 4.9538 ms | 0.90× |
| Cold curves | 4.4892 ms | 5.1390 ms | 0.87× |

Triangles take 1.1% longer, changing curves 11.1% longer, and cold curves 14.5% longer. Every timed cached warm process records exactly 15,360 hits and zero misses; changing/cold processes record zero hits and 15,360 misses. All final payload assertions pass. Moving, scaling or rotating geometry changes the absolute-coordinate key and misses.

Warm-curve allocation requests fall from 15,710,744 to 4,334,232 bytes/batch. Cold curves request 20,082,456 bytes versus 15,710,744 baseline. These are allocation requests rather than retained bytes or RSS.

[Raw CPU data](xps-path-cache.json), [summary](xps-path-cache.summary.json).

## Memory

Separate native processes use the same controls, with allocation instrumentation disabled and GNU time measuring peak RSS:

| Cached workload | Final retained payload | Peak process RSS |
| --- | ---: | ---: |
| Curves, 120 samples | 3,886,964 bytes | 10.54 MiB |
| Dashed, 120 samples | 2,781,764 bytes | 9.82 MiB |
| Triangles, 120 samples | 134,788 bytes | 6.98 MiB |
| Changing, 120 samples | 1,872,216 bytes | 15.59 MiB |
| Cold, 120 samples | 3,886,964 bytes | 10.87 MiB |
| Changing, 1,200 samples | 6,825,588 bytes | 16.98 MiB |

The longer changing run records 153,600 misses and zero hits, exercising repeated budget clears. All reported payloads remain below 8 MiB. The benchmark also retains its sample JSON in memory, so the longer run includes additional measurement storage.

The limit counts stored keys, path structures and vertex capacity. Hash-table/allocator overhead, returned meshes, temporary path construction and the new insertion clone before eviction are additional. `clear()` releases entries and table allocations to the allocator; RSS need not fall immediately. These measurements do not establish an 8 MiB process-memory cap or a general leak bound. [Raw controls and exact commands](memory.summary.json), `memory/`, and `measure-memory.py` preserve the evidence.

## Caller assessment

No ALA application caller currently uses `build_with_cache`. `paths_bench` already retains its demo meshes, so caching their construction would not demonstrate a steady-frame improvement. [Painting](https://github.com/autistic-computer/ala/blob/5e0a214dbb12cda890f62be68d4e3a852547acc2/crates/ala/examples/painting.rs) reconstructs user-drawn polylines during paint; completed strokes plus one growing active stroke are a plausible follow-up workload. Its native draw phase, hit rate and memory have not been measured with this API. Pan/zoom or other coordinate changes can erase reuse.

Recommend retaining the explicit opt-in contract and measuring a real caller before adoption. No application FPS, full draw-phase, GPU or power improvement is established here. Windows/MSVC and Apple native validation remain outstanding.
