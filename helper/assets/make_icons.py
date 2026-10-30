"""Generate the tray/app icons: a Greek capital omega with a point in the centre.

icon_on.ico  — accent colour (keyboard connected)
icon_off.ico — grey (keyboard not found)
"""
from PIL import Image, ImageDraw, ImageFont

FONT_CANDIDATES = [
    r"C:\Windows\Fonts\segoeuib.ttf",  # Segoe UI Bold has a proper Ω
    r"C:\Windows\Fonts\arialbd.ttf",
    r"C:\Windows\Fonts\seguisym.ttf",
]


def font(size):
    for path in FONT_CANDIDATES:
        try:
            return ImageFont.truetype(path, size)
        except OSError:
            continue
    return ImageFont.load_default()


def draw(size, color):
    # Render at 4x and downsample for smooth edges.
    s = size * 4
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    f = font(int(s * 0.95))
    text = "\u03a9"
    bbox = d.textbbox((0, 0), text, font=f)
    w, h = bbox[2] - bbox[0], bbox[3] - bbox[1]
    x = (s - w) / 2 - bbox[0]
    y = (s - h) / 2 - bbox[1]
    d.text((x, y), text, font=f, fill=color)
    # The "point": a filled circle in the middle of the omega's bowl.
    r = s * 0.11
    cx, cy = s / 2, s * 0.47
    d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=color)
    return img.resize((size, size), Image.LANCZOS)


for name, color in (("icon_on", (217, 65, 76, 255)), ("icon_off", (150, 150, 150, 255))):
    frames = [draw(sz, color) for sz in (16, 24, 32, 48, 64, 128, 256)]
    frames[-1].save(f"assets/{name}.ico", sizes=[(f.width, f.height) for f in frames], append_images=frames[:-1])
    frames[-1].save(f"assets/{name}_preview.png")
    print("wrote", name)
