#!/usr/bin/env python3
"""Print the version manifest entries we care about (26.x, 1.21.x)."""
import json
import urllib.request

URL = 'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json'
d = json.load(urllib.request.urlopen(URL, timeout=40))
for v in d['versions']:
    vid = v['id']
    if vid.startswith('26.') or vid.startswith('1.21.1') or vid == '1.8.9':
        print('%-10s %-8s %s' % (vid, v['type'], v['url']))
