"""Composes the README's pictures from the studio's renders (see the
readme-*.json scripts): the header, the three looks side by side and the
panel on a desktop, in English and Chinese, each with rounded corners
(GitHub cannot round a picture itself), as WebP.

    python studio/readme.py RENDERS FONT.ttf OUT-FOLDER
"""
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

sys.path.insert(0, str(Path(__file__).parent))
from compose import ACCENT, Fonts, cutout, mark, shadowed  # noqa: E402

WORDS = {
    "en": {
        "tagline": ["A system monitor at the edge of your screen.", "Push the pointer there; move away and it's gone."],
        "looks": ["Chart paper", "Frosted glass", "Windows 11"],
    },
    "zh": {
        "tagline": ["藏在屏幕边缘的系统监控", "鼠标推到边缘，面板滑出；移开即收回"],
        "looks": ["记录纸", "磨砂玻璃", "Windows 11"],
    },
}
CORNER = 36


def rounded(image, radius=CORNER):
    """`image` with its corners cut round, drawn large for smooth edges."""
    k = 4
    mask = Image.new("L", (image.width * k, image.height * k), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, mask.width - 1, mask.height - 1), radius=radius * k, fill=255)
    out = image.convert("RGBA")
    out.putalpha(mask.resize(image.size, Image.LANCZOS))
    return out


def shade(image, reach, strength):
    """`image` darkened from its left edge, fading out `reach` of the way."""
    x = np.linspace(0.0, 1.0, image.width, dtype=np.float32)
    fall = np.clip(1.0 - x / reach, 0.0, 1.0) ** 1.6 * strength
    pixels = np.asarray(image.convert("RGB"), np.float32) * (1.0 - fall)[None, :, None]
    return Image.fromarray(pixels.astype(np.uint8), "RGB")


def banner(renders, fonts, lang):
    """The header: the panel at the edge of the desktop, the name beside."""
    frame = shade(Image.open(renders / f"banner-{lang}" / "00001.png"), 0.62, 0.55)
    layer = Image.new("RGBA", frame.size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    size = 132
    left, top = 96, 150
    layer.alpha_composite(mark(size, 1.0, 1.0), (left, top))
    name = fonts.get(124, b"Bold")
    draw.text((left + size + 40, top + size / 2), "Glance", font=name, anchor="lm", fill=(255, 255, 255, 255))
    y = top + size + 52
    draw.rounded_rectangle((left, y, left + 120, y + 9), radius=4, fill=ACCENT + (255,))
    y += 44
    line = fonts.get(40, b"Medium" if lang == "zh" else b"Regular")
    for text in WORDS[lang]["tagline"]:
        draw.text((left, y), text, font=line, fill=(236, 239, 246, 240))
        y += 60
    out = frame.convert("RGBA")
    out.alpha_composite(shadowed(layer, blur=16, strength=0.45))
    return rounded(out)


def looks(renders, fonts, lang, wallpaper):
    """The three looks side by side at one scale, down to the memory lane,
    their names beneath, over the desktop."""
    names = ["paper", "glass", "fluent"]
    cards = []
    for name in names:
        folder = renders / f"looks-{name}-{lang}"
        placed = json.loads((folder / "lanes.json").read_text())
        cards.append(cutout(Image.open(folder / "00001.png"), placed, through="memory"))
    gap, side, top = 56, 72, 72
    width = sum(card.width for card in cards) + gap * (len(cards) - 1) + 2 * side
    tallest = max(card.height for card in cards)
    height = top + tallest + 150
    ground = wallpaper.resize((width, int(wallpaper.height * width / wallpaper.width)), Image.LANCZOS)
    ground = ground.crop((0, (ground.height - height) // 2, width, (ground.height - height) // 2 + height))
    ground = Image.blend(ground.filter(ImageFilter.GaussianBlur(10)), Image.new("RGB", ground.size, (6, 8, 16)), 0.3).convert("RGBA")
    layer = Image.new("RGBA", ground.size, (0, 0, 0, 0))
    x = side
    for card in cards:
        layer.alpha_composite(card, (x, top))
        x += card.width + gap
    ground.alpha_composite(shadowed(layer, blur=22, strength=0.6))
    draw = ImageDraw.Draw(ground)
    label = fonts.get(44, b"SemiBold")
    x = side
    for card, text in zip(cards, WORDS[lang]["looks"]):
        draw.text((x + card.width / 2, top + tallest + 75), text, font=label, anchor="mm", fill=(255, 255, 255, 255))
        x += card.width + gap
    return rounded(ground)


def desktop(renders, lang):
    return rounded(Image.open(renders / f"desktop-{lang}" / "00001.png"), radius=28)


if __name__ == "__main__":
    renders, font, out = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
    out.mkdir(parents=True, exist_ok=True)
    fonts = Fonts(font)
    wallpaper = Image.open("target/studio/wall-wide-dark.png").convert("RGB")
    for lang in ("en", "zh"):
        for name, picture in (("banner", banner(renders, fonts, lang)), ("looks", looks(renders, fonts, lang, wallpaper)), ("desktop", desktop(renders, lang))):
            path = out / f"{name}-{lang}.webp"
            picture.save(path, "WEBP", quality=92, method=6)
            print(f"{path}: {picture.width}x{picture.height}, {path.stat().st_size // 1024} KB")
