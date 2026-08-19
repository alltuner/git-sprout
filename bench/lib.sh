# ABOUTME: Timing, real-disk accounting and machine-fact helpers shared by every
# ABOUTME: git-sprout benchmark scenario.

# Real disk consumed is measured as a free-space delta on the volume that holds the
# worktree, because the whole point of the tool is that logical size (du, ls) stays
# the same while allocation does not.

die() { printf 'bench: %s\n' "$*" >&2; exit 1; }
note() { printf '  %s\n' "$*" >&2; }

avail_kb() { df -k "$1" | awk 'NR==2 {print $4}'; }

# Free space is sampled until two consecutive readings agree within 2 MB, because
# APFS keeps accounting for a while after a large write.
settle_kb() {
  local path="$1" prev cur i
  sync
  prev=$(avail_kb "$path")
  for i in 1 2 3 4 5 6 7 8 9 10 11 12; do
    sleep 0.5
    cur=$(avail_kb "$path")
    if [ $(( prev > cur ? prev - cur : cur - prev )) -lt 2048 ]; then
      echo "$cur"; return
    fi
    prev=$cur
  done
  echo "$cur"
}

fs_type() {
  local path="$1"
  case "$(uname -s)" in
    Linux) df -T "$path" | awk 'NR==2 {print $2}' ;;
    Darwin) mount | awk -v d="$(df "$path" | awk 'NR==2 {print $1}')" \
              '$1 == d { gsub(/[(,]/, "", $4); print $4; exit }' ;;
    *) echo unknown ;;
  esac
}

# time_cmd <cwd> <cmd...> -> elapsed seconds on stdout. A single python process
# wraps the command so process-spawn overhead never lands inside the measurement.
time_cmd() {
  python3 -c '
import subprocess, sys, time
cwd, cmd = sys.argv[1], sys.argv[2:]
start = time.monotonic()
proc = subprocess.run(cmd, cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
elapsed = time.monotonic() - start
if proc.returncode != 0:
    sys.stderr.write(proc.stderr.decode("utf-8", "replace"))
    sys.exit(proc.returncode)
print(f"{elapsed:.4f}")
' "$@"
}

# measure <volume> <cwd> <cmd...> -> sets MEASURE_TIME_S and MEASURE_DISK_MB
measure() {
  local vol="$1"; shift
  local before after
  before=$(settle_kb "$vol")
  MEASURE_TIME_S=$(time_cmd "$@") || return 1
  after=$(settle_kb "$vol")
  MEASURE_DISK_MB=$(python3 -c "print(f'{($before - $after) / 1024:.2f}')")
}

# record k=v ... appends one NDJSON object to $BENCH_RAW. A `key:=value` pair is
# emitted as raw JSON so numbers, booleans and arrays keep their type.
record() {
  python3 -c '
import json, sys
out = {}
for arg in sys.argv[1:]:
    key, _, value = arg.partition("=")
    if key.endswith(":"):
        out[key[:-1]] = json.loads(value)
    else:
        out[key] = value
print(json.dumps(out))
' "$@" >> "$BENCH_RAW"
}

json_num() { python3 -c "print(f'{float('$1'):.4f}')"; }

cpu_model() {
  case "$(uname -s)" in
    Darwin) sysctl -n machdep.cpu.brand_string ;;
    Linux) awk -F': ' '/model name/ {print $2; exit}' /proc/cpuinfo 2>/dev/null || uname -m ;;
    *) uname -m ;;
  esac
}

cpu_cores() {
  case "$(uname -s)" in
    Darwin) sysctl -n hw.ncpu ;;
    *) nproc 2>/dev/null || echo 0 ;;
  esac
}

os_name() {
  case "$(uname -s)" in
    Darwin) echo "$(sw_vers -productName) $(sw_vers -productVersion)" ;;
    Linux) . /etc/os-release 2>/dev/null && echo "$PRETTY_NAME" || uname -sr ;;
    *) uname -sr ;;
  esac
}
