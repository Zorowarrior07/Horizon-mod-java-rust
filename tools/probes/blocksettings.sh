#!/usr/bin/env bash
# How must a Block be created on 1.21.4? AbstractBlock$Settings.getLootTableKey
# needs an id, which means Settings must be told what block it belongs to.
out="$1"; src="${2:?path to a minecraft-merged jar}"
{
  echo "===== AbstractBlock\$Settings (1.21.4)"
  javap -cp "$src" net.minecraft.block.AbstractBlock\$Settings 2>&1 | grep -E 'registryKey|dropsLike|create|Settings ' | head -12
  echo
  echo "===== AbstractBlock\$Settings#registryKey call sites in Block"
  javap -p -cp "$src" net.minecraft.block.Block 2>&1 | grep -E 'Block\(|lootTable' | head -8
  echo
  echo "===== Blocks.java: how does vanilla create one?"
  apt-get -qq install -y >/dev/null 2>&1 || true
  javap -c -cp "$src" net.minecraft.block.Blocks 2>&1 | grep -A3 -m3 'registryKey' | head -20
} > "$out" 2>&1
