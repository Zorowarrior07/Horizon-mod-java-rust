#!/usr/bin/env bash
# NeoForge publishes 26.x builds, so try a real 26.x compile through ModDevGradle.
set -uo pipefail
out="$1"
dir=/tmp/neo26
rm -rf "$dir"; mkdir -p "$dir/src/main/java/t" "$dir/src/main/resources/META-INF"
cd "$dir"
cat > settings.gradle <<G
pluginManagement {
    repositories {
        maven { url = 'https://maven.neoforged.net/releases' }
        mavenCentral(); gradlePluginPortal()
    }
}
rootProject.name = 'neo26'
G
cat > build.gradle <<G
plugins {
    id 'net.neoforged.moddev' version '2.0.148'
    id 'java'
}
java { toolchain { languageVersion = JavaLanguageVersion.of(25) } }
neoForge { version = '26.1.2.126' }
G
cat > src/main/java/t/T.java <<J
package t;
public class T { public static void main(String[] a) { System.out.println("hi"); } }
J
cat > src/main/resources/META-INF/neoforge.mods.toml <<T
modLoader="javafml"
loaderVersion="[4,)"
[[mods]]
modId="t"
version="1.0.0"
T
JAVA_HOME=/usr/lib/jvm/temurin-25-jdk-amd64 gradle --no-daemon build > "$out" 2>&1
echo "exit=$?" >> "$out"
