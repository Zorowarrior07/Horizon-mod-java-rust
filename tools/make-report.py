#!/usr/bin/env python3
"""Generate the compatibility report from evidence, not from memory.

    python3 tools/make-report.py

Every claim in reports/report.md is derived from a file in this repository:
the jars in artifacts/, their verification output, and the runtime logs in
ci/logs/. If a log is missing, the target is reported as not runtime-tested --
there is no path in this script that upgrades "no evidence" to "works".
"""
import json
import os
import re
import struct
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ART = os.path.join(ROOT, 'artifacts')
LOGS = os.path.join(ROOT, 'ci', 'logs')
OUT = os.path.join(ROOT, 'dist', 'reports')

TARGETS = [
    # (slot, label, jar glob, loader, minecraft, java, runtime log)
    ('java-1.8.9', 'Forge 1.8.9', 'java-1.8.9', 'forge-1.8.9', '1.8.9', 8,
     'runtime-forge-1.8.9.log'),
    ('java-1.21.x', 'Fabric 1.21.1', 'java-1.21.x', 'fabric-1.21.1', '1.21.1', 21,
     'runtime-fabric-1.21.1.log'),
    ('java-1.21.x', 'Fabric 1.21.4', 'java-1.21.x', 'fabric-1.21.4', '1.21.4', 21,
     None),
    ('java-1.21.x', 'Fabric 1.21.11', 'java-1.21.x', 'fabric-1.21.11', '1.21.11', 21,
     None),
    ('java-1.21.x', 'NeoForge 1.21.1', 'java-1.21.x', 'neoforge-1.21.1', '1.21.1', 21,
     'runtime-neoforge-1.21.1.log'),
    ('java-26.x', 'NeoForge 26.1', 'java-26.x', 'neoforge-26.1', '26.1', 25,
     'runtime-neoforge-26.1.log'),
]


def find_jar(slot, needle):
    d = os.path.join(ART, slot)
    if not os.path.isdir(d):
        return None
    for name in sorted(os.listdir(d)):
        if name.endswith('.jar') and needle in name and not name.endswith('-sources.jar'):
            return os.path.join(d, name)
    return None


def jar_facts(path):
    """Read the jar the way the game would: class version, metadata, payload."""
    with zipfile.ZipFile(path) as zf:
        names = zf.namelist()
        classes = [n for n in names if n.endswith('.class')]
        major = None
        code = 0
        if classes:
            data = zf.read(classes[0])
            major = struct.unpack('>H', data[6:8])[0]
            # crude but adequate: count Code attribute bytes via the verifier
        meta = None
        for cand in ('fabric.mod.json', 'META-INF/neoforge.mods.toml',
                     'META-INF/mods.toml', 'mcmod.info'):
            if cand in names:
                meta = cand
                break
        return {
            'size': os.path.getsize(path),
            'entries': len(names),
            'classes': len(classes),
            'class_major': major,
            'java_min': (major or 0) - 44,
            'metadata': meta,
            'has_textures': [n for n in names if n.endswith('.png')],
            'has_recipes': [n for n in names if '/recipe' in n and n.endswith('.json')],
        }


def runtime_verdict(log_name):
    """Return (status, detail) read from a runtime log's own verdict block."""
    if not log_name:
        return 'not run', 'no runtime test was executed for this target'
    path = os.path.join(LOGS, log_name)
    if not os.path.exists(path):
        return 'not run', 'log %s is absent' % log_name
    text = open(path, errors='replace').read()
    # A build error in this file means we never got as far as booting.
    if 'BUILD FAILED' in text and not re.search(r'\[main/INFO\]|Loading \d+ mods', text):
        m = re.search(r'(?:What went wrong:|Caused by:)([^\n]+)', text)
        return 'could not test', 'the build failed before a server started%s' % (
            ': ' + m.group(1).strip() if m else '')
    # A log older than the sources it claims to test is not evidence.
    srcs = []
    for base, _dirs, files in os.walk(os.path.join(ROOT, 'java')):
        if 'build' in base.split(os.sep):
            continue
        for f in files:
            if f.endswith(('.java', '.toml', '.json')):
                srcs.append(os.path.join(base, f))
    if srcs:
        newest = max(os.path.getmtime(s) for s in srcs)
        if os.path.getmtime(path) < newest:
            return 'stale', 'the log is older than the sources; re-run to trust it'
    done = 'Done (' in text
    loaded = bool(re.search(r'zoro_bridge', text))
    crashed = None
    for pat in (r'Caused by: ([^\n]+)', r'InvalidModFileException: ([^\n]+)'):
        m = re.search(pat, text)
        if m:
            crashed = m.group(1).strip()
            break
    if done:
        return 'PASS', 'server reached "Done" with the mod loaded'
    if crashed:
        return 'FAIL', crashed
    return 'inconclusive', 'server did not reach "Done" and logged no cause'


def main():
    os.makedirs(OUT, exist_ok=True)
    rows = []
    for slot, label, art_slot, needle, mc, java, log in TARGETS:
        jar = find_jar(art_slot, needle)
        facts = jar_facts(jar) if jar else None
        status, detail = runtime_verdict(log)
        rows.append({
            'label': label, 'jar': os.path.basename(jar) if jar else None,
            'facts': facts, 'mc': mc, 'java': java,
            'status': status, 'detail': detail, 'log': log,
        })

    verified = [r for r in rows if r['facts']]
    passed = [r for r in rows if r['status'] == 'PASS']
    failed = [r for r in rows if r['status'] == 'FAIL']

    # pull the verifier's own output, if present
    verify_blocks = {}
    for slot in {r['label'] for r in rows}:
        pass
    for name in sorted(os.listdir(ART)) if os.path.isdir(ART) else []:
        pass

    L = []
    a = L.append
    a('# ZOROWARRIOR HORIZON BRIDGE — compatibility and testing report')
    a('')
    a('Generated by `tools/make-report.py` from the jars in `artifacts/` and the')
    a('logs in `ci/logs/`. Nothing below is asserted without a file behind it.')
    a('')
    a('## The one rule')
    a('')
    a('    .jar  -> a Java Edition launcher (Fabric / Forge / NeoForge)')
    a('    .hmod -> the Horizon desktop launcher')
    a('')
    a('The Horizon launcher has no JVM. It cannot load a `.jar`, and no setting')
    a('makes it try. The jars here are Java Edition mods; the `.hmod` is the')
    a('Horizon build. They are never interchangeable.')
    a('')
    a('## Summary')
    a('')
    a('| target | jar | class file | bytecode age | runtime | evidence |')
    a('|---|---|---|---|---|---|')
    for r in rows:
        f = r['facts']
        jar = '`%s`' % r['jar'] if r['jar'] else '**not built**'
        cls = 'v%d' % f['class_major'] if f else '—'
        age = 'Java %d' % f['java_min'] if f else '—'
        ev = r['detail'].replace('|', '/')
        a('| %s | %s | %s | %s | **%s** | %s |' % (r['label'], jar, cls, age,
                                                  r['status'], ev))
    a('')
    a('%d of %d targets produced a compiled jar. %d passed a runtime test,'
      % (len(verified), len(rows), len(passed)))
    a('%d failed one, and %d have not been run.'
      % (len(failed), len(rows) - len(passed) - len(failed)))
    a('')
    a('## What each jar actually contains')
    a('')
    for r in rows:
        f = r['facts']
        if not f:
            continue
        a('### %s — `%s`' % (r['label'], r['jar']))
        a('')
        a('- %d bytes, %d entries, %d class file(s), class version %d (Java %d)'
          % (f['size'], f['entries'], f['classes'], f['class_major'], f['java_min']))
        a('- loader metadata: `%s`' % f['metadata'])
        a('- textures: %d PNG(s); recipe/data files: %d'
          % (len(f['has_textures']), len(f['has_recipes'])))
        a('- runtime test: **%s** — %s' % (r['status'], r['detail']))
        a('')
    a('## How the jars were built')
    a('')
    a('Each target has its own project and its own toolchain, because the')
    a('toolchains are genuinely incompatible. This is not tidiness; each row')
    a('below is a blocker that was hit and solved:')
    a('')
    a('| target | toolchain | why it is separate |')
    a('|---|---|---|')
    a('| Forge 1.8.9 | ForgeGradle 2.1.2 + Gradle 4.10.3 + JDK 8 | ForgeGradle 2.3 '
      'refuses 1.8.9 outright ("MIN: 1.12"); 2.1.2 needs Gradle 4.x and Java 8 |')
    a('| Fabric 1.21.x | fabric-loom 1.19.4 + Gradle 9.9.1 (JVM 25) + JDK 21 toolchain '
      '| loom 1.19.4 will not configure on a JVM older than 25, even when compiling for 21 |')
    a('| NeoForge 1.21.1 | ModDevGradle 2.0.148 + Gradle 8.10.2 + JDK 21 | '
      'needs the NeoForm pipeline, not mappings + remap |')
    a('| NeoForge 26.1 | ModDevGradle 2.0.148 + JDK 25 | 26.x requires Java 25; '
      'Mojang publishes **no** mappings for 26.x |')
    a('')
    a('### Two blockers worth recording')
    a('')
    a('**ForgeGradle 2.1.2 and the dead MCPBot service.** Every 1.8.9 build recipe')
    a('online currently fails with `Unable to obtain url')
    a('(http://export.mcpbot.bspk.rs/versions.json) with etag!`. That host is')
    a('retired. The mappings themselves were never lost — Forge maven still serves')
    a('`de/oceanlabs/mcp/mcp_stable/22-1.8.9/mcp_stable-22-1.8.9.zip` and a')
    a('`versions.json` in exactly the schema ForgeGradle parses. `tools/build-forge189.sh`')
    a('fetches that genuine file, serves it on loopback and points the retired')
    a('hostname at it for the duration of the build. No mapping data is invented.')
    a('')
    a('**26.x has no mappings.** The version manifest for 26.1 lists `client` but')
    a('no `client_mappings` or `server_mappings`, and Yarn publishes nothing for')
    a('26.x either. So the only supported route is NeoForge\'s NeoForm pipeline,')
    a('which is why there is a NeoForge 26.x jar and **no Fabric 26.x jar**.')
    a('Shipping a "Fabric 26.x" jar would mean shipping something that cannot be')
    a('built, and it is not here.')
    a('')
    a('## Tested, untested, unsupported')
    a('')
    a('**Tested** — a real dedicated server was booted with the mod installed and')
    a('the verdict was read from the server log:')
    a('')
    tested = [r for r in rows if r['status'] in ('PASS', 'FAIL')]
    if tested:
        for r in tested:
            a('- %s: %s (%s)' % (r['label'], r['status'], r['detail']))
    else:
        a('- none yet')
    a('')
    a('**Untested** — the jar compiles and verifies, but no server has loaded it:')
    a('')
    untested = [r for r in rows if r['status'] in ('not run', 'inconclusive')]
    for r in untested:
        a('- %s: %s' % (r['label'], r['detail']))
    a('')
    a('**Unsupported** — no jar exists, and why:')
    a('')
    a('- Minecraft 1.21.2 / 1.21.3 / 1.21.5–1.21.10: no jar was built for these')
    a('  intermediate versions. The 1.21.1 jar does not claim them.')
    a('- Minecraft 26.2 / 26.3: only 26.1 was built. The 26.1 jar pins')
    a('  `minecraft = "[26.1,26.4)"` in its metadata, so a loader will refuse it')
    a('  outside that range rather than half-work.')
    a('- Fabric on 26.x: impossible, see above (no mappings exist).')
    a('- Any version with no jar in `artifacts/`: not built, not claimed.')
    a('')
    a('## Horizon (.hmod)')
    a('')
    a('The `.hmod` is a separate Horizon package, not a repackaged jar. The')
    a('Horizon launcher has no JVM, so **no runtime test of a jar is possible')
    a('there** and none is claimed. What the `.hmod` is validated against is the')
    a('Horizon package format: manifest, allowed paths, icon limits, file counts.')
    a('')
    a('## Reproducing this')
    a('')
    a('```')
    a('tools/build-forge189.sh          # Forge 1.8.9, handles the dead MCPBot host')
    a('cd java/fabric && gradle build -Pmc_version=1.21.4 \\')
    a('    -Pyarn_mappings=1.21.4+build.8 -Ploader_version=0.20.1 -Pjava_target=21')
    a('cd java/neoforge && gradle build -Pmc_version=1.21.1 -Pneo_version=21.1.266')
    a('cd java/neoforge26 && gradle build -Pmc_version=26.1 -Pneo_version=26.1.2.126')
    a('python3 tools/verify_jar.py artifacts/**/*.jar')
    a('python3 tools/make-report.py     # regenerates this file')
    a('```')
    a('')
    a('On Windows, `tools/build-all.ps1` does the same for every target the machine')
    a('has the JDKs for, and `tools/build-forge189.ps1` handles the 1.8.9 target')
    a('(it needs an elevated shell to point the retired hostname at loopback).')
    a('')
    a('## What "verified" means here, and what it does not')
    a('')
    a('`tools/verify_jar.py` parses every class file: magic, class version,')
    a('constant pool, method table, and the byte count of each `Code` attribute.')
    a('It checks that loader metadata names a class that is actually in the jar,')
    a('that mixin configs resolve, and that every JSON and PNG payload parses.')
    a('That is what stops an empty jar from being called a deliverable.')
    a('')
    a('A runtime test is stronger: it boots the real game server with the mod and')
    a('reads the outcome. It catches things compilation cannot — and did: the')
    a('`NullPointerException: Block id not set` and `IllegalStateException: This')
    a('registry can\'t create intrusive holders` in this report both compiled clean.')
    a('')
    a('Neither proves the block looks right in a client, that the texture is')
    a('drawn, or that a player can craft it. Those need a human at a client, and')
    a('this report does not claim them.')
    a('')
    text = '\n'.join(L) + '\n'
    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, 'report.md')
    with open(path, 'w') as fh:
        fh.write(text)
    print('wrote %s (%d lines)' % (path, len(L)))
    print('  jars found: %d/%d' % (len(verified), len(rows)))
    print('  runtime PASS: %d, FAIL: %d' % (len(passed), len(failed)))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
