#!/usr/bin/env bash
out="$1"
{
  jar=$(find ~/.gradle/caches/modules-2 -name 'neoforge-26.1*-universal.jar' 2>/dev/null | head -1)
  echo "jar=$jar"
  echo "===== DeferredRegister.Blocks"
  javap -cp "$jar" 'net.neoforged.neoforge.registries.DeferredRegister$Blocks' 2>&1 | head -20
  echo "===== DeferredRegister.Items"
  javap -cp "$jar" 'net.neoforged.neoforge.registries.DeferredRegister$Items' 2>&1 | head -20
  echo "===== DeferredRegister (register overloads)"
  javap -cp "$jar" net.neoforged.neoforge.registries.DeferredRegister 2>&1 | grep -E 'register|create' | head -15
  echo "===== ResourceKey / id helpers in 26.x"
  mcj=$(find java/neoforge26/build/moddev/artifacts -name 'minecraft-patched-*.jar' | head -1)
  javap -cp "$mcj" net.minecraft.resources.ResourceKey 2>&1 | grep -E 'create' | head -10
} > "$out" 2>&1
