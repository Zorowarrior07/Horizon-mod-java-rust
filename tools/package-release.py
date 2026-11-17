#!/usr/bin/env python3
"""Assemble the release tree and the source ZIP.

    python3 tools/package-release.py

Produces, under dist/:

    dist/
      java-1.8.9/         Forge 1.8.9 jar
      java-1.21.x/        Fabric 1.21.1 / 1.21.4 / 1.21.11, NeoForge 1.21.1
      java-26.x/          NeoForge 26.1 jar
      horizon/            .hmod for the Horizon launcher
      reports/            compatibility report + verification results
      scripts/            the Windows build/install scripts
      source/             ZOROWARRIOR-HORIZON-BRIDGE-source.zip
      README.txt          what to install where

The built jars are read from artifacts/, which is what CI commits. Nothing here
invents a jar: if a target has no jar, its folder gets a MISSING.txt explaining
why, and the report says the same thing.
"""
import json
import os
import shutil
import sys
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DIST = os.path.join(ROOT, 'dist')
ART = os.path.join(ROOT, 'artifacts')

# where built jars come from, and where they belong in the release
LAYOUT = [
    ('java-1.8.9', os.path.join(ART, 'java-1.8.9'), 'forge-1.8.9'),
    ('java-1.21.x', os.path.join(ART, 'java-1.21.x'), None),
    ('java-26.x', os.path.join(ART, 'java-26.x'), None),
]

SIDE_FILES = [
    # (source, destination relative to dist/)
    ('VERIFY.txt', 'reports/verify-{slot}.txt'),
    ('verify.json', 'reports/verify-{slot}.json'),
]


def copy_jars(slot, src, filt):
    dest = os.path.join(DIST, slot)
    os.makedirs(dest, exist_ok=True)
    found = 0
    if not os.path.isdir(src):
        return found
    for name in sorted(os.listdir(src)):
        if not name.endswith('.jar'):
            continue
        if filt and filt not in name:
            continue
        shutil.copy2(os.path.join(src, name), os.path.join(dest, name))
        found += 1
    for side, pattern in SIDE_FILES:
        s = os.path.join(src, side.replace('verify', 'verify'))
        if os.path.exists(os.path.join(src, side)):
            out = os.path.join(DIST, pattern.format(slot=slot))
            os.makedirs(os.path.dirname(out), exist_ok=True)
            shutil.copy2(os.path.join(src, side), out)
    return found


def write_missing(slot, reason):
    path = os.path.join(DIST, slot, 'MISSING.txt')
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'w') as fh:
        fh.write(reason.strip() + '\n')


def source_zip():
    """Zip the sources a person needs to rebuild every jar."""
    dest = os.path.join(DIST, 'source')
    os.makedirs(dest, exist_ok=True)
    out = os.path.join(dest, 'ZOROWARRIOR-HORIZON-BRIDGE-source.zip')
    include = ['java', 'tools', 'docs', '.github', 'README.md', 'Cargo.toml',
               'Cargo.lock', 'rust-toolchain.toml']
    skip_dirs = {'build', '.gradle', 'run', 'target', '__pycache__', '.tools'}
    count = 0
    with zipfile.ZipFile(out, 'w', zipfile.ZIP_DEFLATED) as zf:
        for entry in include:
            full = os.path.join(ROOT, entry)
            if os.path.isfile(full):
                zf.write(full, entry)
                count += 1
                continue
            if not os.path.isdir(full):
                continue
            for base, dirs, files in os.walk(full):
                dirs[:] = [d for d in dirs if d not in skip_dirs]
                if 'build' in base.split(os.sep) or '.gradle' in base.split(os.sep):
                    continue
                for f in files:
                    absolute = os.path.join(base, f)
                    rel = os.path.relpath(absolute, ROOT)
                    zf.write(absolute, rel)
                    count += 1
    return out, count


README = """ZOROWARRIOR HORIZON BRIDGE — what installs where
================================================

ONE RULE, because it is the thing people get wrong:

    .jar  -> a Java Edition launcher (Fabric / Forge / NeoForge)
    .hmod -> the Horizon desktop launcher

The Horizon launcher has no JVM. It cannot load a .jar, and no setting makes it
try. The jars here are genuine Java Edition mods and are useless to Horizon; the
.hmod is the Horizon build and is useless to Minecraft. That is why both exist.

----------------------------------------
java-1.8.9/     Forge 1.8.9
----------------------------------------
    zoro_bridge-1.0.0-forge-1.8.9.jar

  Install: Forge 1.8.9 profile -> mods folder.
  Requires: Java 8, Forge 1.8.9 (11.15.0.1677 or newer).

----------------------------------------
java-1.21.x/    Fabric 1.21.1 / 1.21.4 / 1.21.11, NeoForge 1.21.1
----------------------------------------
    zoro_bridge-1.0.0-fabric-1.21.1.jar
    zoro_bridge-1.0.0-fabric-1.21.4.jar
    zoro_bridge-1.0.0-fabric-1.21.11.jar
    zoro_bridge-1.0.0-neoforge-1.21.1.jar

  Each jar names exactly one Minecraft version. Use the one that matches your
  profile; they are not interchangeable.
  Install: matching profile -> mods folder.
  Requires: Java 21; Fabric Loader 0.20.1+ (no Fabric API needed) or
            NeoForge 21.1+.

----------------------------------------
java-26.x/      NeoForge 26.x
----------------------------------------
    zoro_bridge-1.0.0-neoforge-26.1.jar

  Install: a NeoForge 26.1 profile -> mods folder.
  Requires: Java 25.

----------------------------------------
horizon/        Horizon launcher
----------------------------------------
    zoro_bridge-1.0.0.hmod

  Install: Horizon launcher -> its mod/package import.
  Requires: nothing else. No Java.

----------------------------------------
reports/        Honest status
----------------------------------------
  report.md                compatibility and testing report
  verify-*.txt/.json       the per-jar verification output, verbatim

----------------------------------------
source/         Rebuild it yourself
----------------------------------------
  ZOROWARRIOR-HORIZON-BRIDGE-source.zip
  Contains java/ (all four projects), tools/ (build + verify + package scripts)
  and the CI workflow that built the jars in this folder.
"""


def main():
    if not os.path.isdir(ART):
        print('error: artifacts/ is missing — build the jars first', file=sys.stderr)
        return 2

    for slot, src, filt in LAYOUT:
        n = copy_jars(slot, src, filt)
        print('%-14s %d jar(s)' % (slot, n))

    for slot, reason in (
        ('java-1.8.9',
         'NO JAR. The Forge 1.8.9 build did not produce one in this run.\n'
         'See reports/report.md: build tools/build-forge189.sh (or the .ps1 on\n'
         'Windows) and read ci/logs/forge-1.8.9.log.'),
        ('java-1.21.x',
         'NO JAR. The Fabric/NeoForge 1.21.x build did not produce one.\n'
         'See reports/report.md and ci/logs/.'),
        ('java-26.x',
         'NO JAR. The NeoForge 26.x build did not produce one.\n'
         'See reports/report.md and ci/logs/neoforge-26.1.log.'),
    ):
        d = os.path.join(DIST, slot)
        if os.path.isdir(d) and not any(f.endswith('.jar') for f in os.listdir(d)):
            write_missing(slot, reason)

    # Horizon package, if the Rust CLI produced one
    hor = os.path.join(DIST, 'horizon')
    os.makedirs(hor, exist_ok=True)
    for cand in (os.path.join(ROOT, 'out'), os.path.join(ROOT, 'examples')):
        if not os.path.isdir(cand):
            continue
        for base, _dirs, files in os.walk(cand):
            for f in files:
                if f.endswith('.hmod'):
                    shutil.copy2(os.path.join(base, f), os.path.join(hor, f))
    if not any(f.endswith('.hmod') for f in os.listdir(hor)):
        with open(os.path.join(hor, 'MISSING.txt'), 'w') as fh:
            fh.write('No .hmod in this run. Build one with the Rust CLI:\n'
                     '  cargo build --release\n'
                     '  ./target/release/zorowarrior-horizon-bridge convert <mod.jar> -o dist/horizon/\n')

    # scripts
    scr = os.path.join(DIST, 'scripts')
    os.makedirs(scr, exist_ok=True)
    for name in ('build-all.ps1', 'build-forge189.ps1', 'install.ps1',
                 'build-forge189.sh', 'verify_jar.py', 'package-release.py',
                 'ci-commit.sh'):
        src = os.path.join(ROOT, 'tools', name)
        if os.path.exists(src):
            shutil.copy2(src, scr)

    with open(os.path.join(DIST, 'README.txt'), 'w') as fh:
        fh.write(README)

    zpath, count = source_zip()
    print('source zip: %s (%d files)' % (zpath, count))
    return 0


if __name__ == '__main__':
    sys.exit(main())
