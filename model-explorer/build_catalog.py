#!/usr/bin/env python3
"""Scan ../converted and write catalog.json for the viewer."""
import json, os, re, sys
ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'converted')

def glbs(sub):
    d = os.path.join(ROOT, sub)
    return sorted((f[:-4], os.path.getsize(os.path.join(d, f)))
                  for f in os.listdir(d) if f.endswith('.glb'))

def build():
    maps = {}
    for name, size in glbs('maps'):
        m = re.match(r'^(.*?)(?:_(full|empty))?$', name)
        base, kind = m.group(1), m.group(2) or 'base'
        # a name like "x_empty" may also exist as its own map; only treat as variant if base exists
        maps.setdefault(base, {})[kind] = size
    cat = {
        'models': [[n, s] for n, s in glbs('models')],
        'animated': [[n, s] for n, s in glbs('animated')],
        'terrain': [[n, s] for n, s in glbs('terrain')],
        'maps': [[b, v] for b, v in sorted(maps.items())],
    }
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'catalog.json')
    with open(out, 'w') as f:
        json.dump(cat, f, separators=(',', ':'))
    print({k: len(v) for k, v in cat.items()}, '->', out)

if __name__ == '__main__':
    build()
