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
  echo "===== ResourceKey (26.x: how is an id built?)"
  javap -cp "$mcj" net.minecraft.resources.ResourceKey 2>&1 | grep -E 'create|public' | head -12
  echo "===== anything named *Identifier* or *Location* in net.minecraft.resources"
  unzip -l "$mcj" | grep -E 'net/minecraft/resources/[A-Za-z]*(Identifier|Location)[A-Za-z]*\.class' | head -8
  echo "===== DeferredRegister\$Blocks / \$Items (key-passing)"
  nf=$(find ~/.gradle/caches/modules-2 -name 'neoforge-26.1*-universal.jar' | head -1)
  javap -cp "$nf" 'net.neoforged.neoforge.registries.DeferredRegister$Blocks' 2>&1 | head -12
  javap -cp "$nf" 'net.neoforged.neoforge.registries.DeferredRegister$Items' 2>&1 | head -14
  echo "===== how vanilla makes a block (Blocks static init)"
  javap -c -p -cp "$mcj" net.minecraft.world.level.block.Blocks 2>&1 | grep -E 'Properties\.|Block\."<init>"|Block\.<init>' | head -12
} > "$out" 2>&1
