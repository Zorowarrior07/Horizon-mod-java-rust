#!/usr/bin/env bash
# What URL/schema does ForgeGradle 2.1.2 use for the (dead) MCP metadata, and
# what does the maven-hosted versions.json actually look like?
set -uo pipefail
out="$1"
{
  echo "== maven-hosted versions.json (first 300 bytes)"
  curl -s https://maven.minecraftforge.net/de/oceanlabs/mcp/versions.json | head -c 300
  echo
  echo
  echo "== ForgeGradle 2.1.2 strings around mcp"
  rm -rf /tmp/fg21s && mkdir -p /tmp/fg21s
  unzip -qo /tmp/fg21.jar -d /tmp/fg21s 2>/dev/null || \
    curl -sSL -o /tmp/fg21.jar https://maven.minecraftforge.net/net/minecraftforge/gradle/ForgeGradle/2.1.2/ForgeGradle-2.1.2.jar && \
    unzip -qo /tmp/fg21.jar -d /tmp/fg21s
  grep -rl 'mcpbot' /tmp/fg21s | head -5
  for f in $(grep -rl 'mcpbot' /tmp/fg21s | head -3); do
    echo "--- $f"
    strings "$f" | grep -iE 'mcp|json|http' | head -25
  done
} > "$out" 2>&1
