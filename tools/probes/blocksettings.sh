#!/usr/bin/env bash
# How must a Block be created on 1.21.4? The NPE came from
# AbstractBlock$Settings.getLootTableKey, which means Settings must be told which
# block it describes. Find the method that does that.
out="$1"; src="${2:?path to a minecraft-merged jar}"
{
  echo "===== every Settings method mentioning a key/id"
  javap -cp "$src" net.minecraft.block.AbstractBlock\$Settings 2>&1 | grep -iE 'key|registr|id\(' | head -15
  echo
  echo "===== full Settings signature list (names only)"
  javap -cp "$src" net.minecraft.block.AbstractBlock\$Settings 2>&1 | grep -oE '[a-zA-Z]+\(' | sort -u | tr '\n' ' '
  echo
  echo
  echo "===== how 1.21.4 vanilla registers: javap of Blocks static init (first 40 lines)"
  javap -c -p -cp "$src" net.minecraft.block.Blocks 2>&1 | grep -E 'registryKey|Settings\.|ldc.*block/' | head -25
} > "$out" 2>&1
