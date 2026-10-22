#!/usr/bin/env python3
"""Screenshot an X display (e.g. the Xvfb one from run-xvfb.sh) to PNG.

    python3 scripts/xshot.py out.png [:99]

Uses `xwd` and decodes its output here, so ImageMagick isn't needed. Needs Pillow.
"""
import struct
import subprocess
import sys

from PIL import Image


def main():
    out = sys.argv[1]
    display = sys.argv[2] if len(sys.argv) > 2 else ":99"
    data = subprocess.run(["xwd", "-root", "-silent", "-display", display], check=True, capture_output=True).stdout
    h = struct.unpack(">25I", data[:100])
    header_size, width, height = h[0], h[4], h[5]
    byte_order, bpp, bytes_per_line, ncolors = h[7], h[11], h[12], h[19]
    if bpp != 32:
        sys.exit(f"unsupported xwd depth: {bpp} bits per pixel")
    start = header_size + ncolors * 12
    raw = data[start : start + bytes_per_line * height]
    mode = "BGRX" if byte_order == 0 else "XRGB"
    Image.frombytes("RGB", (width, height), raw, "raw", mode, bytes_per_line).save(out)
    print(f"saved {out} ({width}x{height})")


if __name__ == "__main__":
    main()
