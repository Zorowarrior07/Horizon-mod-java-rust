#!/usr/bin/env bash
# ============================================================================
# Runtime test: boot a real dedicated Minecraft server with the mod loaded and
# check that the game itself accepted our registrations.
#
# This is the difference between "it compiled" and "it works". A jar that
# compiles but whose entrypoint throws, or whose block id is rejected, fails
# this test — which is exactly what we want it to do.
#
# Usage:  tools/runtime-test.sh <project dir> <loader> <log path> [gradle args...]
#
# Success is decided from the actual server log, not from an exit code: the
# server is stopped deliberately once it reaches "Done", so the exit code is not
# meaningful, but the log is.
# ============================================================================
set -uo pipefail

project="${1:?project dir}"; loader="${2:?loader}"; log="${3:?log path}"
shift 3 || true
args=("$@")

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$(dirname "$log")"

echo "===== $loader runtime test: $project" > "$log"

# EULA: accepting it is required to start a dedicated server. This is a
# throwaway server in a build sandbox, used for one boot and then discarded.
server_dir="$root/$project/run"
mkdir -p "$server_dir"
printf 'eula=true\n' > "$server_dir/eula.txt"
printf 'level-type=flat\nonline-mode=false\nmax-players=1\nview-distance=4\n'
printf 'level-type=flat\nonline-mode=false\nmax-players=1\nview-distance=4\n' > "$server_dir/server.properties"

# Run the server task with a timeout: it never exits on its own.
timeout 300 gradle --no-daemon -p "$root/$project" runServer "${args[@]}" >> "$log" 2>&1
echo "gradle exit=$? (expected: 124 for the timeout, or non-zero if stopped)" >> "$log"

# ---- verdict, read from the log itself ----
verdict=0
check() {
  if grep -qiE "$1" "$log"; then
    echo "  OK    $2" | tee -a "$log"
  else
    echo "  FAIL  $2" | tee -a "$log"
    verdict=1
  fi
}

echo "----- verdict" | tee -a "$log"
check 'Done \(.*\)! For help' 'server reached "Done" (the world loaded)'

case "$loader" in
  fabric)
    check 'Loading [0-9]+ mods' 'Fabric loaded the mod list'
    check 'zoro_bridge' 'Fabric knows the mod id zoro_bridge'
    check 'bridge_beacon \+ bridge_chip registered' 'our onInitialize ran and registered both objects'
    ;;
  neoforge)
    check 'zoro_bridge' 'NeoForge knows the mod id zoro_bridge'
    check 'NeoForge 2[16]\.x build initialised|loaded on' 'our mod constructor ran'
    ;;
esac

# The registry accepting the ids shows up as no "Unknown registry" / "Failed to
# register" errors; a bad registration aborts startup before "Done".
if grep -qiE 'Failed to (register|create)|Unknown registry|Invalid registry|Exception in thread "main"' "$log"; then
  echo "  FAIL  a registration error was logged" | tee -a "$log"
  verdict=1
fi

echo "----- result: $([ $verdict -eq 0 ] && echo PASS || echo FAIL)" | tee -a "$log"
exit $verdict
