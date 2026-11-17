#!/usr/bin/env bash
# net.minecraft.registry.Registries$Initializer is vanilla's hook for mods.
# Print its full interface plus the static add/init methods on Registries.
out="$1"; src="$2"
{
  echo "===== Registries\$Initializer"
  javap -cp "$src" 'net.minecraft.registry.Registries$Initializer' 2>&1
  echo
  echo "===== Registries (static methods + initializer plumbing)"
  javap -cp "$src" net.minecraft.registry.Registries 2>&1 | grep -E 'static|Initializer|add' | head -25
  echo
  echo "===== who calls Initializer#initialize (search in the jar)"
  for c in net.minecraft.registry.Registries net.minecraft.registry.RegistryBuilder; do
    echo "-- $c"
    javap -c -p -cp "$src" "$c" 2>&1 | grep -B2 -A2 'Initializer' | head -30
  done
} > "$out" 2>&1
