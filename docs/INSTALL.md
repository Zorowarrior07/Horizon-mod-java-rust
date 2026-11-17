# Installing ZOROWARRIOR HORIZON BRIDGE

Two different programs, two different files. Getting this wrong is the single
most common mistake, so it comes first:

| You have | You can install | You cannot install |
|---|---|---|
| Minecraft Java Edition with a mod loader | the `.jar` files | the `.hmod` |
| the Horizon desktop launcher | the `.hmod` | any `.jar` |

Horizon is not a Java program in the modding sense: it does not start a JVM for
mods and has no Fabric/Forge/NeoForge loader. A `.jar` dropped into Horizon is
ignored, not loaded. That is why this project ships both.

---

## Java Edition

### Pick the file that matches your profile exactly

A jar names one Minecraft version. Using 1.21.4's jar on 1.21.11 will be
rejected by the loader, and if it is not rejected it will misbehave.

| Your profile | Jar | Loader you must have installed | Java |
|---|---|---|---|
| 1.8.9 | `java-1.8.9/zoro_bridge-1.0.0-forge-1.8.9.jar` | Forge 1.8.9 (11.15.0.1677 or newer) | 8 |
| 1.21.1 | `java-1.21.x/zoro_bridge-1.0.0-fabric-1.21.1.jar` | Fabric Loader 0.20.1+, *no* Fabric API needed | 21 |
| 1.21.4 | `java-1.21.x/zoro_bridge-1.0.0-fabric-1.21.4.jar` | Fabric Loader 0.20.1+ | 21 |
| 1.21.11 | `java-1.21.x/zoro_bridge-1.0.0-fabric-1.21.11.jar` | Fabric Loader 0.20.1+ | 21 |
| 1.21.1 | `java-1.21.x/zoro_bridge-1.0.0-neoforge-1.21.1.jar` | NeoForge 21.1+ | 21 |
| 26.1 | `java-26.x/zoro_bridge-1.0.0-neoforge-26.1.jar` | NeoForge 26.1+ | 25 |

### Install

1. Install the loader for that version using your launcher, and start the game
   once so the profile exists.
2. Open the game directory (in the vanilla launcher: *Installations → the profile
   → ⋮ → Open folder*).
3. Put the jar in `mods/`.
4. Start that profile.

`tools/install.ps1` does steps 2–3 for you and reports exactly what it copied:

```powershell
powershell -ExecutionPolicy Bypass -File tools\install.ps1 -WhatIf   # dry run
powershell -ExecutionPolicy Bypass -File tools\install.ps1
```

### If it does not appear

- **The game starts and the block is missing** — you are on a different
  Minecraft version than the jar. Check the jar's filename against your profile.
- **The loader refuses the jar** — the mod pins `minecraft` in its metadata; read
  the error, it names the range it accepts.
- **`NoSuchMethodError` or a crash at startup** — the loader is older than the
  one in the table above.
- **1.21.3–1.21.11 with the Fabric build**: this target is *not runtime-verified*.
  See `reports/report.md`; the jar compiles and verifies, but a server boot has
  not yet succeeded on those versions.

### In game

- The **Bridge Beacon** is crafted from 8 stone around a Bridge Chip; it gives
  light level 12, is mined with an iron pickaxe or better, and drops itself.
- The **Bridge Chip** is crafted from 4 gold ingots around a diamond.
- On 1.8.9 both are in the vanilla creative tabs (*Building Blocks* and
  *Materials*).
- On 1.21.x and 26.x the block and chip exist but are **not** in any creative
  tab — no tab injection is implemented, so find them with `/give`:
  ```
  /give @s zoro_bridge:bridge_beacon
  /give @s zoro_bridge:bridge_chip
  ```

---

## Horizon launcher

Install `horizon/zoro_bridge-1.0.0.hmod` through the launcher's package import.
No Java, no loader, nothing else.

The `.hmod` is not a repackaged jar. It is a Horizon package: a manifest
(`horizon-mod.toml`, API 0.2) plus content under the package's own namespaces,
built by the Rust tooling in this repository. It is validated against the
Horizon package format — manifest schema, allowed paths, icon limits, file
counts — but it has **not** been run inside Horizon, and this project does not
claim it works there beyond that format validation.

---

## Why there is no "one jar for everything"

Each jar is compiled against that version's real game classes:

- 1.8.9 has no registry system, no `Block.Settings`, code-based recipes, and
  `textures/blocks/`. File version 52, Java 8.
- 1.21.x has registries, `AbstractBlock.Settings`, JSON data packs, and changed
  the block API again in 1.21.3. File version 65, Java 21.
- 26.x has no `ResourceLocation` at all and requires Java 25. File version 69.

Those are separate builds because they are separate APIs. One jar claiming all of
them would be a renamed zip with extra steps.
