"""Composes the promotional video from the studio's frames (see studio.rs)
and the scenes in video.json, encoded as a vertical H.264 MP4.

    python studio/compose.py studio/video.json FONT.ttf [--preview T]

Scenes drift with a slow camera and cut with a whip pan; captions come in
character by character over an accent rule; the pointer's push ripples
along the edge; the looks slide in and away as the panel does, then stand
side by side; the camera visits the CPU, graphics and motherboard lanes
(where the studio says they are); the numbers count up; and the mark draws
its pulse at the end.

Needs Pillow, numpy and imageio-ffmpeg. --preview T writes one still per
scene at T seconds into it, and one of each whip, instead of the video.
"""
import json
import math
import re
import subprocess
import sys
from pathlib import Path

import imageio_ffmpeg
import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

W, H = 1080, 1920
MARGIN = 64
ACCENT = (255, 123, 74)
WHIP_FRAMES = 8


def clamp01(x):
    return min(max(x, 0.0), 1.0)


def ease(x):
    x = clamp01(x)
    return x * x * (3 - 2 * x)


def ease_out(x):
    x = clamp01(x)
    return 1 - (1 - x) ** 3


def ease_back(x):
    """Out, overshooting a little before it settles."""
    x = clamp01(x)
    c = 1.4
    return 1 + (c + 1) * (x - 1) ** 3 + c * (x - 1) ** 2


class Fonts:
    def __init__(self, path):
        self.path = path
        self.cache = {}

    def get(self, size, weight):
        key = (size, weight)
        if key not in self.cache:
            font = ImageFont.truetype(self.path, size)
            font.set_variation_by_name(weight)
            self.cache[key] = font
        return self.cache[key]


class Footage:
    """A shot's frames, read on demand (past its end, its last frame), and
    where its lanes rest, as the studio wrote them."""

    def __init__(self, folder):
        self.folder = Path(folder)
        self.files = sorted(self.folder.glob("*.png"))
        if not self.files:
            raise SystemExit(f"no frames in {folder}")
        self.cache = {}
        lanes = self.folder / "lanes.json"
        self.lanes = {lane["id"]: lane for lane in json.loads(lanes.read_text())} if lanes.exists() else {}

    def frame(self, index):
        index = min(max(index, 0), len(self.files) - 1)
        if index not in self.cache:
            if len(self.cache) > 6:
                self.cache.clear()
            self.cache[index] = Image.open(self.files[index]).convert("RGB")
        return self.cache[index]


def over(frame, layer):
    out = frame.convert("RGBA")
    out.alpha_composite(layer)
    return out.convert("RGB")


def camera(frame, zoom, cx=0.5, cy=0.5):
    """The frame seen `zoom` times closer, about (cx, cy) as fractions."""
    w, h = frame.size
    if zoom <= 1.0001 and (w, h) == (W, H):
        return frame
    cw, ch = w / zoom, h / zoom
    left = min(max(cx * w - cw / 2, 0), w - cw)
    top = min(max(cy * h - ch / 2, 0), h - ch)
    return frame.resize((W, H), Image.BILINEAR, box=(left, top, left + cw, top + ch))


def make_vignette():
    y, x = np.mgrid[0:H, 0:W].astype(np.float32)
    d = np.sqrt(((x - W / 2) / (W * 0.75)) ** 2 + ((y - H / 2) / (H * 0.7)) ** 2)
    layer = np.zeros((H, W, 4), np.uint8)
    layer[..., 3] = (np.clip(d - 0.55, 0, 1) * 150).astype(np.uint8)
    return Image.fromarray(layer, "RGBA")


def shadowed(layer, blur=14, strength=0.55):
    """`layer` with a soft shadow under what it draws."""
    shadow = Image.new("RGBA", layer.size, (0, 0, 0, 0))
    shadow.putalpha(layer.getchannel("A").filter(ImageFilter.GaussianBlur(blur)).point(lambda a: a * strength))
    shadow.alpha_composite(layer)
    return shadow


def wrap(draw, text, font, width):
    lines = []
    for paragraph in text.split("\n"):
        line = ""
        for char in paragraph:
            if draw.textlength(line + char, font=font) > width and line:
                lines.append(line)
                line = char
            else:
                line += char
        lines.append(line)
    return lines


def kinetic(fonts, title, subtitle, t, length, width, top, size=76):
    """A caption: the title's characters rising in one after another, an
    accent rule drawn out beneath, the subtitle after; all of it lifting
    away at the scene's end."""
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    if not title and not subtitle:
        return layer
    draw = ImageDraw.Draw(layer)
    big = fonts.get(size, b"Bold")
    small = fonts.get(38, b"Regular")
    leave = ease((t - (length - 0.3)) / 0.3)
    start, step = 0.15, 0.035
    y = top - 40 * leave
    count, widest = 0, 0
    for line in wrap(draw, title, big, width):
        x = MARGIN
        for char in line:
            a = ease_out((t - start - count * step) / 0.35) * (1 - leave)
            if a > 0:
                draw.text((x, y + (1 - a) * 34), char, font=big, fill=(255, 255, 255, int(255 * a)))
            x += draw.textlength(char, font=big)
            count += 1
        widest = max(widest, x - MARGIN)
        y += size * 1.27
    done = start + count * step
    rule = ease_out((t - done + 0.1) / 0.45) * (1 - leave)
    if rule > 0:
        y += 6
        draw.rounded_rectangle((MARGIN, y, MARGIN + max(widest * 0.42, 120) * rule, y + 9), radius=4, fill=ACCENT + (int(255 * (1 - leave)),))
    y += 36
    if subtitle:
        a = ease_out((t - done - 0.05) / 0.45) * (1 - leave)
        for line in wrap(draw, subtitle, small, width):
            draw.text((MARGIN, y + (1 - a) * 18), line, font=small, fill=(232, 236, 245, int(225 * a)))
            y += 54
    return shadowed(layer)


def pointer_layer(x, y, alpha):
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    if alpha <= 0:
        return layer
    s = 2.4
    arrow = [(0, 0), (0, 17), (4, 13), (7, 20), (10, 19), (7, 12), (12, 12)]
    ImageDraw.Draw(layer).polygon([(x + px * s, y + py * s) for px, py in arrow], fill=(255, 255, 255, int(255 * alpha)), outline=(20, 20, 24, int(255 * alpha)), width=3)
    return shadowed(layer, blur=6, strength=0.5)


def ripple_layer(x, y, t):
    """Rings spreading from where the pointer pushes, `t` seconds after,
    and a glow along the edge there."""
    glow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    g = clamp01(t / 0.15) * (1 - clamp01((t - 0.4) / 0.6))
    if g > 0:
        ImageDraw.Draw(glow).ellipse((W - 70, y - 260, W + 70, y + 260), fill=(140, 190, 255, int(210 * g)))
        glow = glow.filter(ImageFilter.GaussianBlur(40))
    rings = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(rings)
    for delay in (0.0, 0.16):
        e = (t - delay) / 0.75
        if 0 < e < 1:
            r = 30 + 420 * ease_out(e)
            draw.ellipse((x - r, y - r, x + r, y + r), outline=(170, 210, 255, int(200 * (1 - e))), width=max(int(6 - 3 * e), 2))
    glow.alpha_composite(rings)
    return glow


def pill_layer(fonts, text, cx, cy, alpha, size=40):
    """A label on a frosted pill, centred on (cx, cy)."""
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    if alpha <= 0:
        return layer
    font = fonts.get(size, b"SemiBold")
    draw = ImageDraw.Draw(layer)
    half_w, half_h = draw.textlength(text, font=font) / 2 + 30, size * 0.98
    draw.rounded_rectangle((cx - half_w, cy - half_h, cx + half_w, cy + half_h), radius=half_h, fill=(255, 255, 255, int(46 * alpha)), outline=(255, 255, 255, int(100 * alpha)), width=2)
    # Centred on the letters' middle, not on their top.
    draw.text((cx, cy), text, font=font, anchor="mm", fill=(255, 255, 255, int(255 * alpha)))
    return layer


def glow_box(rect, alpha, colour=ACCENT):
    """A glowing outline round `rect` (x, y, w, h)."""
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    if alpha <= 0:
        return layer
    x, y, w, h = rect
    box = (x - 10, y - 10, x + w + 10, y + h + 10)
    halo = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    ImageDraw.Draw(halo).rounded_rectangle(box, radius=34, outline=colour + (int(230 * alpha),), width=16)
    halo = halo.filter(ImageFilter.GaussianBlur(18))
    ImageDraw.Draw(layer).rounded_rectangle(box, radius=34, outline=colour + (int(255 * alpha),), width=4)
    halo.alpha_composite(layer)
    return halo


def muted(frame, blur=26, darken=0.45):
    return Image.blend(frame.filter(ImageFilter.GaussianBlur(blur)), Image.new("RGB", frame.size, (6, 8, 16)), darken)


def panel_crop(frame, wallpaper):
    """The panel cut out of a still, where it differs from the wallpaper,
    with rounded corners."""
    diff = np.abs(np.asarray(frame, np.int16) - np.asarray(wallpaper, np.int16)).sum(axis=2) > 60
    ys, xs = np.nonzero(diff)
    box = (max(xs.min() - 20, 0), max(ys.min() - 20, 0), min(xs.max() + 20, frame.width), min(ys.max() + 30, frame.height))
    crop = frame.crop(box).convert("RGBA")
    mask = Image.new("L", crop.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle((14, 14, crop.width - 14, crop.height - 20), radius=26, fill=255)
    crop.putalpha(mask.filter(ImageFilter.GaussianBlur(1)))
    return crop


def mark(size, progress, dot):
    """Glance's mark with its pulse drawn `progress` of the way and its dot
    at `dot` of its size (both 0 to 1), drawn large and scaled down."""
    big = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
    draw = ImageDraw.Draw(big)
    draw.rounded_rectangle((64, 64, 960, 960), radius=208, fill=(20, 35, 31, 255), outline=(58, 74, 69, 255), width=12)
    points = [(176, 640), (336, 640), (432, 400), (544, 720), (640, 304), (720, 560), (848, 560)]
    lengths = [math.dist(a, b) for a, b in zip(points, points[1:])]
    total = sum(lengths) * clamp01(progress)
    path = [points[0]]
    for (a, b), length in zip(zip(points, points[1:]), lengths):
        if total <= 0:
            break
        f = min(total / length, 1.0)
        path.append((a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f))
        total -= length
    if progress > 0 and len(path) > 1:
        draw.line(path, fill=(230, 237, 230, 255), width=64, joint="curve")
        for x, y in (path[0], path[-1]):
            draw.ellipse((x - 32, y - 32, x + 32, y + 32), fill=(230, 237, 230, 255))
    if dot > 0:
        r = 56 * dot
        draw.ellipse((848 - r, 560 - r, 848 + r, 560 + r), fill=ACCENT + (255,))
    return big.resize((size, size), Image.LANCZOS)


def whip(a, b, p):
    """Frame `a` leaving to the left and `b` coming in from the right, `p`
    of the way, smeared along the motion as a fast pan is: a continuous
    blur across the distance moved in a frame."""
    e = ease(p)
    both = np.concatenate([np.asarray(a, np.float32), np.asarray(b, np.float32)], axis=1)
    offset = int(round(W * e))
    smear = int(math.sin(math.pi * clamp01(p)) * 220)
    if smear < 2:
        return Image.fromarray(both[:, offset : offset + W].astype(np.uint8), "RGB")
    # A box blur along x through a running sum, over the frame and a margin.
    lo, hi = max(offset - smear, 0), min(offset + W + smear, 2 * W)
    strip = both[:, lo:hi]
    total = np.cumsum(np.pad(strip, ((0, 0), (1, 0), (0, 0))), axis=1)
    half = smear // 2
    xs = np.arange(offset - lo, offset - lo + W)
    left = np.clip(xs - half, 0, strip.shape[1])
    right = np.clip(xs + half + 1, 0, strip.shape[1])
    blurred = (total[:, right] - total[:, left]) / (right - left)[None, :, None]
    return Image.fromarray(np.clip(blurred, 0, 255).astype(np.uint8), "RGB")


class Video:
    def __init__(self, spec, font):
        self.spec = spec
        self.fps = spec["fps"]
        self.fonts = Fonts(font)
        self.frames = Path(spec["frames"])
        self.footage = {}
        self.vignette = make_vignette()
        self.wallpaper = Image.open(spec["wallpaper"]).convert("RGB")
        self.triptych = None

    def shot(self, name):
        if name not in self.footage:
            self.footage[name] = Footage(self.frames / name)
        return self.footage[name]

    def drift(self, frame, t, length, zoom=(1.0, 1.03), cx=0.82, cy=0.45):
        """The slow camera: a push in over the scene."""
        return camera(frame, zoom[0] + (zoom[1] - zoom[0]) * clamp01(t / length), cx, cy)

    def scene_frame(self, scene, t):
        kind, length = scene["kind"], scene["seconds"]
        n = int(t * self.fps)
        layers = []
        if kind == "intro":
            frame = self.drift(self.shot(scene["shot"]).frame(n), t, length, (1.0, 1.04))
            move = ease((t - 0.35) / 0.85)
            push = ease((t - 1.2) / 0.12)
            x = 420 + (W - 40 - 420) * move + 26 * push
            y = 1160 - 150 * move
            layers.append(ripple_layer(W, y, t - 1.25))
            layers.append(pointer_layer(x, y, 1 - ease((t - 2.8) / 0.5)))
        elif kind == "footage":
            frame = self.drift(self.shot(scene["shot"]).frame(n), t, length)
        elif kind == "looks":
            frame, extra = self.looks(scene, t)
            layers.extend(extra)
        elif kind == "detail":
            frame, extra = self.detail(scene, t)
            layers.extend(extra)
        elif kind == "stats":
            return self.stats(scene, t)
        elif kind == "end":
            return self.end(scene, t)
        else:
            raise SystemExit(f"unknown scene kind {kind}")
        frame = over(frame, self.vignette)
        for layer in layers:
            frame = over(frame, layer)
        caption = kinetic(self.fonts, scene.get("title", ""), scene.get("subtitle", ""), t, length, scene.get("width", 470), scene.get("top", 280))
        frame = over(frame, caption)
        if scene is self.spec["scenes"][0]:
            frame = Image.blend(Image.new("RGB", (W, H)), frame, ease(t / 0.5))
        return frame

    def looks(self, scene, t):
        shots, labels, each = scene["shots"], scene["labels"], scene["each"]
        sliding = each * len(shots)
        if t < sliding:
            index = min(int(t / each), len(shots) - 1)
            local = t - index * each
            frame = self.drift(self.shot(shots[index]).frame(int(local * self.fps)), t, scene["seconds"], (1.0, 1.03))
            a = ease_out((local - 0.05) / 0.25) * (1 - ease((local - each + 0.3) / 0.25))
            return frame, [pill_layer(self.fonts, labels[index], 230, 1600, a)]
        # Then the three side by side, flying in one after another.
        if self.triptych is None:
            self.triptych = [panel_crop(self.shot(name).frame(0), self.wallpaper) for name in scene["stills"]]
        local = t - sliding
        frame = muted(self.drift(self.wallpaper, t, scene["seconds"], (1.0, 1.03)), blur=8, darken=0.25)
        layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        gap = 22
        width = (W - 2 * 40 - 2 * gap) / 3
        # The labels on one line, under the tallest card.
        lowest = 620 + max(crop.height * width / crop.width for crop in self.triptych)
        for i, (crop, label) in enumerate(zip(self.triptych, labels)):
            a = ease_back((local - 0.08 * i) / 0.55)
            alpha = clamp01((local - 0.08 * i) / 0.25)
            if alpha <= 0:
                continue
            card = crop.resize((int(width), int(crop.height * width / crop.width)), Image.LANCZOS)
            if alpha < 1:
                card.putalpha(card.getchannel("A").point(lambda v: int(v * alpha)))
            x = int(40 + i * (width + gap))
            y = int(620 + (1 - a) * 900)
            shadow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
            ImageDraw.Draw(shadow).rounded_rectangle((x + 8, y + 24, x + card.width - 8, y + card.height), radius=24, fill=(0, 0, 0, int(140 * alpha)))
            layer.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(22)))
            layer.alpha_composite(card, (x, y))
            ImageDraw.Draw(layer).text((x + card.width / 2, lowest + 50 + (y - 620)), label, font=self.fonts.get(36, b"SemiBold"), anchor="mm", fill=(255, 255, 255, int(255 * alpha)))
        return frame, [layer]

    def detail(self, scene, t):
        footage = self.shot(scene["shot"])
        big = footage.frame(int(t * self.fps))
        bw, bh = big.size
        stops = scene["stops"]
        each = scene["seconds"] / len(stops)
        index = min(int(t / each), len(stops) - 1)
        local = t - index * each

        # The camera travels to each lane, holding it in the middle.
        def top_for(i):
            lane = footage.lanes[stops[i]["lane"]]
            return min(max(lane["y"] + lane["h"] / 2 - H * 0.5, 0), bh - H)

        previous = top_for(max(index - 1, 0))
        top = previous + (top_for(index) - previous) * ease(local / 0.6)
        left = bw - W
        frame = big.crop((left, int(top), left + W, int(top) + H))
        lane = footage.lanes[stops[index]["lane"]]
        rect = (lane["x"] - left, lane["y"] - top, lane["w"], lane["h"])
        a = ease_out((local - 0.45) / 0.35) * (1 - ease((local - each + 0.25) / 0.25))
        # Its name and what it shows, in the room to its left.
        label = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        draw = ImageDraw.Draw(label)
        cy = rect[1] + rect[3] / 2
        rise = (1 - a) * 20
        draw.rounded_rectangle((MARGIN, cy - 96 + rise, MARGIN + 70 * a, cy - 87 + rise), radius=4, fill=ACCENT + (int(255 * a),))
        draw.text((MARGIN, cy - 70 + rise), stops[index]["name"], font=self.fonts.get(64, b"Bold"), fill=(255, 255, 255, int(255 * a)))
        for j, line in enumerate(stops[index]["what"].split("\n")):
            draw.text((MARGIN, cy + 14 + j * 50 + rise), line, font=self.fonts.get(36, b"Regular"), fill=(232, 236, 245, int(230 * a)))
        return frame, [glow_box(rect, a), shadowed(label)]

    def stats(self, scene, t):
        length = scene["seconds"]
        frame = muted(self.drift(self.shot(scene["shot"]).frame(int(t * self.fps)), t, length, (1.04, 1.1)))
        layer = kinetic(self.fonts, scene["title"], "", t, length, 900, 300, size=84)
        draw = ImageDraw.Draw(layer)
        number = self.fonts.get(104, b"Bold")
        label = self.fonts.get(36, b"Regular")
        leave = ease((t - (length - 0.3)) / 0.3)
        for i, (value, what) in enumerate(scene["stats"]):
            begin = 0.55 + 0.18 * i
            a = ease_back((t - begin) / 0.5) * (1 - leave)
            alpha = clamp01((t - begin) / 0.25) * (1 - leave)
            if alpha <= 0:
                continue
            y = 560 + i * 280 + (1 - a) * 60
            # Its number counts up from nothing; a bound ("<1%") is no
            # amount to count to, and stays as it is.
            match = re.search(r"\d+", value)
            if match and "<" not in value:
                shown = int(round(int(match.group()) * ease_out((t - begin - 0.05) / 0.9)))
                value = value[: match.start()] + str(shown) + value[match.end():]
            draw.rounded_rectangle((MARGIN, y, W - MARGIN, y + 236), radius=38, fill=(255, 255, 255, int(24 * alpha)), outline=(255, 255, 255, int(58 * alpha)), width=2)
            draw.rounded_rectangle((MARGIN + 40, y + 40, MARGIN + 48, y + 196), radius=4, fill=ACCENT + (int(255 * alpha),))
            draw.text((MARGIN + 80, y + 30), value, font=number, fill=(255, 255, 255, int(255 * alpha)))
            draw.text((MARGIN + 84, y + 162), what, font=label, fill=(225, 230, 240, int(230 * alpha)))
        return over(frame, layer)

    def end(self, scene, t):
        frame = muted(self.drift(self.shot(scene["shot"]).frame(int((t + 4.5) * self.fps)), t, scene["seconds"], (1.08, 1.0)), blur=30, darken=0.6)
        layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        # The mark, its pulse drawn as a trace is, then its dot.
        size = 260
        icon = mark(size, (t - 0.25) / 0.8, ease_back((t - 1.0) / 0.35))
        appear = ease_out(t / 0.4)
        if appear < 1:
            icon.putalpha(icon.getchannel("A").point(lambda v: int(v * appear)))
        layer.alpha_composite(icon, ((W - size) // 2, 470))
        a = ease_out((t - 1.1) / 0.5)
        title = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        ImageDraw.Draw(title).text((W / 2, 900 + (1 - a) * 30), scene["title"], font=self.fonts.get(180, b"Bold"), anchor="mm", fill=(255, 255, 255, int(255 * a)))
        # A light sweeping across the name.
        sweep = (t - 1.6) / 0.9
        if 0 < sweep < 1:
            band = Image.new("L", (W, H), 0)
            cx = -200 + (W + 400) * sweep
            ImageDraw.Draw(band).polygon([(cx - 60, 700), (cx + 40, 700), (cx - 80, 1100), (cx - 180, 1100)], fill=170)
            band = band.filter(ImageFilter.GaussianBlur(22))
            shine = Image.new("RGBA", (W, H), (255, 236, 220, 0))
            shine.putalpha(Image.fromarray(np.minimum(np.asarray(band), np.asarray(title.getchannel("A")))))
            title.alpha_composite(shine)
        layer.alpha_composite(title)
        b = ease_out((t - 1.5) / 0.5)
        ImageDraw.Draw(layer).text((W / 2, 1050), scene["subtitle"], font=self.fonts.get(46, b"Medium"), anchor="mm", fill=(235, 238, 245, int(235 * b)))
        layer.alpha_composite(pill_layer(self.fonts, scene["footer"], W / 2, 1190, ease_out((t - 1.9) / 0.5), size=42))
        return over(frame, shadowed(layer, blur=18, strength=0.4))

    def timeline(self):
        """Each frame's maker, in order: the scenes', and a whip between."""
        scenes = self.spec["scenes"]
        for i, scene in enumerate(scenes):
            for k in range(int(round(scene["seconds"] * self.fps))):
                yield lambda scene=scene, k=k: self.scene_frame(scene, k / self.fps)
            if i + 1 < len(scenes):
                following = scenes[i + 1]
                ends = {}
                for k in range(WHIP_FRAMES):
                    p = (k + 1) / (WHIP_FRAMES + 1)

                    def make(scene=scene, following=following, p=p, ends=ends):
                        if not ends:
                            ends["a"] = self.scene_frame(scene, scene["seconds"] - 0.001)
                            ends["b"] = self.scene_frame(following, 0.0)
                        return whip(ends["a"], ends["b"], p)

                    yield make

    def encode(self):
        out = self.spec["out"]
        command = [
            imageio_ffmpeg.get_ffmpeg_exe(), "-y", "-loglevel", "error",
            "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}", "-r", str(self.fps), "-i", "-",
            "-c:v", "libx264", "-preset", "slow", "-crf", "16", "-pix_fmt", "yuv420p",
            "-movflags", "+faststart", out,
        ]
        process = subprocess.Popen(command, stdin=subprocess.PIPE)
        makers = list(self.timeline())
        for i, make in enumerate(makers):
            process.stdin.write(np.asarray(make(), dtype=np.uint8).tobytes())
            if i % 90 == 0:
                print(f"{i}/{len(makers)}", flush=True)
        process.stdin.close()
        if process.wait() != 0:
            raise SystemExit("ffmpeg failed")
        print(f"{out}: {len(makers) / self.fps:.1f} s")

    def preview(self, at, folder):
        folder = Path(folder)
        folder.mkdir(parents=True, exist_ok=True)
        scenes = self.spec["scenes"]
        for i, scene in enumerate(scenes):
            self.scene_frame(scene, min(at, scene["seconds"] - 0.05)).save(folder / f"scene{i + 1}.png")
            if i + 1 < len(scenes):
                whip(self.scene_frame(scene, scene["seconds"] - 0.001), self.scene_frame(scenes[i + 1], 0.0), 0.5).save(folder / f"whip{i + 1}.png")


if __name__ == "__main__":
    spec = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    video = Video(spec, sys.argv[2])
    if len(sys.argv) > 4 and sys.argv[3] == "--preview":
        video.preview(float(sys.argv[4]), Path(spec["out"]).parent / "preview")
    else:
        video.encode()
