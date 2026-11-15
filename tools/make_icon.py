"""Draws Fetchbox's icon (fetchbox.ico + icon.png). Run once: .venv\\Scripts\\python make_icon.py"""
from pathlib import Path
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent.parent  # the Fetchbox folder
S = 1024  # draw big, downsample for crisp edges


def draw(size: int) -> Image.Image:
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    k = S / size

    def sn(v):  # snap to the target pixel grid so small sizes stay sharp instead of half-pixel blurry
        return round(v / k) * k

    def rect(x0, y0, x1, y1, fill, r):
        d.rounded_rectangle([sn(x0), sn(y0), sn(x1) - 1, sn(y1) - 1], radius=min(r, (sn(x1) - sn(x0)) / 2), fill=fill)

    # Background: rounded square with a soft top-to-bottom gradient.
    grad = Image.new("RGBA", (S, S))
    gd = ImageDraw.Draw(grad)
    for y in range(S):
        t = y / S
        c = int(46 - 26 * t)
        gd.line([(0, y), (S, y)], fill=(c, c, c + 4, 255))
    mask = Image.new("L", (S, S), 0)
    pad = 40
    ImageDraw.Draw(mask).rounded_rectangle([pad, pad, S - pad, S - pad], radius=230, fill=255)
    img.paste(grad, (0, 0), mask)
    if size > 16:
        d.rounded_rectangle([pad, pad, S - pad, S - pad], radius=230, outline=(255, 255, 255, 28), width=8)

    white = (245, 245, 245, 255)
    small = size <= 48
    # Stroke width: a whole number of pixels at small sizes (2px at 16, 3px at 24, 4px at 32).
    w = max(2, round(size * 0.125)) * k if small else 92
    rr = 0 if size <= 24 else w / 2   # square ends at tiny sizes read crisper than blurry round ones

    # Down arrow (centred on an odd/even-safe column so the shaft fills whole pixels).
    cx = sn(S / 2 - w / 2) + w / 2
    head = 230 if small else 200
    tray_top, tray_bot, tx = 590, 800, 240
    # Small sizes: keep a 1px gap between the arrow tip and the tray floor so they don't merge.
    tip = sn(tray_bot - w) - k if small else sn(690)
    base = tip - sn(220 if small else 220)
    d.polygon([(sn(cx - head), base), (sn(cx + head), base), (cx, tip)], fill=white)
    rect(cx - w / 2, 200 if small else 230, cx + w / 2, base + k, white, rr)

    # Tray the arrow drops into.
    rect(tx, tray_bot - w, S - tx, tray_bot, white, rr)
    rect(tx, tray_top, tx + w, tray_bot, white, rr)
    rect(S - tx - w, tray_top, S - tx, tray_bot, white, rr)

    # Green "ready" dot — echoes the status light in the app header (dropped at 16px where it's just noise).
    if size > 16:
        r = 74 if not small else max(2, round(size * 0.09)) * k
        x1, y0 = sn(S - pad - 60), sn(pad + 60)
        d.ellipse([x1 - 2 * r, y0, x1 - 1, y0 + 2 * r - 1], fill=(63, 185, 80, 255))

    # Box filter = exact area average of each target pixel: grid-aligned edges stay hard, no ringing.
    return img.resize((size, size), Image.BOX)


if __name__ == "__main__":
    # Every size Windows asks for at 100-200% display scaling, so nothing gets stretched.
    sizes = [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 96, 128, 256]
    frames = [draw(s) for s in sizes]
    frames[-1].save(HERE / "fetchbox.ico", sizes=[(s, s) for s in sizes], append_images=frames[:-1])
    draw(512).save(HERE / "icon.png")
    print("wrote fetchbox.ico and icon.png")
