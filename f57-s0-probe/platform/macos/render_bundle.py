#!/usr/bin/env python3
"""Prepare ONLY the disposable bundle's plists. Never builds, signs, registers or runs it."""
import argparse
from pathlib import Path
import plistlib
import re

p = argparse.ArgumentParser()
p.add_argument('--rig', required=True)
p.add_argument('--bundle-id', required=True)
p.add_argument('--label', required=True)
a = p.parse_args()
rig = Path(a.rig).resolve(strict=True)
if not rig.is_dir() or rig == Path('/') or rig == Path.home():
    p.error('an existing dedicated rig subdirectory is required')
for val in (a.bundle_id, a.label):
    if not re.fullmatch(r'org\.cys\.f57s0\.[A-Za-z0-9.-]+', val):
        p.error('use a unique org.cys.f57s0.* identifier')
app = rig / 'F57S0LocalNetwork.app'
values = {'__BUNDLE_ID__': a.bundle_id, '__LABEL__': a.label, '__APP__': str(app), '__RIG__': str(rig)}
def substitute(x):
    if isinstance(x, str):
        for key, value in values.items(): x = x.replace(key, value)
        return x
    if isinstance(x, list): return [substitute(v) for v in x]
    if isinstance(x, dict): return {key: substitute(value) for key, value in x.items()}
    return x
pairs = [('Info.plist.template', app / 'Contents' / 'Info.plist'),
         ('LaunchAgent.plist.template', rig / (a.label + '.plist'))]
for _, target in pairs:
    if target.exists() or target.is_symlink(): p.error('refusing to overwrite ' + str(target))
    if rig not in target.resolve().parents: p.error('target escapes rig')
for source, target in pairs:
    data = plistlib.loads((Path(__file__).parent / source).read_bytes())
    target.parent.mkdir(parents=True, exist_ok=True)
    with target.open('xb') as f: plistlib.dump(substitute(data), f, sort_keys=False)
    print(target)
