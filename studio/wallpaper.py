"""An original abstract wallpaper for the studio: soft glows of colour over
a deep ground, so that frosted glass and acrylic have something to show.

    python studio/wallpaper.py WIDTH HEIGHT OUT.png [light|dark]
"""
import sys

import numpy as np
from PIL import Image


def glow(x, y, cx, cy, radius):
    return np.exp(-((x - cx) ** 2 + (y - cy) ** 2) / (2.0 * radius ** 2))


def wallpaper(width, height, mood):
    y, x = np.mgrid[0:height, 0:width].astype(np.float32)
    x /= width
    y /= height
    # Ground, then glows: (centre x, centre y, radius, colour).
    if mood == "light":
        image = np.ones((height, width, 3), np.float32) * np.array([0.93, 0.95, 0.98], np.float32)
        glows = [
            (0.15, 0.20, 0.30, (0.55, 0.70, 1.00)),
            (0.95, 0.35, 0.35, (1.00, 0.72, 0.62)),
            (0.40, 0.80, 0.40, (0.62, 0.90, 0.85)),
            (0.80, 0.95, 0.25, (0.85, 0.70, 1.00)),
        ]
    else:
        image = np.ones((height, width, 3), np.float32) * np.array([0.03, 0.04, 0.09], np.float32)
        glows = [
            (0.10, 0.18, 0.32, (0.15, 0.35, 0.95)),
            (0.95, 0.40, 0.34, (0.85, 0.25, 0.55)),
            (0.35, 0.85, 0.38, (0.10, 0.65, 0.75)),
            (0.85, 0.92, 0.22, (0.55, 0.30, 0.95)),
            (0.55, 0.50, 0.18, (0.95, 0.55, 0.25)),
        ]
    aspect = height / width
    for cx, cy, radius, colour in glows:
        weight = glow(x, y * aspect, cx, cy * aspect, radius)[..., None]
        image = image * (1.0 - 0.85 * weight) + np.array(colour, np.float32) * 0.85 * weight
    # A fine grain against banding in the video's encoding.
    rng = np.random.default_rng(7)
    image += rng.normal(0.0, 0.006, image.shape).astype(np.float32)
    return Image.fromarray((np.clip(image, 0.0, 1.0) * 255.0 + 0.5).astype(np.uint8), "RGB")


if __name__ == "__main__":
    width, height, out = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    mood = sys.argv[4] if len(sys.argv) > 4 else "dark"
    wallpaper(width, height, mood).save(out)
