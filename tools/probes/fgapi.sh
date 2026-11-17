#!/usr/bin/env bash
# Print the real GameRegistry/Block method list from a decompiled 1.8.9 tree,
# when one exists, so API questions are answered by the jar and not by memory.
out="$1"
java_home="${JAVA_HOME_8_X64:-/usr/lib/jvm/temurin-8-jdk-amd64}"
cp="${MC_CP:?}"
{
  for c in net.minecraftforge.fml.common.registry.GameRegistry net.minecraft.block.Block; do
    echo "===== $c"
    "$java_home/bin/javap" -cp "$cp" "$c" 2>&1 | grep -E 'register|setRegistryName|setHarvestLevel'
  done
} > "$out" 2>&1
