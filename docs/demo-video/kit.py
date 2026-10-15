"""Drawing kit for the demo film: a 1920x1080 canvas, easing, text, rounded cards, a window frame with a shadow,
a terminal that types, and the plunger mark. Everything is PIL + numpy, so the film builds anywhere Python does.
"""
import math
import os
import subprocess

import imageio_ffmpeg
import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

W, H, FPS = 1920, 1080, 30
FFMPEG = imageio_ffmpeg.get_ffmpeg_exe()

# the palette is the app's own
BG = (14, 16, 21)
PANEL = (22, 24, 29)
CARD = (30, 33, 40)
BORDER = (52, 55, 63)
TEXT = (232, 232, 232)
WEAK = (150, 154, 164)
AMBER = (240, 150, 40)
AMBER_SOFT = (255, 205, 130)
GREEN = (90, 210, 140)
RED = (230, 100, 90)
BLUE = (110, 160, 240)
PURPLE = (180, 140, 235)

_FONTS = {}
FONT_DIR = "C:/Windows/Fonts/"


def font(size, weight="regular", mono=False):
    key = (size, weight, mono)
    if key not in _FONTS:
        if mono:
            name = "consolab.ttf" if weight == "bold" else "consola.ttf"
        else:
            name = {"regular": "segoeui.ttf", "semi": "seguisb.ttf", "bold": "segoeuib.ttf", "light": "segoeuil.ttf"}[weight]
        _FONTS[key] = ImageFont.truetype(FONT_DIR + name, size)
    return _FONTS[key]


# ---- easing and timing -----------------------------------------------------------------------------------
def clamp(x, a=0.0, b=1.0):
    return max(a, min(b, x))


def ease_out(t):
    t = clamp(t)
    return 1 - (1 - t) ** 3


def ease_in_out(t):
    t = clamp(t)
    return t * t * (3 - 2 * t)


def ease_back(t):
    t = clamp(t)
    c1, c3 = 1.4, 2.4
    return 1 + c3 * (t - 1) ** 3 + c1 * (t - 1) ** 2


def prog(t, start, dur):
    """0..1 progress of an animation that starts at `start` and lasts `dur` seconds."""
    return clamp((t - start) / dur) if dur > 0 else float(t >= start)


def lerp(a, b, t):
    return a + (b - a) * t


def mix(c1, c2, t):
    return tuple(int(lerp(a, b, t)) for a, b in zip(c1, c2))


# ---- the canvas -------------------------------------------------------------------------------------------
_GLOW = None


def background(t, accent=AMBER):
    """The film's backdrop: near-black, a slow amber glow, a fine grid. Cheap enough for every frame."""
    global _GLOW
    if _GLOW is None:
        glow = Image.new("RGB", (W, H), BG)
        d = ImageDraw.Draw(glow)
        for r in range(600, 0, -12):
            k = (1 - r / 600) ** 2 * 0.22
            d.ellipse((W * 0.78 - r, H * 0.1 - r, W * 0.78 + r, H * 0.1 + r), fill=mix(BG, accent, k))
        for r in range(520, 0, -12):
            k = (1 - r / 520) ** 2 * 0.10
            d.ellipse((W * 0.08 - r, H * 0.95 - r, W * 0.08 + r, H * 0.95 + r), fill=mix(BG, BLUE, k))
        _GLOW = glow.filter(ImageFilter.GaussianBlur(40))
    img = _GLOW.copy().convert("RGBA")
    return img


def text_size(draw, s, f):
    box = draw.textbbox((0, 0), s, font=f)
    return box[2] - box[0], box[3] - box[1]


def put_text(img, xy, s, f, fill=TEXT, alpha=1.0, anchor="la"):
    """Draws text with an opacity; img is RGBA."""
    if alpha <= 0.01:
        return
    layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    ImageDraw.Draw(layer).text(xy, s, font=f, fill=tuple(fill) + (int(255 * clamp(alpha)),), anchor=anchor)
    img.alpha_composite(layer)


def rounded_rect(img, box, radius, fill=None, outline=None, width=1, alpha=1.0):
    layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    f = tuple(fill) + (int(255 * clamp(alpha)),) if fill is not None else None
    o = tuple(outline) + (int(255 * clamp(alpha)),) if outline is not None else None
    d.rounded_rectangle(box, radius=radius, fill=f, outline=o, width=width)
    img.alpha_composite(layer)


def shadow(img, box, radius=18, blur=30, offset=(0, 18), alpha=0.55):
    pad = blur * 3
    x0, y0, x1, y1 = box
    layer = Image.new("RGBA", (int(x1 - x0 + 2 * pad), int(y1 - y0 + 2 * pad)), (0, 0, 0, 0))
    ImageDraw.Draw(layer).rounded_rectangle((pad, pad, pad + x1 - x0, pad + y1 - y0), radius=radius, fill=(0, 0, 0, int(255 * alpha)))
    layer = layer.filter(ImageFilter.GaussianBlur(blur))
    img.alpha_composite(layer, (int(x0 - pad + offset[0]), int(y0 - pad + offset[1])))


def rounded_mask(size, radius):
    m = Image.new("L", size, 0)
    ImageDraw.Draw(m).rounded_rectangle((0, 0, size[0] - 1, size[1] - 1), radius=radius, fill=255)
    return m


def paste_window(img, frame, center, scale, alpha=1.0, radius=14):
    """Puts a screen capture on the canvas as a floating window with a shadow, scaled about `center`."""
    w, h = int(frame.width * scale), int(frame.height * scale)
    x, y = int(center[0] - w / 2), int(center[1] - h / 2)
    shadow(img, (x, y, x + w, y + h), radius=radius, alpha=0.6 * alpha)
    scaled = frame.resize((w, h), Image.LANCZOS).convert("RGBA")
    mask = rounded_mask((w, h), radius)
    if alpha < 1:
        mask = mask.point(lambda v: int(v * alpha))
    img.paste(scaled, (x, y), mask)
    rounded_rect(img, (x, y, x + w, y + h), radius, outline=(255, 255, 255), width=1, alpha=0.10 * alpha)
    return (x, y, w, h)


# ---- callouts ---------------------------------------------------------------------------------------------
def callout(img, t, start, end, anchor, label, side="right", color=AMBER):
    """A pulsing ring at `anchor` and a label that slides in beside it, between `start` and `end` seconds."""
    if not (start <= t <= end):
        return
    a = ease_out(prog(t, start, 0.35)) * (1 - ease_in_out(prog(t, end - 0.35, 0.35)))
    ax, ay = anchor
    pulse = (t - start) * 1.6 % 1.0
    r = 16 + 22 * pulse
    layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    d.ellipse((ax - r, ay - r, ax + r, ay + r), outline=tuple(color) + (int(200 * (1 - pulse) * a),), width=3)
    d.ellipse((ax - 7, ay - 7, ax + 7, ay + 7), fill=tuple(color) + (int(255 * a),))
    img.alpha_composite(layer)
    f = font(30, "semi")
    tw, th = text_size(ImageDraw.Draw(img), label, f)
    pad_x, pad_y = 22, 14
    bw, bh = tw + 2 * pad_x, th + 2 * pad_y + 6
    slide = (1 - ease_out(prog(t, start, 0.45))) * 26
    if side == "right":
        bx = ax + 44 + slide
    else:
        bx = ax - 44 - bw - slide
    by = ay - bh / 2
    shadow(img, (bx, by, bx + bw, by + bh), radius=14, blur=18, offset=(0, 8), alpha=0.5 * a)
    rounded_rect(img, (bx, by, bx + bw, by + bh), 14, fill=(26, 28, 34), outline=color, width=2, alpha=a)
    put_text(img, (bx + pad_x, by + pad_y - 3), label, f, TEXT, a)


def chip(img, xy, label, alpha=1.0, color=AMBER):
    f = font(26, "semi")
    tw, th = text_size(ImageDraw.Draw(img), label, f)
    x, y = xy
    rounded_rect(img, (x, y, x + tw + 36, y + th + 24), 22, fill=mix(BG, color, 0.18), outline=color, width=2, alpha=alpha)
    put_text(img, (x + 18, y + 8), label, f, color, alpha)


# ---- a terminal that types --------------------------------------------------------------------------------
def terminal(img, box, t, script, title="plunger", font_size=24):
    """script: list of (time, kind, text) with kind 'cmd' (typed at ~38 chars/s) or 'out' (appears at once).

    Lines scroll when the window is full, like a real terminal."""
    x0, y0, x1, y1 = box
    shadow(img, box, radius=16, alpha=0.6)
    rounded_rect(img, box, 16, fill=(12, 14, 18), outline=BORDER, width=2)
    rounded_rect(img, (x0, y0, x1, y0 + 46), 16, fill=(28, 30, 37))
    ImageDraw.Draw(img).rectangle((x0 + 2, y0 + 30, x1 - 2, y0 + 46), fill=(28, 30, 37))
    for i, c in enumerate([(255, 95, 86), (255, 189, 46), (39, 201, 63)]):
        ImageDraw.Draw(img).ellipse((x0 + 20 + i * 26, y0 + 15, x0 + 36 + i * 26, y0 + 31), fill=c)
    put_text(img, ((x0 + x1) / 2, y0 + 23), title, font(20), WEAK, anchor="mm")
    f = font(font_size, mono=True)
    line_h = int(font_size * 1.42)
    lines = []  # (text, color, visible_chars)
    for (ts, kind, text) in script:
        if t < ts:
            break
        if kind == "cmd":
            n = int((t - ts) * 38)
            shown = text[: max(0, n)]
            lines.append(("$ " + shown, TEXT, True, len(shown) < len(text)))
        else:
            for ln in text.split("\n"):
                lines.append((ln, WEAK if kind == "out" else GREEN if kind == "ok" else RED, False, False))
    max_lines = int((y1 - y0 - 46 - 30) / line_h)
    lines = lines[-max_lines:]
    y = y0 + 46 + 18
    d = ImageDraw.Draw(img)
    for text, color, is_cmd, typing in lines:
        if is_cmd:
            d.text((x0 + 24, y), "$", font=f, fill=AMBER)
            d.text((x0 + 24 + d.textlength("$ ", font=f), y), text[2:], font=f, fill=color)
            if typing:
                cx = x0 + 24 + d.textlength(text, font=f) + 2
                d.rectangle((cx, y + 4, cx + 12, y + line_h - 8), fill=AMBER)
        else:
            d.text((x0 + 24, y), text, font=f, fill=color)
        y += line_h


# ---- the mark ---------------------------------------------------------------------------------------------
def plunger_mark(img, center, size, alpha=1.0, tilt=0.0):
    """The logo: a handle and a rubber cup, drawn with shapes so it can animate."""
    s = size / 100.0
    layer = Image.new("RGBA", (int(size * 2), int(size * 2)), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    cx, cy = size, size
    a = int(255 * clamp(alpha))
    # handle
    d.rounded_rectangle((cx - 6 * s, cy - 92 * s, cx + 6 * s, cy + 10 * s), radius=6 * s, fill=(200, 160, 110, a))
    # cup
    d.pieslice((cx - 62 * s, cy - 10 * s, cx + 62 * s, cy + 110 * s), 180, 360, fill=(200, 60, 52, a))
    d.rounded_rectangle((cx - 62 * s, cy + 46 * s, cx + 62 * s, cy + 58 * s), radius=6 * s, fill=(160, 40, 36, a))
    if tilt:
        layer = layer.rotate(tilt, resample=Image.BICUBIC, center=(cx, cy))
    img.alpha_composite(layer, (int(center[0] - size), int(center[1] - size)))


# ---- a decoded screen recording ---------------------------------------------------------------------------
class Clip:
    """A screen recording read once into memory, cropped to the app window, addressable by time."""

    def __init__(self, path, crop=(8, 0, 1172, 772), max_seconds=40):
        w, h = 1180, 780
        cmd = [FFMPEG, "-v", "error", "-i", path, "-t", str(max_seconds), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]
        raw = subprocess.run(cmd, capture_output=True).stdout
        n = len(raw) // (w * h * 3)
        arr = np.frombuffer(raw[: n * w * h * 3], dtype=np.uint8).reshape(n, h, w, 3)
        x0, y0, x1, y1 = crop
        self.frames = np.ascontiguousarray(arr[:, y0:y1, x0:x1])
        self.fps = 30.0
        self.duration = n / self.fps
        self.size = (x1 - x0, y1 - y0)

    def at(self, t):
        i = int(clamp(t * self.fps, 0, len(self.frames) - 1))
        return Image.fromarray(self.frames[i])


def zoom_frame(frame, center, scale):
    """Crops `frame` (PIL) to the view of a camera at `center` (fractions) with `scale`>=1, back to its size."""
    if scale <= 1.001:
        return frame
    w, h = frame.size
    cw, ch = w / scale, h / scale
    cx = clamp(center[0] * w, cw / 2, w - cw / 2)
    cy = clamp(center[1] * h, ch / 2, h - ch / 2)
    return frame.crop((int(cx - cw / 2), int(cy - ch / 2), int(cx + cw / 2), int(cy + ch / 2))).resize((w, h), Image.LANCZOS)
