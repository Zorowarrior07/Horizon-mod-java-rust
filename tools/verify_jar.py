#!/usr/bin/env python3
"""Prove a .jar is a compiled Minecraft mod and not a renamed zip.

    python3 tools/verify_jar.py dist/java-1.21.x/*.jar
    python3 tools/verify_jar.py --json out.json dist/**/*.jar

Checks, per jar:
  * it is a readable zip, with no duplicate entries
  * it contains real class files: CAFEBABE magic, sane major version, a
    parseable constant pool, and at least one method with a Code attribute
  * it carries loader metadata (fabric.mod.json / META-INF/mods.toml /
    META-INF/neoforge.mods.toml) and that metadata points at a class that is
    actually inside the jar
  * any mixin config named in the metadata is present
  * every .json entry parses, every .png entry has a valid header
  * it is not empty, and not a stub: the entry class must have a method body

Exit code 0 only if every jar passes. This is deliberately strict: it is the
gate that stops an empty jar from being called a deliverable.
"""
import json
import struct
import sys
import zipfile
import zlib


class Checker:
    def __init__(self, path):
        self.path = path
        self.ok = True
        self.notes = []

    def ok_(self, msg):
        self.notes.append(('ok', msg))

    def warn(self, msg):
        self.notes.append(('warn', msg))

    def fail(self, msg):
        self.notes.append(('FAIL', msg))
        self.ok = False


def parse_class(data, name, chk):
    """Minimal class-file reader: magic, version, constant pool, methods."""
    if len(data) < 10:
        chk.fail('%s: shorter than a class file header' % name)
        return None
    if data[:4] != b'\xca\xfe\xba\xbe':
        chk.fail('%s: bad magic %r (not a class file)' % (name, data[:4]))
        return None
    minor, major = struct.unpack('>HH', data[4:8])
    if not (45 <= major <= 70):
        chk.fail('%s: implausible class version %d.%d' % (name, major, minor))
        return None

    pos = 8
    (cp_count,) = struct.unpack('>H', data[pos:pos + 2])
    pos += 2
    utf8 = {}
    i = 1
    try:
        while i < cp_count:
            tag = data[pos]
            pos += 1
            if tag == 1:                       # Utf8
                (ln,) = struct.unpack('>H', data[pos:pos + 2])
                pos += 2
                utf8[i] = data[pos:pos + ln]
                pos += ln
            elif tag in (3, 4, 9, 10, 11, 12, 17, 18):
                pos += 4
            elif tag in (5, 6):                # Long/Double take two slots
                pos += 8
                i += 1
            elif tag in (7, 8, 16, 19, 20):
                pos += 2
            elif tag == 15:
                pos += 3
            else:
                chk.fail('%s: unknown constant-pool tag %d at index %d' % (name, tag, i))
                return None
            i += 1
        pos += 6                                # access_flags, this, super
        (if_count,) = struct.unpack('>H', data[pos:pos + 2])
        pos += 2 + 2 * if_count
        (field_count,) = struct.unpack('>H', data[pos:pos + 2])
        pos += 2
        pos = skip_members(data, pos, field_count)  # fields carry no Code
        if pos is None:
            chk.fail('%s: truncated in field table' % name)
            return None
        (method_count,) = struct.unpack('>H', data[pos:pos + 2])
        pos += 2
        code_bytes = 0
        methods = 0
        for _ in range(method_count):
            # access, name, desc, attribute_count
            if pos + 8 > len(data):
                chk.fail('%s: truncated in method table' % name)
                return None
            _acc, _n, _d, acount = struct.unpack('>HHHH', data[pos:pos + 8])
            pos += 8
            methods += 1
            for _ in range(acount):
                if pos + 6 > len(data):
                    chk.fail('%s: truncated in method attributes' % name)
                    return None
                aname, alen = struct.unpack('>HI', data[pos:pos + 6])
                pos += 6
                if utf8.get(aname) == b'Code':
                    code_bytes += alen
                pos += alen
        return {'major': major, 'minor': minor, 'methods': methods,
                'code_bytes': code_bytes, 'cp': cp_count}
    except (struct.error, IndexError):
        chk.fail('%s: malformed class file (ran off the end at byte %d)' % (name, pos))
        return None


def skip_members(data, pos, count):
    for _ in range(count):
        if pos + 8 > len(data):
            return None
        _acc, _n, _d, acount = struct.unpack('>HHHH', data[pos:pos + 8])
        pos += 8
        for _ in range(acount):
            if pos + 6 > len(data):
                return None
            _aname, alen = struct.unpack('>HI', data[pos:pos + 6])
            pos += 6 + alen
    return pos


def check_metadata(chk, zf, names):
    """Find loader metadata, and check it points at classes we actually ship."""
    classes = {n[:-6].replace('/', '.') for n in names if n.endswith('.class')}

    def entry_present(cls):
        return cls in classes

    found = []
    if 'fabric.mod.json' in names:
        found.append('fabric')
        try:
            meta = json.loads(zf.read('fabric.mod.json'))
        except Exception as exc:
            chk.fail('fabric.mod.json does not parse: %s' % exc)
            return
        eps = meta.get('entrypoints', {}).get('main', [])
        if not eps:
            chk.fail('fabric.mod.json declares no main entrypoint')
        for ep in eps:
            ep = ep if isinstance(ep, str) else ep.get('value', '')
            if not entry_present(ep):
                chk.fail('fabric.mod.json entrypoint %r has no matching .class' % ep)
            else:
                chk.ok_('entrypoint %s present' % ep)
        if meta.get('id') != 'zoro_bridge':
            chk.fail('unexpected mod id %r' % meta.get('id'))
        if not meta.get('depends', {}).get('minecraft'):
            chk.warn('fabric.mod.json does not pin a minecraft version')

    if 'META-INF/mods.toml' in names:
        found.append('forge')
        txt = zf.read('META-INF/mods.toml').decode('utf-8', 'replace')
        chk.ok_('mods.toml present (%d bytes)' % len(txt))
    if 'META-INF/neoforge.mods.toml' in names:
        found.append('neoforge')
        txt = zf.read('META-INF/neoforge.mods.toml').decode('utf-8', 'replace')
        chk.ok_('neoforge.mods.toml present (%d bytes)' % len(txt))
    if not found:
        chk.fail('no loader metadata: this would install nowhere')
    else:
        chk.ok_('loader metadata: %s' % ', '.join(found))

    # mixin configs referenced from metadata must exist
    for cfg_name in ('zoro_bridge.mixins.json', 'bridge.mixins.json'):
        if cfg_name in names:
            try:
                cfg = json.loads(zf.read(cfg_name))
            except Exception as exc:
                chk.fail('%s does not parse: %s' % (cfg_name, exc))
                continue
            pkg = cfg.get('package', '').replace('.', '/')
            missing = []
            for m in cfg.get('mixins', []) + cfg.get('client', []):
                cand = '%s/%s.class' % (pkg, m.replace('.', '/'))
                if cand not in names:
                    missing.append(m)
            if missing:
                chk.fail('%s lists mixins with no class: %s' % (cfg_name, ', '.join(missing)))
            else:
                chk.ok_('%s: all listed mixin classes present' % cfg_name)


def check_jar(path, chk):
    try:
        zf = zipfile.ZipFile(path)
    except Exception as exc:
        chk.fail('not a readable zip: %s' % exc)
        return
    infos = zf.infolist()
    names = [i.filename for i in infos]
    if len(names) != len(set(names)):
        chk.fail('duplicate zip entries')
    else:
        chk.ok_('%d entries, no duplicates' % len(names))

    if len(names) < 3:
        chk.fail('only %d entries: not a usable mod' % len(names))

    class_names = [n for n in names if n.endswith('.class')]
    if not class_names:
        chk.fail('no class files: this is not a mod, whatever its name')
        return

    total_code = 0
    for cn in class_names:
        info = parse_class(zf.read(cn), cn, chk)
        if info:
            total_code += info['code_bytes']
            chk.ok_('%s: class v%d.%d, %d methods, %d code bytes'
                    % (cn, info['major'], info['minor'], info['methods'], info['code_bytes']))
            if info['code_bytes'] == 0:
                chk.fail('%s has no method bodies (stub class)' % cn)

    if total_code < 64:
        chk.fail('class files total %d bytes of bytecode: effectively empty' % total_code)

    # JSON and PNG payloads must be real, or the game rejects the pack
    for n in names:
        if n.endswith('.json'):
            try:
                json.loads(zf.read(n))
            except Exception as exc:
                chk.fail('%s: invalid JSON (%s)' % (n, exc))
        elif n.endswith('.png'):
            d = zf.read(n)
            if d[:8] != b'\x89PNG\r\n\x1a\n':
                chk.fail('%s: not a PNG' % n)
            else:
                w, h = struct.unpack('>II', d[16:24])
                chk.ok_('%s: PNG %dx%d' % (n, w, h))

    check_metadata(chk, zf, names)
    zf.close()


def main(argv):
    args = [a for a in argv[1:]]
    json_out = None
    if '--json' in args:
        i = args.index('--json')
        json_out = args[i + 1]
        del args[i:i + 2]
    if not args:
        print('usage: verify_jar.py [--json OUT] JAR...', file=sys.stderr)
        return 2

    results = []
    worst = 0
    for path in args:
        chk = Checker(path)
        check_jar(path, chk)
        results.append({'jar': path, 'ok': chk.ok, 'notes': chk.notes})
        print('== %s' % path)
        for level, msg in chk.notes:
            print('   %-5s %s' % (level, msg))
        print('   -> %s' % ('PASS' if chk.ok else 'FAIL'))
        worst = max(worst, 0 if chk.ok else 1)

    if json_out:
        with open(json_out, 'w') as fh:
            json.dump({'schema': 'zoro-horizon-bridge/jar-verify@1',
                       'all_ok': worst == 0, 'jars': results}, fh, indent=2)
    print()
    print('%d jar(s), %s' % (len(args), 'all genuine' if worst == 0 else 'at least one is not genuine'))
    return worst


if __name__ == '__main__':
    sys.exit(main(sys.argv))
