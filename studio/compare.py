"""Compares two folders of studio renders, frame by frame: how many pixels
differ in each, and by how much at most. A refactoring that changes nothing
on screen leaves every count at zero.

    python studio/compare.py BEFORE AFTER

Exits 1 if any frame differs or is missing from either side.
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image


def frames(folder):
    return {path.relative_to(folder): path for path in Path(folder).rglob("*.png")}


if __name__ == "__main__":
    before, after = Path(sys.argv[1]), Path(sys.argv[2])
    a, b = frames(before), frames(after)
    bad = 0
    for name in sorted(set(a) | set(b)):
        if name not in a or name not in b:
            print(f"{name}: only in {'before' if name in a else 'after'}")
            bad += 1
            continue
        x = np.asarray(Image.open(a[name]).convert("RGBA"), np.int16)
        y = np.asarray(Image.open(b[name]).convert("RGBA"), np.int16)
        if x.shape != y.shape:
            print(f"{name}: size {x.shape[1]}x{x.shape[0]} -> {y.shape[1]}x{y.shape[0]}")
            bad += 1
            continue
        delta = np.abs(x - y).max(axis=2)
        changed = int((delta > 0).sum())
        if changed:
            print(f"{name}: {changed} pixels differ, by up to {int(delta.max())}")
            bad += 1
    print(f"{len(set(a) | set(b))} frames, {bad} differ")
    sys.exit(1 if bad else 0)
