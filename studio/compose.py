"""Composes the promotional video from the studio's frames (see studio.rs)
and the scenes in video.json: captions, the pointer, the looks crossfading,
a pan over the details, and two cards, encoded as a vertical H.264 MP4.

    python studio/compose.py studio/video.json FONT.ttf [--preview SECONDS]

Needs Pillow, numpy and imageio-ffmpeg. --preview writes one still per
scene at that many seconds in, instead of the video.
"""
import json
import math
import subprocess
import sys
from pathlib import Path

import imageio_ffmpeg
import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

W, H = 1080, 1920
MARGIN = 64


def ease(x):
    """Ease in and out, 0 to 1."""
    x = min(max(x, 0.0), 1.0)
    return x * x * (3 - 2 * x)


def ease_out(x):
    x = min(max(x, 0.0), 1.0)
    return 1 - (1 - x) ** 3


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
    """A shot's frames, read on demand; past its end, its last frame."""

    def __init__(self, folder):
        self.files = sorted(Path(folder).glob("*.png"))
        if not self.files:
            raise SystemExit(f"no frames in {folder}")
        self.cache = {}

    def frame(self, index):
        index = min(max(index, 0), len(self.files) - 1)
        if index not in self.cache:
            if len(self.cache) > 8:
                self.cache.clear()
            self.cache[index] = Image.open(self.files[index]).convert("RGB")
        return self.cache[index]


def wrap(draw, text, font, width):
    """Lines of `text` no wider than `width`: its own breaks, then by character."""
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


def caption(frame, fonts, title, subtitle, t, length, top=300, width=540):
    """The scene's title and subtitle at the upper left, rising and fading in,
    fading out at the scene's end."""
    if not title and not subtitle:
        return frame
    appear = ease_out((t - 0.25) / 0.5)
    leave = 1.0 - ease((t - (length - 0.35)) / 0.35)
    alpha = appear * leave
    if alpha <= 0.0:
        return frame
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    rise = (1.0 - appear) * 28
    big = fonts.get(78, b"Bold")
    small = fonts.get(36, b"Regular")
    y = top + rise
    for line in wrap(draw, title, big, width):
        draw.text((MARGIN, y), line, font=big, fill=(255, 255, 255, int(255 * alpha)))
        y += 100
    if subtitle:
        y += 18
        for line in wrap(draw, subtitle, small, width):
            draw.text((MARGIN, y), line, font=small, fill=(235, 238, 245, int(220 * alpha)))
            y += 52
    # A soft shadow under the words, for any background.
    shadow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    shadow.putalpha(layer.getchannel("A").filter(ImageFilter.GaussianBlur(14)).point(lambda a: a * 0.55))
    out = frame.convert("RGBA")
    out.alpha_composite(shadow)
    out.alpha_composite(layer)
    return out.convert("RGB")


def pointer(frame, x, y, alpha):
    """An arrow pointer with its tip at (x, y)."""
    if alpha <= 0.0:
        return frame
    s = 2.4
    arrow = [(0, 0), (0, 17), (4, 13), (7, 20), (10, 19), (7, 12), (12, 12)]
    points = [(x + px * s, y + py * s) for px, py in arrow]
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    draw.polygon(points, fill=(255, 255, 255, int(255 * alpha)), outline=(20, 20, 24, int(255 * alpha)), width=3)
    out = frame.convert("RGBA")
    out.alpha_composite(layer)
    return out.convert("RGB")


def edge_glow(frame, y, alpha):
    """A soft light along the right edge where the pointer pushes."""
    if alpha <= 0.0:
        return frame
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    draw.ellipse((W - 60, y - 220, W + 60, y + 260), fill=(140, 190, 255, int(200 * alpha)))
    layer = layer.filter(ImageFilter.GaussianBlur(40))
    out = frame.convert("RGBA")
    out.alpha_composite(layer)
    return out.convert("RGB")


def pill(frame, fonts, text, x, y, alpha):
    """A small label on a frosted pill."""
    if alpha <= 0.0:
        return frame
    font = fonts.get(40, b"SemiBold")
    layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    width = draw.textlength(text, font=font)
    draw.rounded_rectangle((x, y, x + width + 56, y + 78), radius=39, fill=(255, 255, 255, int(46 * alpha)), outline=(255, 255, 255, int(90 * alpha)), width=2)
    draw.text((x + 28, y + 14), text, font=font, fill=(255, 255, 255, int(255 * alpha)))
    out = frame.convert("RGBA")
    out.alpha_composite(layer)
    return out.convert("RGB")


def muted(frame, blur=26, darken=0.45):
    """The footage pushed back behind a card."""
    return Image.blend(frame.filter(ImageFilter.GaussianBlur(blur)), Image.new("RGB", (W, H), (6, 8, 16)), darken)


class Video:
    def __init__(self, spec, font):
        self.spec = spec
        self.fps = spec["fps"]
        self.fonts = Fonts(font)
        self.frames = Path(spec["frames"])
        self.footage = {}

    def shot(self, name):
        if name not in self.footage:
            self.footage[name] = Footage(self.frames / name)
        return self.footage[name]

    def scene_frame(self, scene, t):
        kind, length = scene["kind"], scene["seconds"]
        n = int(t * self.fps)
        if kind == "intro":
            frame = self.shot(scene["shot"]).frame(n)
            # The pointer travels to the right edge and pushes against it; a
            # glow marks the push, and the panel comes.
            move = ease((t - 0.3) / 0.9)
            push = ease((t - 1.2) / 0.15)
            x = 420 + (W - 40 - 420) * move + 26 * push
            y = 1150 - 140 * move
            frame = edge_glow(frame, y, ease((t - 1.15) / 0.2) * (1.0 - ease((t - 1.7) / 0.5)))
            frame = pointer(frame, x, y, 1.0 - ease((t - 2.6) / 0.5))
        elif kind == "footage":
            frame = self.shot(scene["shot"]).frame(n)
        elif kind == "looks":
            shots = scene["shots"]
            each = length / len(shots)
            index = min(int(t / each), len(shots) - 1)
            # Each look slides in and away as the panel does (see shots.json).
            frame = self.shot(shots[index]).frame(int((t - index * each) * self.fps))
            local = t - index * each
            frame = pill(frame, self.fonts, scene["labels"][index], MARGIN, 1560, ease_out(local / 0.3) * (1 - ease((local - each + 0.25) / 0.25)))
        elif kind == "detail":
            big = self.shot(scene["shot"]).frame(n)
            bw, bh = big.size
            # Down the panel, from the CPU to the motherboard.
            go = ease((t - 0.4) / (length - 0.8))
            left = bw - W
            top = (bh - H) * go
            frame = big.crop((left, int(top), left + W, int(top) + H))
        elif kind == "stats":
            frame = self.stats(scene, t)
        elif kind == "end":
            frame = self.end(scene, t)
        else:
            raise SystemExit(f"unknown scene kind {kind}")
        if kind not in ("stats", "end"):
            frame = caption(frame, self.fonts, scene.get("title", ""), scene.get("subtitle", ""), t, length, width=scene.get("width", 540))
        # Each scene comes in from black for a few frames, to cut cleanly.
        fade = ease(t / 0.15) if scene is not self.spec["scenes"][0] else ease(t / 0.4)
        if fade < 1.0:
            frame = Image.blend(Image.new("RGB", (W, H)), frame, fade)
        return frame

    def stats(self, scene, t):
        frame = muted(self.shot(scene["shot"]).frame(int(t * self.fps)))
        frame = frame.convert("RGBA")
        layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        draw = ImageDraw.Draw(layer)
        appear = ease_out((t - 0.2) / 0.5)
        title = self.fonts.get(84, b"Bold")
        draw.text((MARGIN, 330 + (1 - appear) * 28), scene["title"], font=title, fill=(255, 255, 255, int(255 * appear)))
        number = self.fonts.get(96, b"Bold")
        label = self.fonts.get(36, b"Regular")
        for i, (value, what) in enumerate(scene["stats"]):
            a = ease_out((t - 0.6 - 0.25 * i) / 0.45)
            y = 560 + i * 270 + (1 - a) * 30
            draw.rounded_rectangle((MARGIN, y, W - MARGIN, y + 230), radius=36, fill=(255, 255, 255, int(26 * a)), outline=(255, 255, 255, int(60 * a)), width=2)
            draw.text((MARGIN + 48, y + 34), value, font=number, fill=(255, 255, 255, int(255 * a)))
            draw.text((MARGIN + 52, y + 158), what, font=label, fill=(225, 230, 240, int(230 * a)))
        frame.alpha_composite(layer)
        return frame.convert("RGB")

    def end(self, scene, t):
        frame = muted(self.shot(scene["shot"]).frame(int((t + 5) * self.fps)), blur=30, darken=0.55).convert("RGBA")
        layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        draw = ImageDraw.Draw(layer)
        a = ease_out((t - 0.2) / 0.6)
        name = self.fonts.get(190, b"Bold")
        width = draw.textlength(scene["title"], font=name)
        draw.text(((W - width) / 2, 700 + (1 - a) * 30), scene["title"], font=name, fill=(255, 255, 255, int(255 * a)))
        b = ease_out((t - 0.7) / 0.5)
        sub = self.fonts.get(46, b"Medium")
        width = draw.textlength(scene["subtitle"], font=sub)
        draw.text(((W - width) / 2, 960), scene["subtitle"], font=sub, fill=(235, 238, 245, int(235 * b)))
        c = ease_out((t - 1.2) / 0.5)
        foot = self.fonts.get(44, b"SemiBold")
        width = draw.textlength(scene["footer"], font=foot)
        x, y = (W - width) / 2, 1120
        draw.rounded_rectangle((x - 36, y - 18, x + width + 36, y + 74), radius=46, fill=(255, 255, 255, int(40 * c)), outline=(255, 255, 255, int(110 * c)), width=2)
        draw.text((x, y), scene["footer"], font=foot, fill=(255, 255, 255, int(255 * c)))
        frame.alpha_composite(layer)
        return frame.convert("RGB")

    def frames_in_order(self):
        for scene in self.spec["scenes"]:
            count = int(round(scene["seconds"] * self.fps))
            for i in range(count):
                yield self.scene_frame(scene, i / self.fps)

    def encode(self):
        out = self.spec["out"]
        command = [
            imageio_ffmpeg.get_ffmpeg_exe(), "-y", "-loglevel", "error",
            "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}", "-r", str(self.fps), "-i", "-",
            "-c:v", "libx264", "-preset", "slow", "-crf", "16", "-pix_fmt", "yuv420p",
            "-movflags", "+faststart", out,
        ]
        process = subprocess.Popen(command, stdin=subprocess.PIPE)
        total = sum(int(round(s["seconds"] * self.fps)) for s in self.spec["scenes"])
        for i, frame in enumerate(self.frames_in_order()):
            process.stdin.write(np.asarray(frame, dtype=np.uint8).tobytes())
            if i % 60 == 0:
                print(f"{i}/{total}", flush=True)
        process.stdin.close()
        if process.wait() != 0:
            raise SystemExit("ffmpeg failed")
        print(out)

    def preview(self, at, folder):
        Path(folder).mkdir(parents=True, exist_ok=True)
        for i, scene in enumerate(self.spec["scenes"]):
            self.scene_frame(scene, min(at, scene["seconds"] - 0.05)).save(Path(folder) / f"scene{i + 1}.png")


if __name__ == "__main__":
    spec = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    video = Video(spec, sys.argv[2])
    if len(sys.argv) > 4 and sys.argv[3] == "--preview":
        video.preview(float(sys.argv[4]), Path(spec["out"]).parent / "preview")
    else:
        video.encode()
