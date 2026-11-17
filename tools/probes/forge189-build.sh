#!/usr/bin/env bash
# Build the Forge 1.8.9 mod.
#
# ForgeGradle 2.1.2 is the only toolchain that supports 1.8.9, and it asks
# export.mcpbot.bspk.rs for a mapping list at configuration time. MCPBot was
# retired, so that host is dead — but the mapping *artifacts* it points at are
# still on maven.minecraftforge.net, and Forge maven still serves a versions.json
# in exactly the schema ForgeGradle expects.
#
# So: fetch that real file, serve it on 127.0.0.1:80, and point the dead hostname
# at loopback. Nothing is faked — the same bytes ForgeGradle would have fetched.
set -uo pipefail
out="${1:?output log path}"
repo_java="${REPO_JAVA:?REPO_JAVA must point at <repo>/java}"
artifact_dir="${ARTIFACT_DIR:-$repo_java/../artifacts/java-1.8.9}"

dir=/tmp/fg189
rm -rf "$dir" /tmp/mcpsrv
mkdir -p "$dir" /tmp/mcpsrv "$artifact_dir"

echo "===== fetching the real versions.json" > "$out"
if ! curl -sSf -o /tmp/mcpsrv/versions.json \
      https://maven.minecraftforge.net/de/oceanlabs/mcp/versions.json >> "$out" 2>&1; then
  echo "FATAL: could not fetch versions.json from Forge maven" >> "$out"
  exit 1
fi
head -c 200 /tmp/mcpsrv/versions.json >> "$out"
echo >> "$out"

# Point the retired hostname at loopback and serve the file there.
if ! grep -q 'export.mcpbot.bspk.rs' /etc/hosts 2>/dev/null; then
  echo "127.0.0.1 export.mcpbot.bspk.rs" | sudo tee -a /etc/hosts > /dev/null
fi
( cd /tmp/mcpsrv && sudo python3 -m http.server 80 > /tmp/mcpsrv.log 2>&1 & )
sleep 2
printf 'local mcpbot now returns: %s\n' \
  "$(curl -s -o /dev/null -m 10 -w '%{http_code}' http://export.mcpbot.bspk.rs/versions.json)" >> "$out"

# The project: real sources and resources, copied so the build tree stays clean.
mkdir -p "$dir/src/main"
cp -r "$repo_java"/forge189/src/main/java "$dir/src/main/" 2>/dev/null || true
cp -r "$repo_java"/forge189/src/main/resources "$dir/src/main/" 2>/dev/null || true
cp "$repo_java"/forge189/build.gradle "$repo_java"/forge189/settings.gradle "$dir/" 2>/dev/null || true

# A JDK 8 JVM for Gradle 4.x / 2.x, whatever its directory is called.
jdk8=$(ls -d /usr/lib/jvm/*8* 2>/dev/null | head -1)
echo "JDK8=$jdk8" >> "$out"
if [ -z "$jdk8" ]; then
  echo "FATAL: no JDK 8 found" >> "$out"
  exit 1
fi

cd "$dir"
PASSED=0
for ver in 4.10.3 2.14.1; do
  if [ ! -d "/opt/gradle-$ver" ]; then
    curl -sSL -o "/tmp/gradle-$ver.zip" \
      "https://services.gradle.org/distributions/gradle-$ver-bin.zip" >> "$out" 2>&1
    unzip -qo "/tmp/gradle-$ver.zip" -d /opt >> "$out" 2>&1
  fi
  echo "===== Gradle $ver" >> "$out"
  if JAVA_HOME="$jdk8" "/opt/gradle-$ver/bin/gradle" --no-daemon build --stacktrace \
       >> "$out" 2>&1; then
    echo "SUCCESS with Gradle $ver" >> "$out"
    cp build/libs/*.jar "$artifact_dir"/ 2>/dev/null
    PASSED=1
    break
  fi
  echo "FAILED with Gradle $ver" >> "$out"
done

# Leave the machine as it was found: drop the hosts entry and the server.
sudo sed -i '/export.mcpbot.bspk.rs/d' /etc/hosts 2>/dev/null || true
sudo pkill -f 'http.server 80' 2>/dev/null || true

ls -la "$artifact_dir" >> "$out" 2>&1
exit $((1 - PASSED))
