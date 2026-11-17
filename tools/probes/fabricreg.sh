#!/usr/bin/env bash
# Fabric API's registry modules: what can a mod use to add a block after the
# vanilla registry has frozen?
out="$1"
{
  api=$(find ~/.gradle/caches/modules-2 -name 'fabric-api-*+1.21.4.jar' 2>/dev/null | head -1)
  echo "api=$api"
  if [ -n "$api" ]; then
    unzip -l "$api" | grep -oE 'META-INF/jars/[a-z0-9-]+\.jar' | head -20
  fi
  echo "===== registry-related classes across fabric-api-* jars"
  for j in $(find ~/.gradle/caches/modules-2 -name 'fabric-*registry*.jar' -o -name 'fabric-api-*.jar' 2>/dev/null | head -5); do
    echo "--- $j"
    unzip -l "$j" 2>/dev/null | grep -iE 'registry' | head -12
  done
} > "$out" 2>&1
