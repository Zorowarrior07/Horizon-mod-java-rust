#!/usr/bin/env bash
# What does the vanilla-only API for registering in Fabric's mod phase look like?
# Since 1.21.4 vanilla has its own registry-modification API; check whether the
# classes exist in 1.21.4 and 1.21.11 and what their method signatures are.
out="$1"; src="${2:?merged jar}"
{
  for c in net.minecraft.registry.RegistryBuilder \
           net.minecraft.registry.RegistryBuilder\$InitializedRegistryFactory \
           net.minecraft.registry.RegistryKeys ; do
    echo "===== $c"
    javap -cp "$src" "$c" 2>&1 | head -20
    echo
  done
  echo "===== anything in the jar named *InitializeRegistries* or *ModifyRegistries*"
  unzip -l "$src" | grep -iE 'Initializ|Modif.*Registr|Registr.*Modif' | head -20
} > "$out" 2>&1
