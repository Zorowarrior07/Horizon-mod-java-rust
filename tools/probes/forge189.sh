#!/usr/bin/env bash
# Forge 1.8.9: ForgeGradle 2.1.2 is the only toolchain that supports it, and it
# fetches its mapping list from export.mcpbot.bspk.rs — a service that no longer
# exists. This probe checks whether the mapping *artifacts* still exist, because
# if they do, a local stand-in for versions.json (via /etc/hosts) is enough.
set -uo pipefail
out="$1"
{
  echo "== MCP artefacts for 1.8.9 stable_22"
  for u in \
    https://maven.minecraftforge.net/de/oceanlabs/mcp/mcp_stable/22-1.8.9/mcp_stable-22-1.8.9.zip \
    https://maven.minecraftforge.net/de/oceanlabs/mcp/mcp/1.8.9/mcp-1.8.9-srg.zip \
    https://maven.minecraftforge.net/de/oceanlabs/mcp/versions.json ; do
    printf '  %s  %s\n' "$(curl -s -o /dev/null -m 20 -w '%{http_code}' "$u")" "$u"
  done
  echo
  echo "== forge universal for 1.8.9"
  printf '  %s\n' "$(curl -s -o /dev/null -m 20 -w '%{http_code}' \
    https://maven.minecraftforge.net/net/minecraftforge/forge/1.8.9-11.15.0.1677/forge-1.8.9-11.15.0.1677-universal.jar)"
} > "$out" 2>&1
