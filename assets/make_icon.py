"""RustCleanup icon: a white outline of a small paint brush on pure black.

Usage: python assets/make_icon.py assets   (writes icon.ico, icon.png, preview.png)
Each size is drawn on its own (8x supersampled) so small sizes keep a stroke
that is at least ~1.3 px wide instead of a blurred-away hairline.
"""
import math, sys
from PIL import Image, ImageDraw

SS = 8
SIZES = [16, 20, 24, 32, 40, 48, 64, 256]

def arc(cx, cy, r, a0, a1, n=10):
    return [(cx + r * math.cos(math.radians(a0 + (a1 - a0) * i / n)),
             cy + r * math.sin(math.radians(a0 + (a1 - a0) * i / n))) for i in range(n + 1)]

def brush_parts():
    """Brush pointing down, in a 100x100 box centred on x=50 (y grows down)."""
    hw = 6.5                        # handle half width
    handle = [(50 - hw, 50)] + arc(50, 14, hw, 180, 360) + [(50 + hw, 50)]
    ferrule = [(39, 50), (61, 50), (61, 62), (39, 62)]
    # Bristles: two gentle curves meeting in a point.
    def bez(p0, c, p1, n=12):
        return [((1-t)**2*p0[0]+2*(1-t)*t*c[0]+t*t*p1[0], (1-t)**2*p0[1]+2*(1-t)*t*c[1]+t*t*p1[1])
                for t in [i / n for i in range(n + 1)]]
    bristles = [(39, 62)] + bez((61, 62), (65, 80), (50, 94)) + bez((50, 94), (35, 80), (39, 62))
    return [handle, ferrule, bristles]

def transform(pts, size):
    # Rotate 45 degrees so the handle points to the top right, shrink a bit
    # so the brush reads as "small" on its black tile.
    a = math.radians(45)
    s = 0.95 if size <= 24 * SS else 0.86 if size <= 48 * SS else 0.78
    out = []
    for x, y in pts:
        x, y = x - 50, y - 52
        rx = x * math.cos(a) - y * math.sin(a)
        ry = x * math.sin(a) + y * math.cos(a)
        out.append(((50 + rx * s) / 100 * size, (50 + ry * s) / 100 * size))
    return out

def render(size):
    big = size * SS
    img = Image.new('RGBA', (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    radius = big * 0.22
    d.rounded_rectangle([0, 0, big - 1, big - 1], radius=radius, fill=(0, 0, 0, 255))
    stroke = max(size * 0.05, 1.2) * SS
    for part in brush_parts():
        pts = transform(part, big)
        d.line(pts + [pts[0], pts[1]], fill=(255, 255, 255, 255), width=round(stroke), joint='curve')
    return img.resize((size, size), Image.LANCZOS)

out = sys.argv[1]
images = {s: render(s) for s in SIZES}
images[256].save(f'{out}/icon.png')
images[256].save(f'{out}/icon.ico', sizes=[(s, s) for s in SIZES],
                 append_images=[images[s] for s in SIZES if s != 256])
# Preview strip on grey (preview.png), small sizes also enlarged.
prev = Image.new('RGBA', (1000, 560), (128, 128, 128, 255))
x = 10
for s in SIZES:
    prev.alpha_composite(images[s], (x, 10)); x += s + 10
x = 10
for s in [16, 24, 32]:
    big = images[s].resize((s * 6, s * 6), Image.NEAREST)
    prev.alpha_composite(big, (x, 300)); x += s * 6 + 20
prev.save(f'{out}/preview.png')
print('ok')
