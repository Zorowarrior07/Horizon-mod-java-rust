#!/usr/bin/env bash
# Which launch script does ModDevGradle generate, and what are its arguments?
out="$1"
{
  JAVA_HOME=/usr/lib/jvm/temurin-25-jdk-amd64 \
    gradle --no-daemon -q -p java/neoforge26 createServerLaunchScript 2>&1 | tail -3
  echo "===== generated script"
  find java/neoforge26/build -name '*erver*' -o -name '*launch*' 2>/dev/null | head -10
  f=$(find java/neoforge26/build -name '*server*' -type f 2>/dev/null | grep -v class | head -1)
  echo "--- $f"
  cat "$f" 2>/dev/null | tr ' ' '\n' | grep -iE 'mod|fml|asset|resources|args' | head -30
  echo
  echo "===== run config in created files"
  grep -rhoE '\-D[^ ]*' java/neoforge26/build/*.args 2>/dev/null | head -20
  ls java/neoforge26/build 2>/dev/null
} > "$out" 2>&1
