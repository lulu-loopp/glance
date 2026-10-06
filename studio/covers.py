"""Two independent Douyin cover layouts, composed with real studio panels.

    python studio/covers.py FONT.ttf OUT-FOLDER

The studio executable renders three enabled modules, including its real bottom
bar. Intermediate wallpapers, shot scripts and renders live in OUT-FOLDER/_work.
No panel UI is drawn by Pillow. Run from any working directory.
"""
import argparse
import json
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

from compose import ACCENT, Fonts, cutout, mark, shadowed
from wallpaper import wallpaper
from xhs import placed, wrap

ROOT = Path(__file__).resolve().parents[1]
HEADLINE = "藏在屏幕边缘的系统监控"
LINE = "鼠标一推就出来，移开就收回"
SMALL = "Glance · 免费开源 · Windows 10/11"
WHITE = (255, 255, 255, 255)
SOFT = (231, 236, 247, 255)


def ground(size, portrait):
    """The video's wallpaper palette, with room for white thumbnail type."""
    page = wallpaper(*size, "dark").filter(ImageFilter.GaussianBlur(4))
    page = Image.blend(page, Image.new("RGB", size, (6, 8, 18)), 0.23)
    # A gentle left veil gives the letters contrast without a text box.
    shade = Image.linear_gradient("L").rotate(-90, expand=True).resize(size)
    strength = 110 if portrait else 145
    shade = shade.point(lambda v: int(strength * (v / 255) ** 1.5))
    page = page.convert("RGBA")
    veil = Image.new("RGBA", size, (4, 7, 18, 0))
    veil.putalpha(shade)
    page.alpha_composite(veil)
    return page


def real_panel(page, work, name, width, top, right):
    """Render, cut along the native outline, and place at the screen edge.

    A second render aligns the sampled wallpaper with the final placement;
    the glass then shows the same glow as the surrounding cover wallpaper.
    Native scale is 2, so the final panel is downsampled, never enlarged.
    """
    exe = ROOT / "target/studio-build/release/glance.exe"
    if not exe.is_file():
        raise SystemExit(f"Studio executable missing: {exe}")
    wall_path = work / f"{name}-wall.png"
    spec_path = work / f"{name}-shot.json"
    folder = work / name
    # Plenty of space for the unscaled native layout (712 px wide).
    render_size = (1656, 1656)
    page.convert("RGB").resize(render_size, Image.Resampling.LANCZOS).save(wall_path)
    spec = {
        "width": render_size[0], "height": render_size[1],
        "scale": 2.0, "fps": 30, "wallpaper": str(wall_path),
        "shots": [{"name": name, "skin": "glass", "theme": "dark",
                   "seconds": 0.034, "load": [0.55, 0.55],
                   "modules": ["cpu", "gpu:0", "memory"]}],
    }
    spec_path.write_text(json.dumps(spec, indent=2), encoding="utf-8")

    def render():
        subprocess.run([str(exe), "--studio", str(spec_path), str(work)],
                       cwd=ROOT, check=True)

    render()
    _, where = placed(folder)
    rect = where["panel"]
    factor = width / int(rect["w"])
    left = right - width
    # Map native-render coordinates to final-cover coordinates. This also
    # supplies backdrop pixels outside the mask for the native glass blur.
    sample = page.convert("RGB").transform(
        render_size, Image.Transform.AFFINE,
        (factor, 0, left - rect["x"] * factor,
         0, factor, top - rect["y"] * factor),
        resample=Image.Resampling.BICUBIC,
    )
    sample.save(wall_path)
    render()
    panel = cutout(*placed(folder))
    height = round(panel.height * width / panel.width)
    panel = panel.resize((width, height), Image.Resampling.LANCZOS)
    assert left >= 0 and top + height <= page.height - 20
    holder = Image.new("RGBA", page.size)
    holder.alpha_composite(panel, (left, top))
    page.alpha_composite(shadowed(holder, blur=20, strength=0.65))
    return (left, top, right, top + height)


def text(draw, xy, words, font, fill=WHITE):
    """Position by visible glyph tops, and reject accidental clipping."""
    x, y = xy
    box = draw.textbbox((x, y), words, font=font, anchor="lt")
    assert box[0] >= 0 and box[1] >= 0
    assert box[2] <= draw._image.width and box[3] <= draw._image.height
    draw.text(xy, words, font=font, fill=fill, anchor="lt")
    return box


def supporting(draw, fonts, xy, width, size):
    y = xy[1]
    font = fonts.get(size, b"Medium")
    lines = wrap(draw, LINE, font, width)
    assert "".join(lines) == LINE
    for words in lines:
        text(draw, (xy[0], y), words, font, SOFT)
        y += round(size * 1.55)


def portrait(fonts, work):
    page = ground((1242, 1656), True)
    real_panel(page, work, "portrait", width=576, top=626, right=1218)
    layer = Image.new("RGBA", page.size)
    layer.alpha_composite(mark(112, 1.0, 1.0), (72, 72))
    draw = ImageDraw.Draw(layer)
    text(draw, (214, 111), SMALL, fonts.get(38, b"Medium"), SOFT)
    # A wide headline above the panel; the final four characters have
    # their own emphatic line. It isn't the Xiaohongshu side-column layout.
    lines = (HEADLINE[:-4], HEADLINE[-4:])
    assert "".join(lines) == HEADLINE
    text(draw, (78, 253), lines[0], fonts.get(142, b"Bold"))
    text(draw, (72, 427), lines[1], fonts.get(188, b"Bold"))
    draw.rounded_rectangle((80, 649, 274, 663), radius=7, fill=ACCENT + (255,))
    supporting(draw, fonts, (80, 746), width=470, size=55)
    page.alpha_composite(shadowed(layer, blur=14, strength=0.5))
    return page.convert("RGB")


def landscape(fonts, work):
    page = ground((1656, 1242), False)
    real_panel(page, work, "landscape", width=618, top=89, right=1628)
    layer = Image.new("RGBA", page.size)
    layer.alpha_composite(mark(124, 1.0, 1.0), (76, 83))
    draw = ImageDraw.Draw(layer)
    # The landscape composition is a broad left headline with a tall
    # right-hand product shot, not a crop of the portrait cover.
    lines = (HEADLINE[:-4], HEADLINE[-4:])
    assert "".join(lines) == HEADLINE
    text(draw, (80, 354), lines[0], fonts.get(124, b"Bold"))
    text(draw, (72, 519), lines[1], fonts.get(194, b"Bold"))
    draw.rounded_rectangle((80, 750, 278, 764), radius=7, fill=ACCENT + (255,))
    supporting(draw, fonts, (82, 830), width=850, size=55)
    text(draw, (82, 1119), SMALL, fonts.get(40, b"Medium"), SOFT)
    page.alpha_composite(shadowed(layer, blur=14, strength=0.5))
    return page.convert("RGB")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("font", type=Path)
    parser.add_argument("out_folder", type=Path)
    args = parser.parse_args()
    out = args.out_folder.resolve()
    work = out / "_work"
    work.mkdir(parents=True, exist_ok=True)
    fonts = Fonts(str(args.font.resolve()))
    for name, make in (("cover-3x4.png", portrait), ("cover-4x3.png", landscape)):
        path = out / name
        make(fonts, work).save(path)
        print(path)


if __name__ == "__main__":
    main()
