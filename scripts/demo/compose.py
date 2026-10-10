"""Turns the screen recording into a captioned video and GIF.

    compose.py OUT_DIR

Reads clip.mov, markers.txt, start.txt and geometry.txt (written by record-macos.sh) from OUT_DIR, crops to
the Plunger window, draws a caption for each scene and an end card, and writes plunger-demo.mp4 and
plunger-demo.gif next to them. Needs imageio, imageio-ffmpeg, numpy and pillow.
"""
import os
import sys

import imageio.v2 as imageio
import numpy as np
from PIL import Image, ImageDraw, ImageFont

out_dir = sys.argv[1]
FPS = 15
MARGIN = 14  # points of desktop around the window

wx, wy, ww, wh, sw, sh = [int(v) for v in open(os.path.join(out_dir, "geometry.txt")).read().strip().split(",")]
start = float(open(os.path.join(out_dir, "start.txt")).read().strip())
markers = []
for line in open(os.path.join(out_dir, "markers.txt"), encoding="utf-8").read().split("\n"):
    if "|" in line:
        t, name, caption = line.split("|", 2)
        markers.append((float(t) - start, name, caption))
markers.sort()
end_time = next((t for t, name, _ in markers if name == "end"), None)


def caption_at(t):
    text = ""
    for when, _name, caption in markers:
        if t >= when:
            text = caption
    return text


def font(size, bold=False):
    candidates = [
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf" if bold else "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
        "C:/Windows/Fonts/seguisb.ttf" if bold else "C:/Windows/Fonts/segoeui.ttf",
    ]
    for path in candidates:
        try:
            return ImageFont.truetype(path, size)
        except OSError:
            continue
    return ImageFont.load_default()


reader = imageio.get_reader(os.path.join(out_dir, "clip.mov"), "ffmpeg")
meta = reader.get_meta_data()
src_fps = float(meta.get("fps") or 30)
first = reader.get_data(0)
scale = first.shape[1] / float(sw)  # pixels per point (2 on a Retina display)
left = max(0, int((wx - MARGIN) * scale))
top = max(0, int((wy - MARGIN) * scale))
right = min(first.shape[1], int((wx + ww + MARGIN) * scale))
bottom = min(first.shape[0], int((wy + wh + MARGIN) * scale))
# the output is the window at a fixed width, so it reads well on a phone
OUT_W = 1000
crop_w, crop_h = right - left, bottom - top
OUT_H = int(round(OUT_W * crop_h / crop_w / 2)) * 2
BAR = 64
print(f"source {first.shape[1]}x{first.shape[0]} at {src_fps} fps, scale {scale}, crop {crop_w}x{crop_h} -> {OUT_W}x{OUT_H}")

bold = font(26, True)


def with_caption(frame_rgb, text):
    im = Image.fromarray(frame_rgb).crop((left, top, right, bottom)).resize((OUT_W, OUT_H), Image.LANCZOS).convert("RGBA")
    if text:
        overlay = Image.new("RGBA", im.size, (0, 0, 0, 0))
        d = ImageDraw.Draw(overlay)
        d.rectangle([0, OUT_H - BAR, OUT_W, OUT_H], fill=(12, 13, 19, 232))
        d.rectangle([0, OUT_H - BAR, 6, OUT_H], fill=(99, 120, 230, 255))
        width = d.textlength(text, font=bold)
        d.text(((OUT_W - width) / 2, OUT_H - BAR + 16), text, font=bold, fill=(244, 245, 250, 255))
        im = Image.alpha_composite(im, overlay)
    return im.convert("RGB")


def end_card():
    card = Image.new("RGB", (OUT_W, OUT_H), (17, 18, 24))
    d = ImageDraw.Draw(card)
    title, sub, mono = font(58, True), font(28), font(30)

    def centered(y, text, f, fill):
        d.text(((OUT_W - d.textlength(text, font=f)) / 2, y), text, font=f, fill=fill)

    centered(OUT_H * 0.22, "Plunger", title, (244, 245, 250))
    centered(OUT_H * 0.22 + 85, "An API client for you, and an MCP server for your AI agent", sub, (176, 180, 200))
    centered(OUT_H * 0.22 + 130, "Secrets stay hidden. Responses stay small. Free and open source.", sub, (176, 180, 200))
    d.rounded_rectangle([OUT_W / 2 - 250, OUT_H * 0.22 + 215, OUT_W / 2 + 250, OUT_H * 0.22 + 275], radius=10, fill=(30, 32, 44), outline=(70, 76, 110))
    centered(OUT_H * 0.22 + 228, "pip install plunger-cli", mono, (160, 220, 170))
    centered(OUT_H * 0.22 + 320, "github.com/metamug/plunger", sub, (140, 160, 255))
    return card


frames = []
step = max(1, int(round(src_fps / FPS)))
count = 0
for index, raw in enumerate(reader):
    t = index / src_fps
    if end_time is not None and t > end_time + 0.8:
        break
    if t < markers[0][0] - 0.2:
        continue
    if index % step:
        continue
    frames.append(with_caption(raw, caption_at(t)))
    count += 1
print("frames kept:", count)

card = end_card()
hold = [card] * int(FPS * 3.2)
video = frames + hold

mp4 = os.path.join(out_dir, "plunger-demo.mp4")
writer = imageio.get_writer(mp4, fps=FPS, codec="libx264", quality=8, pixelformat="yuv420p", macro_block_size=2)
for im in video:
    writer.append_data(np.asarray(im))
writer.close()

# GIF: fewer frames, one shared palette, identical neighbours merged
gif_frames, gif_times = [], []
for i, im in enumerate(video):
    if i % 2:
        continue
    small = im.resize((820, int(round(820 * OUT_H / OUT_W))), Image.LANCZOS)
    if gif_frames and np.array_equal(np.asarray(small), np.asarray(gif_frames[-1])):
        gif_times[-1] += int(2000 / FPS)
    else:
        gif_frames.append(small)
        gif_times.append(int(2000 / FPS))
sample = Image.new("RGB", (gif_frames[0].width, gif_frames[0].height * 3))
for k, idx in enumerate([len(gif_frames) // 4, len(gif_frames) // 2, len(gif_frames) - 1]):
    sample.paste(gif_frames[idx], (0, k * gif_frames[0].height))
palette = sample.quantize(colors=160, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
quant = [f.quantize(palette=palette, dither=Image.Dither.NONE) for f in gif_frames]
gif = os.path.join(out_dir, "plunger-demo.gif")
quant[0].save(gif, save_all=True, append_images=quant[1:], duration=gif_times, loop=0, optimize=True, disposal=1)
print("mp4 MB:", round(os.path.getsize(mp4) / 1e6, 2), "gif MB:", round(os.path.getsize(gif) / 1e6, 2), "seconds:", round(len(video) / FPS, 1))
