#!/usr/bin/env python3
"""Builds Shiioo's brand assets from the pixel-art sources in ./source.

Everything is drawn on a pixel grid and written as SVG <rect> runs, so the
marks stay crisp at any size and need no fonts (GitHub does not load web
fonts inside SVG images). PNGs are rendered from the same grid.

Requires ImageMagick (`magick`) for reading the source PNGs and writing PNG
and ICO files. Run from anywhere: `python3 assets/brand/build.py`.
"""

import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
SRC = HERE / "source"

CHARCOAL = (43, 45, 51)  # the suit
TEAL = (47, 125, 115)  # the necktie
PAPER = (244, 241, 234)
INK_ON_DARK = (236, 233, 226)

# 5x7 pixel glyphs. Lowercase for the wordmark, capitals for the tagline.
GLYPHS = {
    "s": [".....", ".....", ".####", "#....", ".###.", "....#", "####."],
    "h": ["#....", "#....", "#.##.", "##..#", "#...#", "#...#", "#...#"],
    "i": ["#", ".", "#", "#", "#", "#", "#"],
    "o": [".....", ".....", ".###.", "#...#", "#...#", "#...#", ".###."],
    "A": [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
    "C": [".###.", "#...#", "#....", "#....", "#....", "#...#", ".###."],
    "I": ["###", ".#.", ".#.", ".#.", ".#.", ".#.", "###"],
    "L": ["#....", "#....", "#....", "#....", "#....", "#....", "#####"],
    "M": ["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#"],
    "N": ["#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#"],
    "O": [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
    "P": ["####.", "#...#", "#...#", "####.", "#....", "#....", "#...."],
    "R": ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"],
    "S": [".###.", "#...#", "#....", ".###.", "....#", "#...#", ".###."],
    "T": ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."],
    "U": ["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
    "V": ["#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#.."],
    "Y": ["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."],
}


class Canvas:
    def __init__(self, w, h):
        self.w, self.h = w, h
        self.px = [[None] * w for _ in range(h)]

    def set(self, x, y, rgb):
        if 0 <= x < self.w and 0 <= y < self.h:
            self.px[y][x] = rgb

    def blit(self, sprite, ox, oy):
        for y, row in enumerate(sprite.px):
            for x, c in enumerate(row):
                if c is not None:
                    self.set(ox + x, oy + y, c)

    def text(self, s, ox, oy, scale, rgb, shadow=None, offset=2):
        if shadow:
            self._text(s, ox + offset, oy + offset, scale, shadow)
        self._text(s, ox, oy, scale, rgb)

    def _text(self, s, ox, oy, scale, rgb):
        x = ox
        for ch in s:
            if ch == " ":
                x += 3 * scale
                continue
            g = GLYPHS[ch]
            for gy, line in enumerate(g):
                for gx, cell in enumerate(line):
                    if cell == "#":
                        for dy in range(scale):
                            for dx in range(scale):
                                self.set(x + gx * scale + dx, oy + gy * scale + dy, rgb)
            x += (len(g[0]) + 1) * scale

    def svg(self, background=None):
        out = [
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.w} {self.h}" '
            f'width="{self.w * 4}" height="{self.h * 4}" shape-rendering="crispEdges">'
        ]
        if background:
            out.append(f'<rect width="{self.w}" height="{self.h}" fill="{hexrgb(background)}"/>')
        for y, row in enumerate(self.px):
            x = 0
            while x < self.w:
                c = row[x]
                if c is None:
                    x += 1
                    continue
                start = x
                while x < self.w and row[x] == c:
                    x += 1
                out.append(f'<rect x="{start}" y="{y}" width="{x - start}" height="1" fill="{hexrgb(c)}"/>')
        out.append("</svg>\n")
        return "\n".join(out)

    def rgba(self, background=None):
        buf = bytearray()
        for row in self.px:
            for c in row:
                if c is None:
                    buf += bytes(background) + b"\xff" if background else b"\0\0\0\0"
                else:
                    buf += bytes(c) + b"\xff"
        return bytes(buf)


def hexrgb(c):
    return "#%02x%02x%02x" % c


def text_width(s, scale):
    w = 0
    for ch in s:
        w += 3 if ch == " " else len(GLYPHS[ch][0]) + 1
    return (w - 1) * scale


def load(path, trim=True):
    """Reads a PNG into a Canvas, dropping mostly-transparent pixels and,
    unless told otherwise, trimming empty margins."""
    size = subprocess.run(
        ["magick", "identify", "-format", "%w %h", str(path)],
        check=True, capture_output=True, text=True,
    ).stdout.split()
    w, h = int(size[0]), int(size[1])
    raw = subprocess.run(
        ["magick", str(path), "-depth", "8", "rgba:-"], check=True, capture_output=True
    ).stdout
    rows = []
    for y in range(h):
        row = []
        for x in range(w):
            r, g, b, a = raw[(y * w + x) * 4:(y * w + x) * 4 + 4]
            row.append((r, g, b) if a >= 128 else None)
        rows.append(row)
    ys = [y for y, row in enumerate(rows) if any(row)] if trim else [0, h - 1]
    xs = [x for x in range(w) if any(rows[y][x] for y in range(h))] if trim else [0, w - 1]
    c = Canvas(xs[-1] - xs[0] + 1, ys[-1] - ys[0] + 1)
    for y in range(c.h):
        for x in range(c.w):
            c.px[y][x] = rows[ys[0] + y][xs[0] + x]
    return c


def png(canvas, path, scale, background=None, downscale_to=None):
    cmd = ["magick", "-size", f"{canvas.w}x{canvas.h}", "-depth", "8", "rgba:-"]
    if downscale_to:
        cmd += ["-filter", "Box", "-resize", f"{downscale_to}x{downscale_to}"]
    else:
        cmd += ["-filter", "Point", "-resize", f"{scale * 100}%"]
    subprocess.run(cmd + [str(path)], input=canvas.rgba(background), check=True)


def square(sprite, pad):
    side = max(sprite.w, sprite.h) + 2 * pad
    c = Canvas(side, side)
    c.blit(sprite, (side - sprite.w) // 2, (side - sprite.h) // 2)
    return c


def lockup(mascot, ink, accent):
    word, tag = "shiioo", "VIRTUAL COMPANY OS"
    word_scale, tag_scale, gap, pad = 7, 2, 10, 8
    text_w = max(text_width(word, word_scale), text_width(tag, tag_scale))
    text_h = 7 * word_scale + gap + 7 * tag_scale
    w = pad + mascot.w + 14 + text_w + word_scale + pad
    h = pad + max(mascot.h, text_h) + pad
    c = Canvas(w, h)
    c.blit(mascot, pad, (h - mascot.h) // 2)
    tx = pad + mascot.w + 14
    ty = (h - text_h) // 2
    c.text(word, tx, ty, word_scale, ink, shadow=accent)
    c.text(tag, tx + 1, ty + 7 * word_scale + gap, tag_scale, accent)
    return c


def main():
    mascot = load(SRC / "mascot-128.png")
    icon = load(SRC / "icon-64.png", trim=False)

    (HERE / "shiioo-icon.svg").write_text(icon.svg())
    for size in (256, 512):
        png(icon, HERE / f"shiioo-icon-{size}.png", scale=size // icon.w)
    png(icon, HERE / "shiioo-icon-64.png", scale=1)
    # Small sizes come from dedicated 32px art; shrinking the 64px bust
    # turns the face to mush.
    favicon = load(SRC / "favicon-32.png", trim=False)
    (HERE / "shiioo-favicon.svg").write_text(favicon.svg())
    png(favicon, HERE / "shiioo-icon-32.png", scale=1)
    png(favicon, HERE / "shiioo-icon-16.png", scale=1, downscale_to=16)
    subprocess.run(
        ["magick", *(str(HERE / f"shiioo-icon-{s}.png") for s in (16, 32, 64)), str(HERE / "favicon.ico")],
        check=True,
    )

    for variant, ink in (("light", CHARCOAL), ("dark", INK_ON_DARK)):
        mark = lockup(mascot, ink, TEAL)
        (HERE / f"shiioo-mark-{variant}.svg").write_text(mark.svg())
        png(mark, HERE / f"shiioo-mark-{variant}.png", scale=4)

    social = Canvas(320, 160)  # 1280x640 at 4x, GitHub's social preview size
    mark = lockup(mascot, CHARCOAL, TEAL)
    social.blit(mark, (social.w - mark.w) // 2, (social.h - mark.h) // 2)
    png(social, HERE / "shiioo-social.png", scale=4, background=PAPER)


if __name__ == "__main__":
    main()
