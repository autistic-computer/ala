# cosmic-text snapshot

This directory contains cosmic-text 0.19.0 from crates.io, upstream commit
`c24886c2471e5606587c46090cd25dbbf209186b` in
[pop-os/cosmic-text](https://github.com/pop-os/cosmic-text).
The package uses the MIT or Apache-2.0 license; both license files remain here.

The snapshot includes the library source, README and a standalone manifest.
It excludes upstream examples, benchmarks, test assets and development dependencies.
Our cache tests use the existing ala Lilex fixture.

The local patch partitions the existing six-entry shaping-plan FIFO by font face.
Each font keeps the same plan matching rules for script, direction, language,
features and variation settings. A fallback search through many fonts no longer
evicts the primary face's plans. FontSystem owns the cache and drops it with the
text system. The cache holds at most six plans per shaped face and clears before
mutable database access, including font additions and removals.

The Linux mixed-text probe exercised 2,125 distinct face/script/feature keys.
Partitioning reduced plan construction from 246,122 to 2,125 for 1,024 lines,
with identical glyph output, and retained about 2.5 MB more allocations on that
machine. The earlier global six-to-64 FIFO experiment did not improve throughput.

The cache reserves each additional slot explicitly instead of using geometric
growth. This preserves the six-plan limit and avoids unused capacity. Separate
425-face fallback probes saved about 0.43 MiB of requested allocations, with
identical shaped output and no consistent CPU penalty in 15 alternating pairs.
Ordinary one-face text saved only 704 bytes. See the RAM follow-up in
`knowledge/research/2026-10-16-linux-performance.md` for workloads and limits.

To refresh this snapshot, diff the library source against the pinned release,
apply the cache, exact-growth and database-invalidation changes, and run the
plan cache tests plus the ala platform text tests. Remove this snapshot when an
upstream release provides equivalent bounded reuse or another measured shaping
implementation.
