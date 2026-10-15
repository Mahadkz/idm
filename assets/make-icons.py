#!/usr/bin/env python3
"""Render assets/xdm.png (256px), xdm-48.png and xdm.ico from the same geometry as xdm.svg."""
import sys
from pathlib import Path
from PIL import Image, ImageDraw
out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).parent
S = 1024  # supersample
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
k = S / 256
d.rounded_rectangle((0, 0, S - 1, S - 1), radius=48 * k, fill=(0x1f, 0x6f, 0xeb, 255))
d.polygon([(p[0] * k, p[1] * k) for p in [(104, 48), (152, 48), (152, 130), (188, 130), (128, 192), (68, 130), (104, 130)]], fill="white")
d.rounded_rectangle((64 * k, 206 * k, 192 * k, 222 * k), radius=8 * k, fill="white")
img.resize((256, 256), Image.LANCZOS).save(out / "xdm.png")
img.resize((48, 48), Image.LANCZOS).save(out / "xdm-48.png")
img.resize((256, 256), Image.LANCZOS).save(out / "xdm.ico", sizes=[(16, 16), (32, 32), (48, 48), (256, 256)])
