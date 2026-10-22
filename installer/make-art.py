#!/usr/bin/env python3
"""Draws the installer's welcome/finish side panel: the app icon on Burrow's lavender.

    python3 installer/make-art.py

Writes installer/welcome.bmp (164x314, the size NSIS's Modern UI expects). Needs Pillow.
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
W, H = 164, 314
TOP, BOTTOM = (221, 215, 247), (189, 179, 236)  # the sidebar's lavender, light to dark
INK = (58, 48, 102)


def main():
    img = Image.new("RGB", (W, H))
    draw = ImageDraw.Draw(img)
    for y in range(H):
        t = y / (H - 1)
        draw.line([(0, y), (W, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)))

    icon = Image.open(ROOT / "crates" / "burrow" / "resources" / "icon-256.png").convert("RGBA")
    icon = icon.resize((96, 96), Image.LANCZOS)
    img.paste(icon, ((W - 96) // 2, 78), icon)

    font = ImageFont.load_default(size=24)
    text = "Burrow"
    w = draw.textlength(text, font=font)
    draw.text(((W - w) / 2, 190), text, font=font, fill=INK)

    img.save(Path(__file__).resolve().parent / "welcome.bmp")


if __name__ == "__main__":
    main()
