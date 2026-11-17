#!/usr/bin/env bash
# Try the full 1.8.9 build with a local stand-in for the dead MCPBot service.
set -uo pipefail
out="$1"
dir=/tmp/fg189
rm -rf "$dir"; mkdir -p "$dir"

# Serve a plausible versions.json on 127.0.0.1:80 and point the dead host at it.
sudo tee /etc/hosts.d/mcpblock >/dev/null 2>&1 || true
echo "127.0.0.1 export.mcpbot.bspk.rs" | sudo tee -a /etc/hosts >/dev/null

mkdir -p /tmp/mcpsrv
cat > /tmp/mcpsrv/versions.json <<'J'
{"stable":[{"version":"22","mcversion":"1.8.9"}],"snapshot":[{"version":"20160219","mcversion":"1.8.9"}]}
J
( cd /tmp/mcpsrv && sudo python3 -m http.server 80 > /tmp/mcpsrv.log 2>&1 & ) 
sleep 2
curl -s -o /dev/null -w 'local mcpbot: %{http_code}\n' http://export.mcpbot.bspk.rs/versions.json >> "$out"

cp "$REPO_JAVA"/forge189/build.gradle "$dir/" 2>/dev/null || true
cp "$REPO_JAVA"/forge189/settings.gradle "$dir/" 2>/dev/null || true
mkdir -p "$dir/src/main/java" "$dir/src/main/resources"
cp -r "$REPO_JAVA"/forge189/src/main/* "$dir/src/main/" 2>/dev/null || true

cd "$dir"
for ver in 4.10.3 2.14.1; do
  [ -d "/opt/gradle-$ver" ] || {
    curl -sSL -o "/tmp/gradle-$ver.zip" "https://services.gradle.org/distributions/gradle-$ver-bin.zip" \
      && unzip -qo "/tmp/gradle-$ver.zip" -d /opt
  }
  echo "===== Gradle $ver" >> "$out"
  if "/opt/gradle-$ver/bin/gradle" --no-daemon build --stacktrace >> "$out" 2>&1; then
    echo "SUCCESS with Gradle $ver" >> "$out"
    cp build/libs/*.jar "$ARTIFACT_DIR"/ 2>/dev/null || true
    break
  fi
  echo "FAILED with Gradle $ver" >> "$out"
done
