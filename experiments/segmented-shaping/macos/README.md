# Apple Silicon validation, 2026-10-29

Tested source: `a665851306cd5e882fcc7163de633f2544246b2c`. The evidence commit only adds this report. Both published-source CI jobs passed; see `tested-source-ci.json`.

MacBook Pro Mac16,5, Apple M4 Max (12 performance + 4 efficiency CPU cores, 40 GPU cores), 48 GiB RAM; macOS 27.2 (26B123), Xcode 27.0, Rust 1.99.0, Python 3.14.10. AC power, automatic power mode, no initial thermal/performance warning. Display: 1920 x 1080 at 60 Hz, scale 1 in the native fixture. This is a shared workstation with background services active; no task-owned compilation or profiling overlapped these timing passes. CPU affinity was not pinned on macOS.

Five alternating process repetitions, 120 samples per repetition and workload, with allocation counting in separate processes. Table values are medians of process medians. Raw samples, all controls, source/compiler/binary hashes and verification are in `cpu.json.gz`; `cpu.summary.json` is readable without decompression. These are local baseline/candidate comparisons, not comparisons against Linux wall times.

**Verdict: blocked on correctness on CoreText too, and the measured update workload is slower. Do not adopt.**

The 80-paragraph corpus has 0 differing wraps in 400 comparisons, but complete line layouts differ and maximum horizontal glyph drift is 1.25 px. Glyph identities match. The repeated ASCII probe drifts by 1.10546875 px; baseline/segmented widths are 40320.2890625/40319.1796875 px. All ten fallback cases, direct font invalidation and warm-cache reuse assertions pass. Overall `layout_equivalent` is false. Successful process completion means the diagnostics completed. Pixel equivalence remains unestablished.

The verifier explicitly uses bundled IBM Plex Sans at 16 px. Timing uses the existing Paragraph fixture and its inherited platform default font; its bundled font registration does not force the Paragraph font. Native CoreText and Linux fallback-font timing are therefore not controlled font comparisons across machines.

Diagnostic timings compare unequal layouts:

| Bytes/workload | Baseline ms | Segmented ms | Ratio | Segmented time change |
| --- | ---: | ---: | ---: | ---: |
| 2000/append | 0.0761 | 0.0775 | 0.98x | +1.8% |
| 2000/middle | 0.0684 | 0.0711 | 0.96x | +3.9% |
| 2000/cold | 0.0699 | 0.0806 | 0.87x | +15.3% |
| 2000/unchanged | 0.0535 | 0.0536 | 1.00x | +0.2% |
| 20000/append | 0.2549 | 0.2666 | 0.96x | +4.6% |
| 20000/middle | 0.2510 | 0.2614 | 0.96x | +4.2% |
| 20000/cold | 0.2567 | 0.3325 | 0.77x | +29.5% |
| 20000/unchanged | 0.1285 | 0.1285 | 1.00x | -0.0% |
| 100000/append | 1.0982 | 1.1541 | 0.95x | +5.1% |
| 100000/middle | 1.0940 | 1.1536 | 0.95x | +5.5% |
| 100000/cold | 1.1002 | 1.5049 | 0.73x | +36.8% |
| 100000/unchanged | 0.4519 | 0.4523 | 1.00x | +0.1% |

The first original-corpus position mismatch is generator seed 2, byte 221: baseline x 2073.5966796875 px, segmented x 2073.596435546875 px. Later prefix differences accumulate; the maximum is generator seed 80, byte 4922: 46110.03125 versus 46108.78125 px. Complete text/cut/float diagnostics are preserved in the raw verification.

Append/middle updates are about 2-5% slower, cold workloads 15-37% slower, and unchanged text is roughly equal. All 144 processes remain within the 262,144 retained-glyph bound. Reduced shaped-byte counts do not establish an end-to-end improvement. This run does not test external font-generation changes, broader typography or caret/selection correctness. The layout contract needs a backend-level solution before adoption; Windows/DirectWrite validation remains outstanding.
