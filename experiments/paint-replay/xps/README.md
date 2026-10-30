# XPS native validation, 2026-10-29

**Verdict: Linux correctness and CPU benefit validated. GPU timestamp spans regress under automatic clocks; no GPU speed or power benefit is established. Keep the prototype opt-in and the PR draft.**

Tested source: `bbb6a3ff8d1afbf4a1bf1cc7b2998eecf464ea48`, including the live harness change that reports its selected GPU. The original PR head was `9b905de70b8d7956ea3f801977997da3081cda5f`. Baseline and replay run from the same executable, with allocation instrumentation disabled during timing. The main checkout was newly cloned and clean; this branch has its own worktree. Existing desktop work was preserved.

Machine: Dell XPS 13 9300, Intel i7-1065G7 (4 cores/8 threads), about 14.7 GiB usable RAM, Intel Iris Plus G7 (ICL GT2), i915 and Mesa 26.0.8. Ubuntu 26.04.1, kernel 7.0.0-42-generic, GNOME Wayland, 1920×1200 at 59.950 Hz, 125% scale, AC power and performance mode. Every live run reports `is_software_emulated: false` and the Intel GPU. Actual Intel Vulkan command traces confirm hardware execution. Rust 1.99.0 and Python 3.14.4; full details and commands are in [machine.json](machine.json), [build-environment.json](build-environment.json) and [commands.txt](commands.txt).

## Correctness and CPU fixture

All 24 changing scenes/hitboxes match, all 46,080,000 pixels match across 17 distinct images, and both cached input-dispatch checks pass. Linux reports `pixel_checks_available: true`. All 605 release core tests pass with `bench-support,perf-experiments`; formatting passes. The harness now prints the actual device, driver and software-emulation status for every live run.

The CPU fixture times main-thread update/draw/arena-clear work without GPU submission. Five alternating process repetitions × 120 samples, pinned to CPU 2, with separate allocation passes:

| Rows | Baseline median | Replay median | Ratio |
| --- | ---: | ---: | ---: |
| 30 | 0.4352 ms | 0.1188 ms | 3.66× |
| 120 | 0.8697 ms | 0.1403 ms | 6.20× |
| 480 | 2.6639 ms | 0.2346 ms | 11.35× |

[Verification](verify-xps.json), [raw CPU samples](xps-paint-replay.json), [CPU summary](xps-paint-replay.summary.json), [core test log](core-tests-release.log.gz). A debug test build briefly overlapped an earlier timing command's build phase and was interrupted before samples began. That attempt is preserved separately. Reported results use the subsequent clean run, with no concurrent builds.

## Native frame behavior

Five baseline/replay pairs ran sequentially for 60 seconds per window, followed by a 60-second static control. Both modes use the same 800×600 logical window at 125% scale and the same fonts/build options. Accessibility remained enabled in every steady capture. Exclude the first second; values below are medians across the five per-run summaries.

| Measurement | Baseline | Replay |
| --- | ---: | ---: |
| CPU draw median / p95 | 4.273 / 4.759 ms | 0.846 / 1.026 ms |
| CPU present-submit median / p95 | 0.703 / 1.043 ms | 0.627 / 0.906 ms |
| Views rendered / layout nodes per steady frame | 2 / 967 | 1 / 4 |
| Draw cadence | 59.82 Hz | 59.84 Hz |
| Process CPU seconds per 60-second run, including startup | 19.34 s | 6.71 s |
| Peak RSS | 113.8 MiB | 113.2 MiB |

CPU draw improves 5.05×; paired ratios range 4.41–5.29×. Both modes remain display limited. `draw` and `present_submit` measure CPU work/blocking. They do not measure GPU execution or physical scanout.

Both modes build all 967 layout nodes on the first draw. First-draw medians are 23.61 ms baseline and 21.83 ms replay; these small startup samples do not establish a cold-start benefit. The CPU fixture measures warm, unchanged pane contents while opacity changes. Structural/content invalidation passes correctness checks; performance with ongoing pane edits was not measured.

Static emits five startup draws, the last at 88.9 ms, and zero draws after the first second during the full minute. It stops requesting animation frames. Two native desktop observations show the replay fade changing only within the effect's 175×30-pixel region; rows below it match exactly. [Visual check](visual-check.json) and the two PNGs contain only crops of the observed fixture region; these are desktop crops, not attested window captures.

## Actual GPU profiling and regression

Separate 60-second captures use [Mesa u_trace](https://docs.mesa3d.org/u_trace.html). The summary unions GPU batch timestamp spans, so nested draw/render-pass events are not double-counted. CPU frame results above come from the unprofiled captures.

| Capture order | Baseline GPU span median / p95 | Replay GPU span median / p95 |
| --- | ---: | ---: |
| Baseline, then replay | 2.768 / 3.541 ms | 3.571 / 3.935 ms |
| Replay, then baseline | 2.738 / 3.519 ms | 3.543 / 3.866 ms |

Replay's measured GPU spans are about 29% longer in both orders. Actual command topology is unchanged: 51 draws, 7 command buffers, 1 render pass and 4 BLORP operations per steady frame, with identical shaders and ordering. Replay draws 4,020 vertices versus 4,028 because its visible mode label has two fewer glyphs, about 0.20% less vertex work.

The reverse pair samples system-wide GPU frequency every 47 ms. Median requested frequency is 400 MHz for baseline and 300 MHz for replay. Actual-frequency distributions differ, and throttle status is nonzero in 174/1,246 baseline samples and 200/1,246 replay samples. These observations support a clock/power-state explanation; they do not isolate the cause or replace a controlled-clock comparison. No clock or driver settings were changed, and no power measurements were taken. Retain the measured GPU-duration regression in adoption decisions.

The GPU static control completes three startup GPU frames within 61.6 ms of its first GPU timestamp, with no steady GPU activity over the remaining minute. Mesa leaves one unfinished final frame record at shutdown (141–144 bytes); raw output is preserved and only complete frames are analyzed. Each active trace contains about 3,590 complete GPU frames. System policy blocks `perf` counters (`perf_event_paranoid=4`); it was left unchanged.

[Frame/GPU summary](live.summary.json), [GPU command analysis](gpu.topology.json), [frequency summary](gpu.frequency.summary.json). Raw NDJSON, driver JSON and frequency samples are committed as gzip files under `live/`, `gpu/` and `gpu-reverse/`; manifests save exact commands, binary hashes, power mode and temperatures. Local uncompressed copies are preserved and ignored. `collect-live.py`, `summarize-live.py` and `gpu-topology.py` reproduce collection and analysis.

This validates the unvirtualized opacity fixture on this Linux machine. Caller-level invalidation/lifetime review and representative application measurements remain necessary. Windows/MSVC/DirectWrite and Apple/Metal native measurements remain outstanding.
