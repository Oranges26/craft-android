"""Draw the launcher icons of the ports (mipmap PNGs at every density).

    python3 tools/make_icons.py
"""
import math
import os

from PIL import Image, ImageDraw

DENSITIES = {"mdpi": 48, "hdpi": 72, "xhdpi": 96, "xxhdpi": 144, "xxxhdpi": 192}
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def base(size, top, bottom):
    s = size * 4  # draw large, scale down for smooth edges
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    grad = Image.new("RGBA", (s, s))
    gd = ImageDraw.Draw(grad)
    for y in range(s):
        t = y / (s - 1)
        c = tuple(int(top[i] * (1 - t) + bottom[i] * t) for i in range(3)) + (255,)
        gd.line([(0, y), (s, y)], fill=c)
    mask = Image.new("L", (s, s), 0)
    ImageDraw.Draw(mask).rounded_rectangle([s * 0.04, s * 0.04, s * 0.96, s * 0.96], radius=s * 0.22, fill=255)
    img.paste(grad, (0, 0), mask)
    return img, ImageDraw.Draw(img), s


def aperture(d, s):
    cx = cy = s / 2
    r = s * 0.30
    d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=(255, 255, 255, 255), width=int(s * 0.045))
    for k in range(6):
        a = k * math.pi / 3
        p1 = (cx + r * math.cos(a), cy + r * math.sin(a))
        a2 = a + math.pi / 3 + 0.5
        p2 = (cx + r * 0.45 * math.cos(a2), cy + r * 0.45 * math.sin(a2))
        d.line([p1, p2], fill=(255, 255, 255, 255), width=int(s * 0.035))


def film(d, s):
    x0, x1, y0, y1 = s * 0.24, s * 0.76, s * 0.22, s * 0.78
    d.rounded_rectangle([x0, y0, x1, y1], radius=s * 0.04, outline=(255, 255, 255, 255), width=int(s * 0.04))
    for k in range(5):
        y = y0 + (y1 - y0) * (k + 0.5) / 5
        for x in (x0 + s * 0.06, x1 - s * 0.06):
            d.rectangle([x - s * 0.022, y - s * 0.022, x + s * 0.022, y + s * 0.022], fill=(255, 255, 255, 255))
    cx, cy = s / 2, s / 2
    d.polygon([(cx - s * 0.07, cy - s * 0.1), (cx - s * 0.07, cy + s * 0.1), (cx + s * 0.1, cy)], fill=(255, 255, 255, 255))


def grid(d, s):
    g = s * 0.13
    for i in range(2):
        for j in range(2):
            x = s * 0.27 + i * (g + s * 0.06)
            y = s * 0.27 + j * (g + s * 0.06)
            d.rounded_rectangle([x, y, x + g + s * 0.02, y + g + s * 0.02], radius=s * 0.04, fill=(255, 255, 255, 255))


def spark(d, s):
    cx, cy = s / 2, s / 2
    pts = []
    for k in range(8):
        r = s * (0.30 if k % 2 == 0 else 0.10)
        a = k * math.pi / 4 - math.pi / 2
        pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    d.polygon(pts, fill=(255, 255, 255, 255))


def brush(d, s):
    d.line([(s * 0.30, s * 0.70), (s * 0.68, s * 0.30)], fill=(255, 255, 255, 255), width=int(s * 0.09))
    d.ellipse([s * 0.22, s * 0.62, s * 0.38, s * 0.78], fill=(255, 255, 255, 255))


APPS = {
    "lightcraft/android/res": ((70, 160, 255), (30, 70, 190), aperture),
    "filmcraft/android/res": ((255, 120, 80), (180, 40, 90), film),
    "launcher/res": ((90, 90, 110), (30, 30, 40), grid),
}

for path, (top, bottom, glyph) in APPS.items():
    for dens, px in DENSITIES.items():
        img, d, s = base(px, top, bottom)
        glyph(d, s)
        out = img.resize((px, px), Image.LANCZOS)
        folder = os.path.join(ROOT, path, f"mipmap-{dens}")
        os.makedirs(folder, exist_ok=True)
        out.save(os.path.join(folder, "ic_launcher.png"))
    print("icons:", path)
