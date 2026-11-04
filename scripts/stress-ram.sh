#!/usr/bin/env bash
# Clean RAM stress test (development builds only), the Linux twin of
# stress-ram.ps1.
#
# Builds a development copy of RustCleanup, then has it fill RAM with helper
# processes holding incompressible data (so zram/zswap can't shrink it) and
# time Clean RAM against them, several runs in a row. Each run prints how much
# the cleanup freed and how long paging out and dropping the cache took.
#
# Run it with sudo to test the real thing. As a normal user only your own
# processes are paged out and --purge (drop the page cache) is skipped.
#
#   scripts/stress-ram.sh
#   scripts/stress-ram.sh --size-mb 4096 --hogs 8 --runs 10 --purge --csv stress.csv
#   scripts/stress-ram.sh --release   # optimised build with the stress-test feature
set -u
SIZE_MB=2048 HOGS=4 RUNS=5 PURGE="" CSV="" RELEASE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --size-mb) SIZE_MB="$2"; shift 2 ;;
        --hogs) HOGS="$2"; shift 2 ;;
        --runs) RUNS="$2"; shift 2 ;;
        --purge) PURGE=1; shift ;;
        --csv) CSV="$2"; shift 2 ;;
        --release) RELEASE=1; shift ;;
        -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
        *) echo "Unknown option: $1" >&2; exit 2 ;;
    esac
done

cd "$(dirname "$0")/.." || exit 1
if [ -n "$RELEASE" ]; then
    cargo build --release --features stress-test || exit 1
    exe=target/release/RustCleanup
else
    cargo build || exit 1
    exe=target/debug/RustCleanup
fi

cli=(--stress-ram --size-mb "$SIZE_MB" --hogs "$HOGS" --runs "$RUNS")
[ -n "$PURGE" ] && cli+=(--purge)
[ -n "$CSV" ] && cli+=(--csv "$CSV")

if [ "$(id -u)" -ne 0 ]; then
    echo "Warning: not root, so only your own processes are paged out and the page cache is not dropped." >&2
fi
echo "Running $exe ${cli[*]}"
"$exe" "${cli[@]}"
code=$?
echo "Stress test exited with code $code"
exit $code
