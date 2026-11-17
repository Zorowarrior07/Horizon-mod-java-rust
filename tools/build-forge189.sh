#!/usr/bin/env bash
# ============================================================================
# ZOROWARRIOR HORIZON BRIDGE — build the Forge 1.8.9 mod jar.
#
# Why this is not a one-line `gradle build`:
#
#   * Forge 1.8.9 needs ForgeGradle 2.1.2, which only runs on Gradle 4.x / 2.x
#     and needs a real JDK 8. (ForgeGradle 2.3 refuses 1.8.9 outright.)
#   * ForgeGradle 2.1.2 asks export.mcpbot.bspk.rs for its mapping list when the
#     project is configured. That host has been dead for years, so every
#     "1.8.9 build" recipe on the internet now fails with
#     "Unable to obtain url ... with etag!".
#
# The mappings themselves were never lost: Forge's own maven still serves
# de/oceanlabs/mcp/mcp_stable/22-1.8.9/mcp_stable-22-1.8.9.zip, and it still
# serves a versions.json in exactly the schema ForgeGradle parses. So this
# script fetches that genuine file, serves it on loopback, and points the retired
# hostname at loopback for the duration of the build. Nothing is faked — the same
# bytes ForgeGradle would have downloaded in 2016 — and the hosts entry is
# removed again on exit.
#
# Usage:  tools/build-forge189.sh
# Output: artifacts/java-1.8.9/zoro_bridge-1.0.0-forge-1.8.9.jar
# ============================================================================
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/artifacts/java-1.8.9"
log="$root/ci/logs/forge-1.8.9.log"
mkdir -p "$out" "$(dirname "$log")"

jdk8="${JAVA_HOME_8_X64:-$(ls -d /usr/lib/jvm/*8* 2>/dev/null | head -1)}"
if [ -z "${jdk8:-}" ] || [ ! -x "$jdk8/bin/javac" ]; then
  echo "error: a JDK 8 is required for ForgeGradle 2.1 (set JAVA_HOME_8_X64)" >&2
  exit 2
fi

work="$(mktemp -d)"
trap 'sudo sed -i "/export.mcpbot.bspk.rs/d" /etc/hosts 2>/dev/null; \
      sudo pkill -f "http.server 80" 2>/dev/null; rm -rf "$work"' EXIT

# 1. A genuine versions.json, served on loopback under the retired hostname.
mkdir -p "$work/srv"
curl -sSf -o "$work/srv/versions.json" \
  https://maven.minecraftforge.net/de/oceanlabs/mcp/versions.json
grep -q 'export.mcpbot.bspk.rs' /etc/hosts 2>/dev/null || \
  echo '127.0.0.1 export.mcpbot.bspk.rs' | sudo tee -a /etc/hosts > /dev/null
( cd "$work/srv" && sudo python3 -m http.server 80 > /dev/null 2>&1 & )
sleep 2

# 2. Gradle 4.10.3, the newest that ForgeGradle 2.1 tolerates.
if [ ! -d "$work/gradle-4.10.3" ]; then
  curl -sSL -o "$work/gradle.zip" \
    https://services.gradle.org/distributions/gradle-4.10.3-bin.zip
  unzip -qo "$work/gradle.zip" -d "$work"
fi

# 3. Copy the project so the build tree is disposable, then build.
cp -r "$root/java/forge189" "$work/project"
( cd "$work/project" && JAVA_HOME="$jdk8" "$work/gradle-4.10.3/bin/gradle" \
    --no-daemon build --stacktrace ) > "$log" 2>&1
rc=$?

if [ $rc -ne 0 ]; then
  echo "BUILD FAILED — see $log" >&2
  grep -nE 'error:|What went wrong|Caused by:' "$log" | head -20 >&2
  exit $rc
fi

cp "$work/project"/build/libs/*.jar "$out"/
rm -f "$out"/*-sources.jar
python3 "$root/tools/verify_jar.py" "$out"/*.jar || exit 1
echo "built: $out"
