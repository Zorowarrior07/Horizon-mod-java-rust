#!/usr/bin/env bash
# ModDevGradle offers several dev-mod variants. Which one does runServer use, and
# which directories does it read metadata from?
out="$1"
{
  echo "===== moddev tasks mentioning run or mods"
  gradle --no-daemon -q -p java/neoforge26 tasks --all 2>/dev/null \
    | grep -iE 'run|mod.*jar|devJar' | head -20
  echo
  echo "===== RunGameTask / ModDevGradle extension properties"
  gradle --no-daemon -q -p java/neoforge26 properties 2>/dev/null \
    | grep -iE 'neoForge|moddev' | head -20
} > "$out" 2>&1
