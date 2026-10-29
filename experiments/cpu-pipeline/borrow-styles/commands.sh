#!/bin/sh
set -eu
. /workspace/.ala-perf/env.sh
cd /workspace/ala
taskset -c 0 python3 tools/perf.py run --variant target/perf-results/style-original --label borrow-a1 > /tmp/ala-borrow-a1.log 2>&1
taskset -c 0 python3 tools/perf.py run --variant target/perf-results/style-borrowed --label borrow-b1 > /tmp/ala-borrow-b1.log 2>&1
python3 tools/perf.py compare target/perf-results/borrow-a1 target/perf-results/borrow-b1 > target/perf-results/borrow-experiment/pair-1.txt
taskset -c 0 python3 tools/perf.py run --variant target/perf-results/style-borrowed --label borrow-b2 > /tmp/ala-borrow-b2.log 2>&1
taskset -c 0 python3 tools/perf.py run --variant target/perf-results/style-original --label borrow-a2 > /tmp/ala-borrow-a2.log 2>&1
python3 tools/perf.py compare target/perf-results/borrow-a2 target/perf-results/borrow-b2 > target/perf-results/borrow-experiment/pair-2.txt
taskset -c 0 python3 tools/perf.py run --variant target/perf-results/style-original --label borrow-a3 > /tmp/ala-borrow-a3.log 2>&1
taskset -c 0 python3 tools/perf.py run --variant target/perf-results/style-borrowed --label borrow-b3 > /tmp/ala-borrow-b3.log 2>&1
python3 tools/perf.py compare target/perf-results/borrow-a3 target/perf-results/borrow-b3 > target/perf-results/borrow-experiment/pair-3.txt
