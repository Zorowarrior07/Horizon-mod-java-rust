#!/usr/bin/env bash
# How does ForgeGradle 2.1.2 obtain its MCP data, and is that source alive?
set -uo pipefail
out="$1"
echo "== ForgeGradle 2.1.2 jar" > "$out"
curl -sSL -o /tmp/fg21.jar \
  https://maven.minecraftforge.net/net/minecraftforge/gradle/ForgeGradle/2.1.2/ForgeGradle-2.1.2.jar \
  >> "$out" 2>&1
ls -l /tmp/fg21.jar >> "$out" 2>&1
rm -rf /tmp/fg21 && unzip -qo /tmp/fg21.jar -d /tmp/fg21 >> "$out" 2>&1
{
  echo "== URLs in the constant pool"
  grep -rao 'http[s]\?://[a-zA-Z0-9./_%-]*' /tmp/fg21 | sort -u | head -40
  echo
  echo "== MCP artefacts on maven.minecraftforge.net"
  for u in \
    https://maven.minecraftforge.net/de/oceanlabs/mcp/mcp_stable/22-1.8.9/mcp_stable-22-1.8.9.zip \
    https://maven.minecraftforge.net/de/oceanlabs/mcp/mcp/1.8.9/mcp-1.8.9-srg.zip \
    https://maven.minecraftforge.net/de/oceanlabs/mcp/mcp_snapshot/20160219-1.8.9/mcp_snapshot-20160219-1.8.9.zip ; do
    printf '  %s  %s\n' "$(curl -s -o /dev/null -w '%{http_code}' "$u")" "$u"
  done
  echo
  echo "== is mcpbot reachable at all?"
  for u in https://export.mcpbot.bspk.rs/versions.json http://export.mcpbot.bspk.rs/versions.json; do
    printf '  %s  %s\n' "$(curl -s -o /dev/null -m 15 -w '%{http_code}' "$u")" "$u"
  done
  echo
  echo "== fallback: files.minecraftforge.net mcp json"
  curl -s -o /dev/null -m 20 -w '  %{http_code}  http://files.minecraftforge.net/maven/de/oceanlabs/mcp/versions.json\n' \
    http://files.minecraftforge.net/maven/de/oceanlabs/mcp/versions.json
} >> "$out" 2>&1
