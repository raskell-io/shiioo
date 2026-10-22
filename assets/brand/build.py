#!/usr/bin/env python3
"""Builds Shiioo's brand assets from the sources in ./source and ./fonts.

The mascot and icons are pixel art, written as SVG <rect> runs so they stay
crisp at any size. The wordmark and tagline are set in real typefaces
(Bricolage Grotesque and Inter, both SIL OFL, see ./fonts) and converted to outlines,
because GitHub does not load fonts inside SVG images.

Requires fontTools (`pip install fonttools`), ImageMagick (`magick`) for
reading source PNGs and writing ICO, and resvg for rendering the lockups to
PNG. Run from anywhere: `python3 assets/brand/build.py`.
"""

import subprocess
from functools import lru_cache
from pathlib import Path

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

HERE = Path(__file__).resolve().parent
SRC = HERE / "source"
FONTS = HERE / "fonts"

CHARCOAL = (43, 45, 51)  # the suit
TEAL = (47, 125, 115)  # the necktie
PAPER = (244, 241, 234)
INK_ON_DARK = (236, 233, 226)
TEAL_ON_DARK = (94, 180, 166)

WORDMARK = ("BricolageGrotesque.ttf", (("wght", 760), ("opsz", 96), ("wdth", 100)))
WORDMARK_TRACKING = -0.018  # em; negative tightens
TAGLINE = ("Inter.ttf", (("wght", 600), ("opsz", 14)))


class Canvas:
    def __init__(self, w, h):
        self.w, self.h = w, h
        self.px = [[None] * w for _ in range(h)]

    def rects(self, ox=0, oy=0):
        out = []
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
                out.append(
                    f'<rect x="{ox + start}" y="{oy + y}" width="{x - start}" height="1" fill="{hexrgb(c)}"/>'
                )
        return "\n".join(out)

    def svg(self):
        return svg_doc(self.w, self.h, f'<g shape-rendering="crispEdges">\n{self.rects()}\n</g>', scale=4)

    def rgba(self):
        buf = bytearray()
        for row in self.px:
            for c in row:
                buf += bytes(c) + b"\xff" if c else b"\0\0\0\0"
        return bytes(buf)


def hexrgb(c):
    return "#%02x%02x%02x" % c


def svg_doc(w, h, body, scale=1, background=None):
    bg = f'<rect width="{w}" height="{h}" fill="{hexrgb(background)}"/>\n' if background else ""
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" '
        f'width="{w * scale:g}" height="{h * scale:g}">\n{bg}{body}\n</svg>\n'
    )


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


def pixel_png(canvas, path, scale=1, downscale_to=None):
    cmd = ["magick", "-size", f"{canvas.w}x{canvas.h}", "-depth", "8", "rgba:-"]
    if downscale_to:
        cmd += ["-filter", "Box", "-resize", f"{downscale_to}x{downscale_to}"]
    else:
        cmd += ["-filter", "Point", "-resize", f"{scale * 100}%"]
    subprocess.run(cmd + [str(path)], input=canvas.rgba(), check=True)


@lru_cache(maxsize=None)
def instance(font):
    name, axes = font
    return instancer.instantiateVariableFont(TTFont(FONTS / name), dict(axes))


class Text:
    """A line of text converted to outlines, in font units scaled to `size`."""

    def __init__(self, font, text, size, tracking=0.0):
        tt = instance(font)
        glyphs, cmap = tt.getGlyphSet(), tt.getBestCmap()
        hmtx = tt["hmtx"]
        self.scale = size / tt["head"].unitsPerEm
        self.items = []  # (glyph name, x offset in font units)
        x = 0.0
        for i, ch in enumerate(text):
            name = cmap[ord(ch)]
            self.items.append((name, x))
            x += hmtx[name][0] + (tracking * tt["head"].unitsPerEm if i < len(text) - 1 else 0)
        self.glyphs = glyphs
        bounds = BoundsPen(glyphs)
        for name, gx in self.items:
            glyphs[name].draw(TransformPen(bounds, (1, 0, 0, 1, gx, 0)))
        xmin, ymin, xmax, ymax = bounds.bounds
        # Ink box in output units, y pointing down, origin at the baseline.
        self.left, self.right = xmin * self.scale, xmax * self.scale
        self.top, self.bottom = -ymax * self.scale, -ymin * self.scale
        self.width, self.height = self.right - self.left, self.bottom - self.top

    def path(self, x, y, rgb):
        """Path with the ink box's top-left corner at (x, y)."""
        pen = SVGPathPen(self.glyphs, ntos=lambda v: f"{v:.2f}".rstrip("0").rstrip("."))
        ox, oy = x - self.left, y - self.top
        for name, gx in self.items:
            t = (self.scale, 0, 0, -self.scale, ox + gx * self.scale, oy)
            self.glyphs[name].draw(TransformPen(pen, t))
        return f'<path fill="{hexrgb(rgb)}" d="{pen.getCommands()}"/>'


def tracked_to_width(font, text, size, width):
    """Finds the letter spacing that makes `text` exactly `width` wide."""
    lo, hi = 0.0, 1.0
    for _ in range(40):
        mid = (lo + hi) / 2
        if Text(font, text, size, mid).width < width:
            lo = mid
        else:
            hi = mid
    return Text(font, text, size, lo)


def lockup(mascot, ink, accent, pad=8):
    """Mascot (pixel art) beside the wordmark and tagline (outlined type).
    Returns the SVG body and its size, in mascot-pixel units."""
    word = Text(WORDMARK, "shiioo", size=mascot.h * 0.62, tracking=WORDMARK_TRACKING)
    tag = tracked_to_width(TAGLINE, "VIRTUAL COMPANY OS", size=word.height * 0.2, width=word.width)
    gap_x, gap_y = mascot.w * 0.14, word.height * 0.24
    text_h = word.height + gap_y + tag.height
    w = pad + mascot.w + gap_x + word.width + pad
    h = pad + max(mascot.h, text_h) + pad
    mx, my = pad, (h - mascot.h) / 2
    tx, ty = pad + mascot.w + gap_x, (h - text_h) / 2
    body = "\n".join([
        f'<g shape-rendering="crispEdges">\n{mascot.rects(mx, my)}\n</g>',
        word.path(tx, ty, ink),
        tag.path(tx, ty + word.height + gap_y, accent),
    ])
    return body, w, h


def render(svg_path, png_path, width):
    subprocess.run(["resvg", "--width", str(width), str(svg_path), str(png_path)], check=True)


def main():
    mascot = load(SRC / "mascot-128.png")

    icon = load(SRC / "icon-64.png", trim=False)
    (HERE / "shiioo-icon.svg").write_text(icon.svg())
    for size in (64, 256, 512):
        pixel_png(icon, HERE / f"shiioo-icon-{size}.png", scale=size // icon.w)
    # Small sizes come from dedicated 32px art; shrinking the 64px bust
    # turns the face to mush.
    favicon = load(SRC / "favicon-32.png", trim=False)
    (HERE / "shiioo-favicon.svg").write_text(favicon.svg())
    pixel_png(favicon, HERE / "shiioo-icon-32.png")
    pixel_png(favicon, HERE / "shiioo-icon-16.png", downscale_to=16)
    subprocess.run(
        ["magick", *(str(HERE / f"shiioo-icon-{s}.png") for s in (16, 32, 64)), str(HERE / "favicon.ico")],
        check=True,
    )

    for variant, ink, accent in (("light", CHARCOAL, TEAL), ("dark", INK_ON_DARK, TEAL_ON_DARK)):
        body, w, h = lockup(mascot, ink, accent)
        svg = HERE / f"shiioo-mark-{variant}.svg"
        svg.write_text(svg_doc(w, h, body, scale=4))
        render(svg, HERE / f"shiioo-mark-{variant}.png", width=round(w * 4))

    # GitHub's social preview: 1280x640, the lockup centred on paper.
    body, w, h = lockup(mascot, CHARCOAL, TEAL)
    sw, sh = w * 1.25, w * 0.625
    social = HERE / "shiioo-social.svg"
    social.write_text(
        svg_doc(sw, sh, f'<g transform="translate({(sw - w) / 2:.2f} {(sh - h) / 2:.2f})">\n{body}\n</g>',
                background=PAPER)
    )
    render(social, HERE / "shiioo-social.png", width=1280)
    social.unlink()


if __name__ == "__main__":
    main()
