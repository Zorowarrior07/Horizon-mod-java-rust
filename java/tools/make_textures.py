#!/usr/bin/env python3
"""Draw the mod's two 16x16 textures, deterministically, with no dependencies.

Run from anywhere:  python3 java/tools/make_textures.py

They are deliberately simple pixel art so the bytes are reproducible: the same
script output is what gets committed, so nobody has to trust a binary blob.
"""
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.normpath(os.path.join(HERE, '..', 'shared', 'assets', 'zoro_bridge', 'textures'))


def write_png(path, rows):
    """rows: list of 16 rows, each a list of 16 (r,g,b,a) tuples."""
    raw = b''.join(b'\x00' + bytes(v for px in row for v in px) for row in rows)

    def chunk(tag, data):
        body = tag + data
        return (struct.pack('>I', len(data)) + body
                + struct.pack('>I', zlib.crc32(body) & 0xFFFFFFFF))

    png = (b'\x89PNG\r\n\x1a\n'
           + chunk(b'IHDR', struct.pack('>IIBBBBB', 16, 16, 8, 6, 0, 0, 0))
           + chunk(b'IDAT', zlib.compress(raw, 9))
           + chunk(b'IEND', b''))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'wb') as fh:
        fh.write(png)
    return len(png)


STONE = (122, 122, 122, 255)
STONE_DARK = (104, 104, 104, 255)
GOLD = (231, 180, 58, 255)
GOLD_DARK = (176, 132, 34, 255)
CRYSTAL = (86, 200, 232, 255)
CRYSTAL_HOT = (176, 240, 255, 255)


def beacon():
    rows = []
    for y in range(16):
        row = []
        for x in range(16):
            # stone body with a bit of checker so it does not read as a flat
            # grey square in the inventory
            base = STONE if (x // 4 + y // 4) % 2 == 0 else STONE_DARK
            row.append(base)
        rows.append(row)

    # a gold cross through the middle
    for i in range(3, 13):
        rows[7][i] = GOLD
        rows[8][i] = GOLD
        rows[i][7] = GOLD
        rows[i][8] = GOLD
    for (x, y) in ((7, 7), (8, 7), (7, 8), (8, 8)):
        rows[y][x] = GOLD_DARK

    # four lit crystals, one in each quadrant
    for (cx, cy) in ((3, 3), (12, 3), (3, 12), (12, 12)):
        rows[cy][cx] = CRYSTAL_HOT
        for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            rows[cy + dy][cx + dx] = CRYSTAL
    return rows


def chip():
    rows = []
    for y in range(16):
        row = []
        for x in range(16):
            edge = x < 2 or x > 13 or y < 2 or y > 13
            row.append(GOLD_DARK if edge else GOLD)
        rows.append(row)

    # inner diamond of crystal, the "logic" of the chip
    for y in range(4, 12):
        for x in range(4, 12):
            if abs(x - 7.5) + abs(y - 7.5) <= 3.0:
                rows[y][x] = CRYSTAL if (x + y) % 3 else CRYSTAL_HOT

    # contact pins, top and bottom, so it reads as a chip not a coin
    for x in (4, 6, 8, 10):
        rows[0][x] = rows[1][x] = GOLD_DARK
        rows[15][x] = rows[14][x] = GOLD_DARK
    return rows


def main():
    for name, rows in (('block/bridge_beacon.png', beacon()),
                       ('item/bridge_chip.png', chip())):
        path = os.path.join(OUT, name)
        size = write_png(path, rows)
        print('wrote %s (%d bytes)' % (path, size))


if __name__ == '__main__':
    main()
