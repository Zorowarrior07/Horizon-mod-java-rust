#!/usr/bin/env python3
"""Why can't Loom find official mappings for 26.x? Look at the version json."""
import json
import urllib.request

for vid in ('26.1', '1.21.11'):
    url = None
    man = json.load(urllib.request.urlopen(
        'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json', timeout=40))
    for v in man['versions']:
        if v['id'] == vid:
            url = v['url']
    print('==', vid, url)
    if not url:
        continue
    d = json.load(urllib.request.urlopen(url, timeout=60))
    print('  javaVersion:', d.get('javaVersion'))
    for k in ('client', 'client_mappings', 'server_mappings'):
        ent = d.get('downloads', {}).get(k)
        print('  %-16s %s' % (k, ent['url'] if ent else 'MISSING'))
