#!/usr/bin/env bash
# Can fabric-loom + Mojang official mappings build for a 26.x version?
# Runs in a throwaway project so it cannot disturb the real ones.
set -uo pipefail
out="$1"
dir=/tmp/loom26
rm -rf "$dir"; mkdir -p "$dir/src/main/java/t" "$dir/src/main/resources"
cd "$dir"

cat > settings.gradle <<'G'
pluginManagement {
    repositories {
        maven { url = 'https://maven.fabricmc.net/' }
        mavenCentral(); gradlePluginPortal()
    }
}
rootProject.name = 'loom26'
G

cat > build.gradle <<'G'
plugins { id 'fabric-loom' version '1.19.4'; id 'java' }
java { toolchain { languageVersion = JavaLanguageVersion.of(25) } }
repositories { mavenCentral(); maven { url = 'https://maven.fabricmc.net/' } }
dependencies {
    minecraft "com.mojang:minecraft:26.1"
    mappings loom.officialMojangMappings()
    modImplementation "net.fabricmc:fabric-loader:0.20.1"
}
G

cat > src/main/java/t/T.java <<'J'
package t;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.item.Item;
public class T implements net.fabricmc.api.ModInitializer {
    static final Item CHIP = new Item(new Item.Properties());
    @Override public void onInitialize() {
        Registry.register(BuiltInRegistries.ITEM,
            ResourceLocation.fromNamespaceAndPath("t", "chip"), CHIP);
    }
}
J
echo '{"schemaVersion":1,"id":"t","version":"1.0.0","entrypoints":{"main":["t.T"]}}' \
  > src/main/resources/fabric.mod.json

gradle --no-daemon build > "$out" 2>&1
echo "exit=$?" >> "$out"
