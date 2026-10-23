#!/usr/bin/env python3
# Draws the app icon (white speech bubble on a blue disc) as PNGs at the
# Sailfish launcher sizes. Plain Python: no image library on the build host.
import struct, sys, zlib

def png(path, size, pixels):
    raw = b"".join(b"\0" + bytes(pixels[y * size * 4:(y + 1) * size * 4]) for y in range(size))
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))

def inside_bubble(x, y):
    # rounded rectangle body plus a tail at the lower left, in unit coordinates
    left, top, right, bottom, r = 0.24, 0.27, 0.76, 0.63, 0.1
    cx, cy = min(max(x, left + r), right - r), min(max(y, top + r), bottom - r)
    if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
        return True
    # tail: triangle (0.32,0.6) (0.46,0.6) (0.28,0.76)
    (ax, ay), (bx, by), (qx, qy) = (0.32, 0.6), (0.46, 0.6), (0.28, 0.76)
    d = (by - qy) * (ax - qx) + (qx - bx) * (ay - qy)
    a = ((by - qy) * (x - qx) + (qx - bx) * (y - qy)) / d
    b = ((qy - ay) * (x - qx) + (ax - qx) * (y - qy)) / d
    return a >= 0 and b >= 0 and a + b <= 1

def draw(size, ss=4):
    out = []
    for py in range(size):
        for px in range(size):
            disc = bubble = 0
            for sy in range(ss):
                for sx in range(ss):
                    x, y = (px + (sx + 0.5) / ss) / size, (py + (sy + 0.5) / ss) / size
                    if (x - 0.5) ** 2 + (y - 0.5) ** 2 <= 0.46 ** 2:
                        disc += 1
                        bubble += inside_bubble(x, y)
            n = ss * ss
            blue, white = (disc - bubble) / n, bubble / n
            alpha = blue + white
            if alpha == 0:
                out += [0, 0, 0, 0]
                continue
            rgb = [int((c1 * blue + 255 * white) / alpha) for c1 in (0, 122, 255)]
            out += rgb + [int(alpha * 255)]
    return out

for size in map(int, sys.argv[2:]):
    png(f"{sys.argv[1]}/{size}x{size}.png", size, draw(size))
