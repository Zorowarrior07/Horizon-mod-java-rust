#!/usr/bin/env bash
# Which NeoForge ModDevGradle versions exist? 26.x needs newer than 2.0.107.
out="$1"
{
  curl -s https://maven.neoforged.net/releases/net/neoforged/moddev/net.neoforged.moddev.gradle.plugin/maven-metadata.xml \
    | grep -oE '<version>[^<]+</version>' | tail -12
  echo "-- core plugin"
  curl -s https://maven.neoforged.net/releases/net/neoforged/neoform/runtime/maven-metadata.xml \
    | grep -oE '<version>[^<]+</version>' | tail -5
} > "$out" 2>&1
