#!/usr/bin/env python3
"""Draws Burrow's app icon: a soft lavender tile with a white burrow arch.

    python3 scripts/make-icon.py

Writes crates/burrow/resources/icon-{16,32,64,256}.png and burrow.ico. Needs Pillow.
"""
from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "crates" / "burrow" / "resources"
SS = 4  # supersampling factor for smooth edges


def lerp(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def draw(size: int) -> Image.Image:
    s = size * SS
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))

    # Diagonal gradient tile.
    top, bottom = (178, 160, 255), (99, 76, 222)
    grad = Image.new("RGBA", (s, s))
    px = grad.load()
    for y in range(s):
        for x in range(s):
            px[x, y] = lerp(top, bottom, (x + y) / (2 * s)) + (255,)
    mask = Image.new("L", (s, s), 0)
    pad = round(s * 0.06)
    ImageDraw.Draw(mask).rounded_rectangle((pad, pad, s - pad, s - pad), radius=round(s * 0.24), fill=255)
    img.paste(grad, (0, 0), mask)

    # The burrow: a white arch with a darker opening.
    d = ImageDraw.Draw(img)
    w = s * 0.56
    left, right = (s - w) / 2, (s + w) / 2
    base = s * 0.74
    top_y = s * 0.26
    d.rounded_rectangle((left, top_y, right, base), radius=w / 2, fill=(255, 255, 255, 255))
    d.rectangle((left, (top_y + base) / 2, right, base), fill=(255, 255, 255, 255))
    inner = w * 0.42
    il, ir = (s - inner) / 2, (s + inner) / 2
    d.rounded_rectangle((il, s * 0.45, ir, base), radius=inner / 2, fill=(74, 52, 190, 255))
    d.rectangle((il, (s * 0.45 + base) / 2, ir, base), fill=(74, 52, 190, 255))
    # Ground line.
    d.rounded_rectangle((s * 0.18, base - s * 0.012, s * 0.82, base + s * 0.05), radius=s * 0.03, fill=(255, 255, 255, 255))
    return img.resize((size, size), Image.LANCZOS)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    big = draw(256)
    for n in (16, 32, 64, 256):
        (draw(n) if n < 64 else big.resize((n, n), Image.LANCZOS)).save(OUT / f"icon-{n}.png")
    big.save(OUT / "burrow.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    print(f"wrote icons to {OUT}")


if __name__ == "__main__":
    main()
