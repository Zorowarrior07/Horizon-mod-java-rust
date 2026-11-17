#!/usr/bin/env bash
# Confirm the exact helper names on DeferredRegister.Items for 26.x.
out="$1"
{
  nf=$(find ~/.gradle/caches/modules-2 -name 'neoforge-26.1*-universal.jar' | head -1)
  echo "===== DeferredRegister.Items"
  javap -cp "$nf" 'net.neoforged.neoforge.registries.DeferredRegister$Items' 2>&1 | grep -E 'register|Simple' | head -14
} > "$out" 2>&1
