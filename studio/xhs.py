"""Composes a set of 3:4 pictures for a Xiaohongshu post from the studio's
renders (see xhs-shots.json and shots.json): the cover, the two ways to
open the panel, what it shows, the three looks, the small things, and how
light it is and where to get it.

    python studio/xhs.py FONT.ttf OUT-FOLDER
"""
import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

sys.path.insert(0, str(Path(__file__).parent))
from compose import ACCENT, Fonts, cutout, mark, shadowed  # noqa: E402

W, H = 1242, 1656
MARGIN = 84
WHITE = (255, 255, 255, 255)
SOFT = (226, 231, 242, 235)
DIM = (190, 198, 214, 220)
FRAMES = Path("target/studio/frames")


def placed(folder):
    """A still's frame and where its panel is."""
    folder = Path(folder)
    return Image.open(folder / "00001.png").convert("RGB"), json.loads((folder / "lanes.json").read_text())


def ground(blur=12, darken=0.3):
    """The wallpaper, softened, as a page's ground."""
    wall = Image.open("target/studio/wall-xhs-dark.png").convert("RGB")
    wall = wall.filter(ImageFilter.GaussianBlur(blur))
    return Image.blend(wall, Image.new("RGB", wall.size, (6, 8, 16)), darken).convert("RGBA")


# Marks that may not begin a line, as Chinese typesetting has it: they stay
# with the character before them, which goes down with them.
CLOSING = set("，。、；：！？）》」』”’%")


def wrap(draw, text, font, width):
    """`text` broken into lines no wider than `width`, at any character
    (Chinese breaks anywhere) but never before a closing mark; explicit
    line breaks kept."""
    lines = []
    for paragraph in text.split("\n"):
        line = ""
        for char in paragraph:
            if line and draw.textlength(line + char, font=font) > width:
                if char in CLOSING and len(line) > 1:
                    lines.append(line[:-1])
                    line = line[-1] + char
                else:
                    lines.append(line)
                    line = char.lstrip()
            else:
                line += char
        lines.append(line)
    return lines


def write(draw, xy, text, font, fill, width=None, spacing=1.45):
    """Draws `text` from `xy`, wrapped to `width`; returns where it ends."""
    x, y = xy
    size = font.size
    for line in wrap(draw, text, font, width) if width else text.split("\n"):
        draw.text((x, y), line, font=font, fill=fill)
        y += size * spacing
    return y


def heading(layer, fonts, title, subtitle=None, top=120):
    """A page's title, the accent rule under it, and a line beneath."""
    draw = ImageDraw.Draw(layer)
    y = write(draw, (MARGIN, top), title, fonts.get(80, b"Bold"), WHITE, spacing=1.3)
    draw.rounded_rectangle((MARGIN, y + 6, MARGIN + 140, y + 17), radius=5, fill=ACCENT + (255,))
    y += 52
    if subtitle:
        y = write(draw, (MARGIN, y), subtitle, fonts.get(38, b"Regular"), SOFT)
    return y


def card(layer, box, radius=40):
    """A frosted card: a light veil with a fine rim."""
    veil = Image.new("RGBA", layer.size, (0, 0, 0, 0))
    ImageDraw.Draw(veil).rounded_rectangle(box, radius=radius, fill=(255, 255, 255, 24), outline=(255, 255, 255, 58), width=2)
    layer.alpha_composite(veil)


def finish(page, layer):
    page.alpha_composite(shadowed(layer, blur=16, strength=0.45))
    return page.convert("RGB")


def cover(fonts):
    frame, _ = placed("target/studio/xhs-frames/xhs-cover")
    page = frame.convert("RGBA")
    # Darker towards the left, where the words are.
    # linear_gradient runs dark to light downwards; turned, light to the
    # left: darkest at the left edge, clear past the middle.
    shade = Image.linear_gradient("L").rotate(-90, expand=True).resize((W, H))
    shade = shade.point(lambda v: int(170 * max(0.0, (v / 255 - 0.45) / 0.55) ** 1.4))
    page.alpha_composite(Image.merge("RGBA", (*Image.new("RGB", (W, H), (4, 6, 14)).split(), shade)))
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    layer.alpha_composite(mark(132, 1.0, 1.0), (MARGIN, 190))
    y = write(draw, (MARGIN, 400), "藏在\n屏幕边缘的\n系统监控", fonts.get(112, b"Bold"), WHITE, spacing=1.28)
    draw.rounded_rectangle((MARGIN, y + 10, MARGIN + 170, y + 24), radius=6, fill=ACCENT + (255,))
    y = write(draw, (MARGIN, y + 74), "鼠标一推就出来\n移开就收回", fonts.get(50, b"Medium"), SOFT, spacing=1.5)
    write(draw, (MARGIN, H - 250), "Glance", fonts.get(72, b"Bold"), WHITE)
    write(draw, (MARGIN, H - 150), "免费 · 开源 · Windows 10/11", fonts.get(38, b"Regular"), SOFT)
    return finish(page, layer)


def keycap(draw, x, y, label, font):
    """A key, `label` on it; returns its right edge."""
    width = max(draw.textlength(label, font=font) + 60, 120)
    draw.rounded_rectangle((x, y + 8, x + width, y + 128), radius=22, fill=(10, 14, 26, 200))
    draw.rounded_rectangle((x, y, x + width, y + 118), radius=22, fill=(250, 251, 255, 245), outline=(255, 255, 255, 255), width=2)
    draw.text((x + width / 2, y + 58), label, font=font, anchor="mm", fill=(24, 28, 40, 255))
    return x + width


def ways(fonts):
    page = ground()
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    heading(layer, fonts, "两种打开方式")
    title, body = fonts.get(54, b"Bold"), fonts.get(36, b"Regular")

    # One: a small screen, the panel at its right edge, the pointer pushing.
    top = 330
    card(layer, (MARGIN - 20, top, W - MARGIN + 20, top + 700))
    screen = (MARGIN + 30, top + 40, W - MARGIN - 30, top + 420)
    sw, sh = screen[2] - screen[0], screen[3] - screen[1]
    wall = Image.open("target/studio/wall-xhs-dark.png").convert("RGB").resize((sw, int(sw * H / W))).crop((0, 0, sw, sh))
    shot = Image.new("RGBA", (sw, sh))
    shot.paste(wall)
    frame, where = placed(FRAMES / "still-glass")
    panel = cutout(frame, where, through="memory")
    scale = (sh - 40) / panel.height
    panel = panel.resize((int(panel.width * scale), int(panel.height * scale)), Image.LANCZOS)
    shot.alpha_composite(shadowed(Image.new("RGBA", shot.size), blur=1), (0, 0))
    holder = Image.new("RGBA", shot.size, (0, 0, 0, 0))
    holder.alpha_composite(panel, (sw - panel.width - 10, 20))
    shot.alpha_composite(shadowed(holder, blur=10, strength=0.5))
    rounded = Image.new("L", shot.size, 0)
    ImageDraw.Draw(rounded).rounded_rectangle((0, 0, sw - 1, sh - 1), radius=26, fill=255)
    shot.putalpha(rounded)
    layer.alpha_composite(shot, (screen[0], screen[1]))
    # The pointer, and the way it is pushed: towards the edge.
    px, py = screen[2] - panel.width - 60, screen[1] + sh // 2 + 40
    arrow = [(0, 0), (0, 17), (4, 13), (7, 20), (10, 19), (7, 12), (12, 12)]
    s = 3.2
    draw.polygon([(px + x * s, py + y * s) for x, y in arrow], fill=WHITE, outline=(20, 20, 24, 255), width=3)
    draw.line((px - 150, py + 20, px - 30, py + 20), fill=(255, 255, 255, 150), width=4)
    draw.polygon([(px - 30, py + 8), (px - 6, py + 20), (px - 30, py + 32)], fill=(255, 255, 255, 150))
    y = write(draw, (MARGIN + 30, top + 460), "推一下屏幕边缘", title, WHITE)
    write(draw, (MARGIN + 30, y + 4), "鼠标往右边缘推一下，面板就滑出来；移开自动收起。要有意推一下才会打开，贴边的滚动条照常好用。", body, SOFT, width=W - 2 * MARGIN - 60)

    # Two: the shortcut.
    top = 1070
    card(layer, (MARGIN - 20, top, W - MARGIN + 20, top + 480))
    key = fonts.get(50, b"SemiBold")
    x = MARGIN + 30
    for i, label in enumerate(["Ctrl", "Alt", "G"]):
        if i:
            draw.text((x + 34, top + 108), "+", font=fonts.get(56, b"Bold"), anchor="mm", fill=WHITE)
            x += 68
        x = keycap(draw, x, top + 50, label, key)
    y = write(draw, (MARGIN + 30, top + 230), "或者按快捷键", title, WHITE)
    write(draw, (MARGIN + 30, y + 4), "在鼠标所在位置打开面板，再按一次收起；面板固定时也能收起。", body, SOFT, width=W - 2 * MARGIN - 60)
    return finish(page, layer)


LANES = {
    "cpu": ("CPU", "占用 · 频率 · 功耗\n每个线程的负载"),
    "gpu:0": ("显卡", "占用 · 显存\n功耗 · 风扇 · 频率"),
    "memory": ("内存", "用量 · 每根内存条温度"),
    "network": ("网络", "实时上下行速度"),
    "disk": ("磁盘", "读写速度 · 硬盘温度"),
    "processes": ("进程", "最忙的程序，点表头排序"),
    "storage": ("存储", "各分区剩余空间"),
    "board": ("主板", "各处温度 · 风扇转速"),
}


def shows(fonts):
    page = ground()
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    heading(layer, fonts, "一个面板，全都看完")
    frame, where = placed(FRAMES / "still-glass")
    panel = cutout(frame, where)
    height = H - 300 - 60
    scale = height / panel.height
    panel = panel.resize((int(panel.width * scale), height), Image.LANCZOS)
    left, top = W - MARGIN + 30 - panel.width, 300
    holder = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    holder.alpha_composite(panel, (left, top))
    page.alpha_composite(shadowed(holder, blur=18, strength=0.55))
    # Each lane named at its middle, on the left, a line across to it.
    name, what = fonts.get(46, b"Bold"), fonts.get(30, b"Regular")
    origin = (where["panel"]["x"], where["panel"]["y"])
    previous = 0
    for lane_id, (title, detail) in LANES.items():
        lane = where["lanes"].get(lane_id)
        if lane is None:
            continue
        middle = top + (lane["y"] - origin[1] + lane["h"] / 2) * scale
        lines = detail.count("\n") + 1
        block = 58 + lines * 42
        y = max(middle - block / 2, previous + 16)
        previous = y + block
        draw.text((MARGIN, y), title, font=name, fill=WHITE)
        write(draw, (MARGIN, y + 60), detail, what, DIM, spacing=1.4)
        reach = MARGIN + 6 + max(draw.textlength(line, font=what) for line in detail.split("\n"))
        reach = max(reach, MARGIN + draw.textlength(title, font=name)) + 24
        draw.line((reach, y + 30, left - 14, middle), fill=(255, 255, 255, 110), width=2)
        draw.ellipse((left - 20, middle - 6, left - 8, middle + 6), fill=ACCENT + (255,))
    return finish(page, layer)


def looks(fonts):
    page = ground()
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    heading(layer, fonts, "三种外观，随你换", "深浅色也可以跟随壁纸自动切换")
    crops = [cutout(*placed(FRAMES / name)) for name in ("still-paper", "still-glass", "still-fluent")]
    gap = 30
    column = (W - 2 * 56 - 2 * gap) / 3
    top = 360
    # As large as the columns allow, and short enough for the names below.
    scale = min(column / max(crop.width for crop in crops), (H - top - 130) / max(crop.height for crop in crops))
    cards = [crop.resize((round(crop.width * scale), round(crop.height * scale)), Image.LANCZOS) for crop in crops]
    holder = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    lowest = top + max(c.height for c in cards)
    for i, (c, label) in enumerate(zip(cards, ["记录纸", "磨砂玻璃", "Windows 11"])):
        centre = 56 + column / 2 + i * (column + gap)
        holder.alpha_composite(c, (int(centre - c.width / 2), top))
        draw.text((centre, lowest + 56), label, font=fonts.get(42, b"SemiBold"), anchor="mm", fill=WHITE)
    page.alpha_composite(shadowed(holder, blur=18, strength=0.6))
    return finish(page, layer)


DETAILS = [
    ("图钉固定", "点面板底栏的图钉，面板就一直显示，边干活边盯着温度"),
    ("托盘看一眼", "鼠标停在托盘图标上，CPU、显卡、内存的读数直接显示"),
    ("过热提醒", "CPU 或显卡持续 30 秒达到警示温度，托盘提醒你（默认关闭）"),
    ("自动更新", "每天检查一次新版本，只运行签名一致的安装包，可关闭"),
    ("诊断信息", "遇到问题一键复制硬件和读数情况，方便反馈"),
]


def details(fonts):
    page = ground()
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    heading(layer, fonts, "顺手的小细节")
    title, body = fonts.get(48, b"Bold"), fonts.get(34, b"Regular")
    top, tall, gap = 330, 228, 26
    for i, (name, what) in enumerate(DETAILS):
        y = top + i * (tall + gap)
        card(layer, (MARGIN - 20, y, W - MARGIN + 20, y + tall), radius=34)
        draw.rounded_rectangle((MARGIN + 18, y + 44, MARGIN + 28, y + tall - 44), radius=5, fill=ACCENT + (255,))
        draw.text((MARGIN + 60, y + 36), name, font=title, fill=WHITE)
        write(draw, (MARGIN + 60, y + 110), what, body, SOFT, width=W - 2 * MARGIN - 90, spacing=1.4)
    return finish(page, layer)


def light(fonts):
    page = ground()
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    heading(layer, fonts, "很轻，而且免费")
    stats = [("≈1 MB", "程序大小"), ("≈55 MB", "内存占用"), ("<1%", "CPU 占用（面板收起时）"), ("免费", "无广告 · 无需账号")]
    gap = 30
    width = (W - 2 * MARGIN - gap) / 2
    for i, (value, what) in enumerate(stats):
        x = MARGIN + (i % 2) * (width + gap)
        y = 330 + (i // 2) * (290 + gap)
        card(layer, (x, y, x + width, y + 290), radius=36)
        draw.text((x + 44, y + 50), value, font=fonts.get(92, b"Bold"), fill=WHITE)
        write(draw, (x + 46, y + 190), what, fonts.get(32, b"Regular"), SOFT, width=width - 80)
    top = 1000
    card(layer, (MARGIN - 20, top, W - MARGIN + 20, top + 480), radius=40)
    draw.text((MARGIN + 30, top + 50), "去哪下载", font=fonts.get(54, b"Bold"), fill=WHITE)
    draw.rounded_rectangle((MARGIN + 30, top + 136, MARGIN + 150, top + 146), radius=5, fill=ACCENT + (255,))
    draw.text((MARGIN + 30, top + 180), "GitHub：lulu-loopp/glance", font=fonts.get(46, b"SemiBold"), fill=WHITE)
    write(
        draw,
        (MARGIN + 30, top + 262),
        "在 Releases 页下载 Glance 安装程序\n安装包已签名 · MIT 开源 · Windows 10/11",
        fonts.get(34, b"Regular"),
        SOFT,
        spacing=1.55,
    )
    layer.alpha_composite(mark(96, 1.0, 1.0), (W - MARGIN - 96, top + 44))
    return finish(page, layer)


if __name__ == "__main__":
    font, out = sys.argv[1], Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    fonts = Fonts(font)
    pages = [cover, ways, shows, looks, details, light]
    for i, make in enumerate(pages, 1):
        path = out / f"{i}-{make.__name__}.png"
        make(fonts).save(path)
        print(path)
