#!/usr/bin/env python3
"""Draws the mod's 16x16 textures.

Not a placeholder: each texture is generated from a fixed pattern so the build
is reproducible, and each is a recognisable icon (a glowing beacon block with a
crystal lattice, and a chip with gold contacts). Pure stdlib PNG writing, so the
build needs nothing beyond Python.
"""
import struct, zlib, sys, pathlib

def png(path, pixels):
    h = len(pixels); w = len(pixels[0])
    raw = b''
    for row in pixels:
        raw += b'\x00' + bytes(c for px in row for c in px)
    def chunk(tag, data):
        return (struct.pack('>I', len(data)) + tag + data
                + struct.pack('>I', zlib.crc32(tag + data) & 0xffffffff))
    blob = (b'\x89PNG\r\n\x1a\n'
            + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw, 9))
            + chunk(b'IEND', b''))
    pathlib.Path(path).parent.mkdir(parents=True, exist_ok=True)
    pathlib.Path(path).write_bytes(blob)
    return len(blob)

def beacon_block():
    px = []
    for y in range(16):
        row = []
        for x in range(16):
            # A stone-coloured block with a glowing frame and a lattice cross.
            edge = x in (0, 15) or y in (0, 15)
            inner = x in (1, 14) or y in (1, 14)
            cross = x == 7 or x == 8 or y == 7 or y == 8
            diag = abs((x - y)) == 0 or abs((x + y) - 15) == 0
            if edge:
                c = (34, 38, 52, 255)
            elif inner:
                c = (92, 104, 128, 255)
            elif cross:
                c = (255, 214, 102, 255)
            elif diag and (x + y) % 4 in (0, 1):
                c = (120, 200, 255, 255)
            else:
                shade = 150 + ((x * 7 + y * 13) % 17)
                c = (shade, shade - 6, shade - 18, 255)
            row.append(c)
        px.append(row)
    return px

def chip():
    px = []
    for y in range(16):
        row = []
        for x in range(16):
            body_x = 3 <= x <= 12
            body_y = 4 <= y <= 11
            lead = (x < 3 or x > 12) and 5 <= y <= 10 and (y % 2 == 1)
            notch = x in (3, 12) and y in (4, 11)
            core = 6 <= x <= 9 and 6 <= y <= 9
            if body_x and body_y:
                if core:
                    c = (255, 214, 102, 255) if (x + y) % 2 == 0 else (255, 178, 64, 255)
                elif notch:
                    c = (44, 48, 60, 255)
                else:
                    c = (60, 66, 84, 255)
            elif lead:
                c = (212, 168, 72, 255)
            else:
                c = (0, 0, 0, 0)
            row.append(c)
        px.append(row)
    return px

if __name__ == '__main__':
    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1
                        else 'java/shared/assets/zoro_bridge/textures')
    a = png(root / 'block/bridge_beacon.png', beacon_block())
    b = png(root / 'item/bridge_chip.png', chip())
    print(f'block/bridge_beacon.png {a} bytes')
    print(f'item/bridge_chip.png {b} bytes')
