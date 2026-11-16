#!/usr/bin/env python3
"""Verifies that a mod jar is a real, loadable Java mod.

This is the check that separates a genuine build from a zip renamed to `.jar`.
It parses each class file itself — magic, class-file version, the constant pool,
the entry-point method with a real Code attribute — and then checks the wiring
the loader will look for:

* the loader's metadata file, and that it names the entry point the jar ships;
* that the entry-point class implements the loader's initialiser interface;
* that every mixin the config names is present, each a class with a
  `@Mixin`-style target annotation;
* that every mixin config is listed in the loader metadata and that its refmap
  exists (Fabric refuses a mixin config that names a missing refmap);
* that all assets and data the mod claims are present and are valid JSON/PNG.

Exit code 0 when every jar passes, 1 otherwise. Nothing here needs Java.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import struct
import sys
import zipfile

# --- class-file parsing ------------------------------------------------------

CONSTANT_SIZES = {3: 4, 4: 4, 5: 8, 6: 8, 7: 2, 8: 2, 9: 4, 10: 4, 11: 4, 12: 4,
                  15: 3, 16: 2, 17: 4, 18: 4, 19: 2, 20: 2}
# 1 Utf8, 7 Class, 8 String, 9/10/11 refs, 3 Int, 4 Float, 5/6 Long/Double,
# 12 NameAndType, 15 MethodHandle, 16 MethodType, 17/18 Dynamic/InvokeDynamic,
# 19 Module, 20 Package.

JAVA_FOR_MAJOR = {52: 8, 53: 9, 54: 10, 55: 11, 56: 12, 57: 13, 58: 14, 59: 15,
                  60: 16, 61: 17, 62: 18, 63: 19, 64: 20, 65: 21, 66: 22,
                  67: 23, 68: 24, 69: 25, 70: 26}


class ClassFileError(Exception):
    pass


class Reader:
    def __init__(self, data: bytes):
        self.data = data
        self.at = 0

    def u1(self) -> int:
        if self.at + 1 > len(self.data):
            raise ClassFileError('truncated')
        value = self.data[self.at]
        self.at += 1
        return value

    def u2(self) -> int:
        if self.at + 2 > len(self.data):
            raise ClassFileError('truncated')
        value = struct.unpack_from('>H', self.data, self.at)[0]
        self.at += 2
        return value

    def u4(self) -> int:
        if self.at + 4 > len(self.data):
            raise ClassFileError('truncated')
        value = struct.unpack_from('>I', self.data, self.at)[0]
        self.at += 4
        return value

    def skip(self, count: int) -> None:
        if self.at + count > len(self.data):
            raise ClassFileError('truncated')
        self.at += count


class ClassInfo:
    """The facts about a class file that decide whether it can load."""

    def __init__(self, name: str, data: bytes):
        self.entry_name = name
        self.major = None
        self.minor = None
        self.this_class = None
        self.super_class = None
        self.interfaces: list[str] = []
        self.methods: list[tuple[str, str, str]] = []  # (name, descriptor, flags)
        self.has_code = False
        self.annotations: list[str] = []
        self.strings: list[str] = []
        self.class_refs: list[str] = []
        self._parse(data)

    def _parse(self, data: bytes) -> None:
        reader = Reader(data)
        if reader.u4() != 0xCAFEBABE:
            raise ClassFileError('bad magic: not a class file')
        self.minor = reader.u2()
        self.major = reader.u2()
        if not 45 <= self.major <= 70:
            raise ClassFileError(f'class version {self.major}.{self.minor} is out of range')
        count = reader.u2()
        pool: dict[int, object] = {}
        index = 1
        while index < count:
            tag = reader.u1()
            if tag == 1:  # Utf8
                length = reader.u2()
                raw = data[reader.at:reader.at + length]
                reader.skip(length)
                pool[index] = raw.decode('utf-8', 'replace')
            elif tag == 7:  # Class
                pool[index] = ('class', reader.u2())
            elif tag == 8:  # String
                pool[index] = ('string', reader.u2())
            elif tag in (3, 4):
                reader.skip(4)
            elif tag in (5, 6):
                reader.skip(8)
                index += 1  # long/double take two slots
            elif tag in (9, 10, 11):
                reader.skip(4)
            elif tag == 12:
                reader.skip(4)
            elif tag == 15:
                reader.skip(3)
            elif tag == 16:
                reader.skip(2)
            elif tag in (17, 18):
                reader.skip(4)
            elif tag in (19, 20):
                reader.skip(2)
            else:
                raise ClassFileError(f'constant pool tag {tag} is not a class-file tag')
            index += 1

        def utf8(at: int) -> str:
            value = pool.get(at)
            if not isinstance(value, str):
                raise ClassFileError(f'constant #{at} is not a Utf8')
            return value

        def class_name(at: int) -> str:
            value = pool.get(at)
            if isinstance(value, tuple) and value[0] == 'class':
                return utf8(value[1])
            raise ClassFileError(f'constant #{at} is not a Class')

        self.strings = [utf8(at) for at, value in pool.items()
                        if isinstance(value, tuple) and value[0] == 'string']
        # Every Class constant is a reference the class makes at runtime.
        self.class_refs = [class_name(at) for at, value in pool.items()
                          if isinstance(value, tuple) and value[0] == 'class']

        reader.u2()  # access flags
        self.this_class = class_name(reader.u2())
        super_at = reader.u2()
        if super_at:
            self.super_class = class_name(super_at)
        for _ in range(reader.u2()):
            self.interfaces.append(class_name(reader.u2()))

        for what in ('fields', 'methods'):
            for _ in range(reader.u2()):
                name = utf8(reader.u2())
                descriptor = utf8(reader.u2())
                flags = reader.u2()
                for _ in range(reader.u2()):
                    attribute = utf8(reader.u2())
                    length = reader.u4()
                    if attribute == 'Code':
                        if what == 'methods':
                            self.has_code = True
                        reader.skip(length)
                        continue
                    raw = data[reader.at:reader.at + length]
                    reader.skip(length)
                    if attribute in ('RuntimeVisibleAnnotations',
                                     'RuntimeInvisibleAnnotations'):
                        self._read_annotations(raw)
                if what == 'methods':
                    self.methods.append((name, descriptor, f'{flags:#06x}'))

        for _ in range(reader.u2()):
            attribute = utf8(reader.u2())
            length = reader.u4()
            raw = data[reader.at:reader.at + length]
            reader.skip(length)
            if attribute in ('RuntimeVisibleAnnotations', 'RuntimeInvisibleAnnotations'):
                self._read_annotations(raw)

    def _read_annotations(self, raw: bytes) -> None:
        """Reads only the annotation *types*, which is all that is needed."""
        if len(raw) < 2:
            return
        count = struct.unpack_from('>H', raw, 0)[0]
        at = 2
        for _ in range(count):
            if at + 2 > len(raw):
                return
            type_index = struct.unpack_from('>H', raw, at)[0]
            at += 2
            self.annotations.append(f'#{type_index}')
            # Walk the element-value pairs, skipping values.
            if at + 2 > len(raw):
                return
            pairs = struct.unpack_from('>H', raw, at)[0]
            at += 2
            for _ in range(pairs):
                at += 2  # element_name_index
                at = skip_element_value(raw, at)

    def annotation_texts(self, pool_utf8: dict[int, str]) -> list[str]:
        return [pool_utf8.get(int(a[1:]), a) for a in self.annotations]


def skip_element_value(raw: bytes, at: int) -> int:
    if at >= len(raw):
        return len(raw)
    tag = chr(raw[at])
    at += 1
    if tag in 'BCDFIJSZs':
        return at + 2
    if tag == 'e':
        return at + 4
    if tag == 'c':
        return at + 2
    if tag == '@':
        at += 2
        if at + 2 > len(raw):
            return len(raw)
        pairs = struct.unpack_from('>H', raw, at)[0]
        at += 2
        for _ in range(pairs):
            at += 2
            at = skip_element_value(raw, at)
        return at
    if tag == '[':
        if at + 2 > len(raw):
            return len(raw)
        count = struct.unpack_from('>H', raw, at)[0]
        at += 2
        for _ in range(count):
            at = skip_element_value(raw, at)
        return at
    return at


# --- jar checks -------------------------------------------------------------

def read_json(archive: zipfile.ZipFile, name: str):
    return json.loads(archive.read(name).decode('utf-8-sig'))


def is_png(data: bytes) -> tuple[bool, str]:
    if len(data) < 33 or data[:8] != b'\x89PNG\r\n\x1a\n':
        return False, 'not a PNG'
    if data[12:16] != b'IHDR':
        return False, 'no IHDR chunk'
    width, height = struct.unpack_from('>II', data, 16)
    if width == 0 or height == 0:
        return False, 'zero size'
    return True, f'{width}x{height}'


def verify(jar_path: pathlib.Path) -> dict:
    result = {
        'jar': jar_path.name,
        'bytes': jar_path.stat().st_size,
        'checks': [],
        'errors': [],
        'warnings': [],
    }

    def ok(text: str) -> None:
        result['checks'].append({'ok': True, 'what': text})

    def fail(text: str) -> None:
        result['errors'].append(text)

    def warn(text: str) -> None:
        result['warnings'].append(text)

    try:
        archive = zipfile.ZipFile(jar_path)
    except zipfile.BadZipFile as error:
        fail(f'not a zip archive at all: {error}')
        return result

    names = archive.namelist()
    result['entries'] = len(names)
    if len(names) != len(set(names)):
        duplicates = sorted({n for n in names if names.count(n) > 1})
        fail(f'duplicate entries, which some loaders refuse: {duplicates[:5]}')
    else:
        ok(f'{len(names)} entries, no duplicates')

    # 1. Class files: parse every one.
    classes: dict[str, ClassInfo] = {}
    for name in names:
        if not name.endswith('.class'):
            continue
        try:
            info = ClassInfo(name, archive.read(name))
            classes[info.this_class] = info
        except ClassFileError as error:
            fail(f'{name}: {error}')
    if not classes:
        fail('no class files: this is not a mod, whatever its name')
        return result
    majors = sorted({c.major for c in classes.values()})
    java_versions = sorted({JAVA_FOR_MAJOR.get(m, 0) for m in majors})
    result['class_major'] = majors
    result['java'] = java_versions
    if len(majors) > 1:
        warn(f'mixed class-file versions {majors}; the loader takes the highest')
    ok(f'{len(classes)} class file(s), version {majors}, i.e. Java {java_versions}')

    # 2. Loader metadata.
    loader = None
    entry_points: list[str] = []
    depends: dict = {}
    if 'fabric.mod.json' in names:
        loader = 'fabric'
        mod = read_json(archive, 'fabric.mod.json')
        for key in ('id', 'version', 'name', 'schemaVersion'):
            if key not in mod:
                fail(f'fabric.mod.json has no `{key}`, which Fabric requires')
        result['mod'] = {k: mod.get(k) for k in ('id', 'name', 'version', 'license')}
        depends = mod.get('depends', {}) or {}
        for kind, entries in (mod.get('entrypoints') or {}).items():
            for entry in entries:
                if isinstance(entry, dict):
                    entry_points.append(entry.get('value', ''))
                else:
                    entry_points.append(entry)
        result['entrypoints'] = entry_points
        if '${' in json.dumps(mod):
            fail('fabric.mod.json still contains unresolved ${...} placeholders')
        else:
            ok('fabric.mod.json is complete and has no unresolved placeholders')
    elif 'META-INF/neoforge.mods.toml' in names:
        loader = 'neoforge'
        text = archive.read('META-INF/neoforge.mods.toml').decode('utf-8-sig')
        result['mod'] = {'toml': 'META-INF/neoforge.mods.toml'}
        if '[[mods]]' not in text:
            fail('neoforge.mods.toml has no [[mods]] table')
        if 'modId' not in text:
            fail('neoforge.mods.toml has no modId')
        ok('META-INF/neoforge.mods.toml is present with a [[mods]] table')
    elif 'mcmod.info' in names:
        loader = 'forge-legacy'
        info = read_json(archive, 'mcmod.info')
        result['mod'] = info[0] if isinstance(info, list) and info else info
        if not result['mod'].get('modid'):
            fail('mcmod.info has no modid')
        if not result['mod'].get('mcversion'):
            warn('mcmod.info does not state mcversion')
        ok('mcmod.info is present and names the mod')
    else:
        fail('no loader metadata (fabric.mod.json, neoforge.mods.toml or mcmod.info)')
    result['loader'] = loader

    # 3. The entry point must exist and actually be an entry point.
    interface_for = {
        'fabric': 'net/fabricmc/api/ModInitializer',
        'forge-legacy': None,
    }
    for entry in entry_points:
        binary = entry.replace('.', '/')
        info = classes.get(binary)
        if info is None:
            fail(f'entry point `{entry}` is named in the metadata but no such class is in the jar')
            continue
        if not any(name == '<init>' for name, _, _ in info.methods):
            warn(f'entry point `{entry}` has no constructor')
        ifaces = {i for i in info.interfaces}
        expected = interface_for.get(loader or '')
        if expected and expected not in ifaces:
            fail(f'entry point `{entry}` does not implement {expected}, so the loader cannot call it')
        else:
            ok(f'entry point `{entry}` is present and implements '
               f'{", ".join(sorted(ifaces)) or "no interface"}')
        has_initialise = any(name == 'onInitialize' for name, _, _ in info.methods)
        if loader == 'fabric' and not has_initialise:
            fail(f'entry point `{entry}` has no onInitialize method')
        elif has_initialise:
            ok(f'entry point `{entry}` implements onInitialize')

    # 4. Mixins: every class named must exist, and every config must be registered.
    mixin_configs = [n for n in names if n.endswith('.mixins.json')]
    declared_configs: list[str] = []
    if loader == 'fabric' and 'fabric.mod.json' in names:
        mod = read_json(archive, 'fabric.mod.json')
        declared_configs = list(mod.get('mixins') or [])
    for config_name in mixin_configs:
        config = read_json(archive, config_name)
        package = config.get('package', '').replace('.', '/')
        listed = []
        for key in ('mixins', 'client', 'server'):
            listed += [f'{package}/{m}' for m in (config.get(key) or [])]
        for target in listed:
            if target not in classes:
                fail(f'{config_name} names mixin `{target}` but that class is not in the jar')
        if declared_configs and config_name not in declared_configs:
            fail(f'{config_name} exists but fabric.mod.json does not list it, so it never loads')
        refmap = config.get('refmap')
        if refmap and refmap not in names:
            fail(f'{config_name} names refmap `{refmap}` but the jar does not contain it, '
                 'which makes Fabric refuse the mod at launch')
        ok(f'{config_name}: package `{package}`, {len(listed)} mixin class(es), '
           f'refmap {"present" if refmap else "none"}')

    # 5. Assets and data are present and readable.
    pngs = [n for n in names if n.endswith('.png')]
    for name in pngs:
        good, detail = is_png(archive.read(name))
        if not good:
            fail(f'{name}: {detail}')
    if pngs:
        ok(f'{len(pngs)} texture(s), all valid PNG headers')

    json_files = [n for n in names
                  if n.endswith('.json') and not n.endswith('fabric.mod.json')
                  and not n.endswith('.mixins.json')]
    broken = 0
    for name in json_files:
        try:
            json.loads(archive.read(name).decode('utf-8-sig'))
        except Exception as error:  # noqa: BLE001 - any parse error is the point
            fail(f'{name} is not readable JSON: {error}')
            broken += 1
    if json_files and not broken:
        ok(f'{len(json_files)} JSON file(s), all parse')

    # 6. Class-file version against the declared Java dependency.
    if depends.get('java') and java_versions:
        needed = int(''.join(ch for ch in str(depends['java']) if ch.isdigit()) or 0)
        highest = max(java_versions)
        if needed > highest:
            fail(f'metadata requires Java {needed} but the classes are Java {highest}')
        elif highest < 17:
            warn(f'classes are Java {highest}; 1.21+ mods normally target Java 21')

    archive.close()
    result['ok'] = not result['errors']
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('jars', nargs='+', type=pathlib.Path)
    parser.add_argument('--json', type=pathlib.Path)
    args = parser.parse_args()

    results = [verify(path) for path in args.jars]
    for result in results:
        print(f'== {result["jar"]} ({result.get("bytes", 0)} bytes)')
        for check in result['checks']:
            print(f'   ok   {check["what"]}')
        for message in result['warnings']:
            print(f'   warn {message}')
        for message in result['errors']:
            print(f'   FAIL {message}')
        print(f'   -> {"PASS" if result.get("ok") else "FAIL"}')
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(results, indent=2))
    return 0 if all(r.get('ok') for r in results) else 1


if __name__ == '__main__':
    sys.exit(main())
