#!/usr/bin/env bash
# What fabric-loom versions exist? 26.x support likely needs a newer one.
curl -s https://maven.fabricmc.net/net/fabricmc/fabric-loom/maven-metadata.xml \
  | grep -oE '<version>[^<]+</version>' | tail -15
