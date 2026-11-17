#!/usr/bin/env bash
# 26.x dropped ResourceLocation. How does a mod build a ResourceKey then?
out="$1"
{
  mcj=$(find java/neoforge26/build/moddev/artifacts -name 'minecraft-patched-*.jar' | head -1)
  echo "mcjar=$mcj"
  echo "===== ResourceKey"
  javap -cp "$mcj" net.minecraft.resources.ResourceKey 2>&1 | head -15
  echo "===== classes in net.minecraft.resources"
  unzip -l "$mcj" | grep -E 'net/minecraft/resources/[A-Za-z]+\.class' | awk '{print $4}' | head -25
  echo "===== DeferredRegister.Blocks.register signatures"
  nf=$(find ~/.gradle/caches/modules-2 -name 'neoforge-26.1*-universal.jar' | head -1)
  javap -cp "$nf" 'net.neoforged.neoforge.registries.DeferredRegister$Blocks' 2>&1 | head -14
  javap -cp "$nf" 'net.neoforged.neoforge.registries.DeferredRegister$Items' 2>&1 | head -16
} > "$out" 2>&1
