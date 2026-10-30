# CPU pipeline evidence

This directory preserves the evidence used by the CPU profiling/toolkit draft
and the separate style-borrowing draft. It is independent of the opacity replay,
segmented shaping, and retained-layout experiments in the parallel fork survey.

`baseline/` contains two same-binary CPU runs, a software-rendering smoke
measurement, a diagnostic phase run, profiles, environment information, and
validation logs. The [assessment](../../docs/performance-report.md) explains
their scope and limitations. A synthetic redraw is not application frame rate.

Each `runs/*.json` file stores the original metadata and summary plus Criterion's
raw `sample`, `estimates`, `benchmark`, and `tukey` records for each case. The
adjacent `.log` retains the executable's output with display whitespace normalized.
Metadata records the original
base commit, worktree fingerprint, compiler/features/flags, binary hash,
affinity/quota, and selected runtime settings. The captured worktree predates
the publishing commits; later documentation/packaging changes do not relabel
those historical measurements as tests of newly published commit hashes.

Profile JSON preserves every sampled leaf's weight and percentage, total
CPU-clock weight, capture metadata, and sample/loss counts. Sampling includes
process setup. Those shares do not count allocations or copied bytes.

Compiled executables, `perf.data`, large folded stacks, full frame JSON streams,
and SVG captures are excluded from Git. They remain in the original cloud
workspace and can be regenerated with [the toolkit](../../docs/performance.md).
Absolute paths and the environment script mentioned in historical logs describe
that session, not prerequisites for a native-machine reproduction.

Use [native validation](../../docs/performance-validation.md) for current,
portable reproduction commands and the Linux/macOS/Windows validation gates.

`borrow-styles/` contains the follow-up optimization's six confirmation runs,
paired summaries, original/candidate build metadata, rejected inlining screens,
profile/work-count evidence, and validation logs. The
[experiment report](../../docs/performance-experiment.md) interprets those results.
