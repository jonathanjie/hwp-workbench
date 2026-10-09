#!/usr/bin/env python3
"""Create native icon sizes from our source icon, without network access.

Requires the developer's Pillow installation; it is not an app dependency.
"""
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
DESTINATION = Path(__file__).resolve().parent / 'icons'
DESTINATION.mkdir(exist_ok=True)
with Image.open(ROOT / 'desktop/assets/icon.png') as source:
    icon = source.convert('RGBA')
    for name, size in [('32x32.png', 32), ('128x128.png', 128),
                       ('128x128@2x.png', 256), ('icon.png', 512)]:
        icon.resize((size, size), Image.Resampling.LANCZOS).save(DESTINATION / name)
    icon.save(DESTINATION / 'icon.icns', format='ICNS')
    icon.save(DESTINATION / 'icon.ico', format='ICO', sizes=[(16, 16), (32, 32), (48, 48), (256, 256)])
