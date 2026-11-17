#!/usr/bin/env bash
# 26.x changed how a Block is constructed (the 1.21.x Properties-only ctor no
# longer exists there). Print the real signatures.
out="$1"
{
  cat > /tmp/cp.gradle <<'G'
  allprojects { tasks.register('printCp') { doLast {
      configurations.compileClasspath.files.each { println "CP " + it } } } }
G
  ( cd java/neoforge26 && JAVA_HOME=/usr/lib/jvm/temurin-25-jdk-amd64 \
      gradle --no-daemon -q -I /tmp/cp.gradle printCp \
      -Pmc_version=26.1 -Pneo_version=26.1.2.126 -Pjava_target=25 ) > /tmp/cp26.txt 2>&1 || true
  mcj=$(grep -oE 'CP .*minecraft-(patched|merged)[^ ]*\.jar' /tmp/cp26.txt | sed 's/^CP //' | head -1)
  echo "jar=$mcj"
  echo "===== Block constructors"
  javap -cp "$mcj" net.minecraft.world.level.block.Block 2>&1 | grep -E 'Block\(' | head -8
  echo "===== BlockBehaviour\$Properties (key-related)"
  javap -cp "$mcj" 'net.minecraft.world.level.block.state.BlockBehaviour$Properties' 2>&1 | grep -iE 'key|setId|of\(' | head -12
  echo "===== BlockBehaviour ctors"
  javap -cp "$mcj" net.minecraft.world.level.block.state.BlockBehaviour 2>&1 | grep -E 'BlockBehaviour\(' | head -6
  echo "===== how vanilla makes a block (Blocks static init)"
  javap -c -p -cp "$mcj" net.minecraft.world.level.block.Blocks 2>&1 | grep -E 'Properties\.|Block\."<init>"|Block\.<init>' | head -12
} > "$out" 2>&1
