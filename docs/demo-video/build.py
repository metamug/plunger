"""Renders the demo film from the narration (audio/*.mp3) and the screen recordings (clips/*.mp4).

    python docs/demo-video/build.py [workdir]                     full film -> <workdir>/plunger-demo.mp4
    python docs/demo-video/build.py [workdir] --scene 04_window --at 3,8,12     preview frames -> <workdir>/preview

Every scene lasts as long as its narration plus a lead-in and a tail, so changing the words re-times the film.
"""
import json
import os
import subprocess
import sys
import wave

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, os.path.dirname(__file__))
import kit  # noqa: E402
from kit import (AMBER, AMBER_SOFT, BLUE, BORDER, CARD, FFMPEG, FPS, GREEN, H, PURPLE, RED, TEXT, W, WEAK, Clip,  # noqa: E402
                 background, callout, chip, ease_back, ease_in_out, ease_out, font, mix, paste_window, plunger_mark,
                 prog, put_text, rounded_rect, shadow, terminal, text_size, zoom_frame)
from script import SCENES  # noqa: E402

args = [a for a in sys.argv[1:] if not a.startswith("--")]
WORK = args[0] if args else "C:/Temp/demo"
LEAD, TAIL = 0.7, 0.9
audio_dir = os.path.join(WORK, "audio")
clip_dir = os.path.join(WORK, "clips")
DUR = json.load(open(os.path.join(audio_dir, "durations.json")))
_clips = {}


def clip(name):
    if name not in _clips:
        _clips[name] = Clip(os.path.join(clip_dir, name + ".mp4"))
    return _clips[name]


def scene_len(sid):
    return LEAD + DUR[sid] + TAIL


def fade_in_out(t, total, head=0.35, tail=0.35):
    return ease_out(prog(t, 0, head)) * (1 - ease_in_out(prog(t, total - tail, tail)))


def title_chip(img, t, label, color=AMBER):
    a = ease_out(prog(t, 0.2, 0.5))
    chip(img, (64, 52 - (1 - a) * 18), label, a, color)


# ---- the scenes: each takes (t, total_seconds) and returns an RGBA frame ----------------------------------
def s01_hook(t, total):
    img = background(t)
    lines = [("Your AI agent just called an API.", 0.7, TEXT, 84),
             ("What did it send?", 3.4, AMBER_SOFT, 96),
             ("Where did your token go?", 5.2, TEXT, 84)]
    y = 330
    for s, start, color, size in lines:
        a = ease_out(prog(t, start, 0.7))
        f = font(size, "semi")
        w, _ = text_size(__import__("PIL.ImageDraw", fromlist=["x"]).Draw(img), s, f)
        put_text(img, (W / 2 - w / 2, y + (1 - a) * 34), s, f, color, a)
        y += 150
    return img


def s02_problem(t, total):
    img = background(t)
    title_chip(img, t, "The problem", RED)
    # a curl command, with the secret lit up
    box = (96, 190, 1000, 560)
    a = ease_out(prog(t, 0.5, 0.6))
    cmd = [("$ curl https://api.example.com/orders \\", TEXT),
           ('    -H "Authorization: Bearer sk_live_9f2a71c0b4e8"', TEXT),
           ("    -d '{\"user\": \"ann\", \"password\": \"hunter2\"}'", TEXT),
           ("", TEXT),
           ("# no record of this anywhere", WEAK)]
    shadow(img, box, alpha=0.6 * a)
    rounded_rect(img, box, 16, fill=(12, 14, 18), outline=BORDER, width=2, alpha=a)
    f = font(25, mono=True)
    d = __import__("PIL.ImageDraw", fromlist=["x"]).Draw(img)
    y = box[1] + 40
    flash = 0.5 + 0.5 * np.sin(max(0.0, t - 3.0) * 5.0) if t > 3.0 else 0
    for text, color in cmd:
        d.text((box[0] + 32, y), text, font=f, fill=color if a > 0.95 else mix(CARD, color, a))
        if "sk_live" in text and t > 3.0:
            x0 = box[0] + 32 + d.textlength('    -H "Authorization: Bearer ', font=f)
            x1 = x0 + d.textlength("sk_live_9f2a71c0b4e8", font=f)
            rounded_rect(img, (x0 - 4, y - 2, x1 + 4, y + 38), 6, outline=RED, width=3, alpha=0.5 + 0.5 * flash)
        y += 52
    cards = [("No record of what was sent", 4.4), ("Secrets end up in logs", 7.4), ("Undefined variables go out as text", 9.8)]
    y = 210
    for label, start in cards:
        a = ease_back(prog(t, 0.6 + start, 0.55))
        x = 1120 + (1 - a) * 160
        rounded_rect(img, (x, y, x + 700, y + 118), 18, fill=(28, 24, 26), outline=RED, width=2, alpha=min(1, a))
        d.ellipse((x + 28, y + 36, x + 74, y + 82), outline=RED + (255,), width=4)
        d.line((x + 40, y + 48, x + 62, y + 70), fill=RED, width=4)
        d.line((x + 62, y + 48, x + 40, y + 70), fill=RED, width=4)
        put_text(img, (x + 100, y + 30), label, font(34, "semi"), TEXT, min(1, a))
        y += 150
    last = ease_out(prog(t, 0.6 + 12.4, 0.6))
    s = "Nobody can say what was actually sent."
    w, _ = text_size(d, s, font(46, "semi"))
    put_text(img, (W / 2 - w / 2, 800 + (1 - last) * 24), s, font(46, "semi"), AMBER_SOFT, last)
    return img


def _node(img, center, label, sub, color, a, glyph):
    x, y = center
    rounded_rect(img, (x - 220, y - 100, x + 220, y + 100), 22, fill=(24, 26, 32), outline=color, width=3, alpha=a)
    d = __import__("PIL.ImageDraw", fromlist=["x"]).Draw(img)
    gx, gy = x - 160, y
    if glyph == "window":
        d.rounded_rectangle((gx - 28, gy - 22, gx + 28, gy + 22), radius=6, outline=color, width=4)
        d.line((gx - 28, gy - 8, gx + 28, gy - 8), fill=color, width=3)
    elif glyph == "cli":
        d.line((gx - 22, gy - 14, gx - 4, gy), fill=color, width=5)
        d.line((gx - 4, gy, gx - 22, gy + 14), fill=color, width=5)
        d.line((gx + 2, gy + 16, gx + 26, gy + 16), fill=color, width=5)
    else:
        d.ellipse((gx - 26, gy - 26, gx + 26, gy + 26), outline=color, width=4)
        d.ellipse((gx - 9, gy - 9, gx + 9, gy + 9), fill=color)
    put_text(img, (x - 100, y - 46), label, font(36, "semi"), TEXT, a)
    put_text(img, (x - 100, y + 6), sub, font(26), WEAK, a)


def s03_meet(t, total):
    img = background(t)
    # the mark and the name
    a = ease_back(prog(t, 0.6, 0.8))
    plunger_mark(img, (W / 2 - 330, 255), 88 * a + 1, alpha=ease_out(prog(t, 0.6, 0.4)), tilt=(1 - a) * 18)
    put_text(img, (W / 2 - 230, 150), "Plunger", font(130, "bold"), TEXT, ease_out(prog(t, 0.9, 0.7)))
    put_text(img, (W / 2 - 330, 340), "An HTTP client for you, and for your agent.", font(44), WEAK, ease_out(prog(t, 3.0, 0.7)))
    # three ways in, one shared core
    nodes = [((420, 640), "The window", "see and edit", BLUE, "window", 6.8),
             ((960, 640), "The command line", "plunger send …", GREEN, "cli", 8.2),
             ((1500, 640), "The MCP server", "for your agent", AMBER, "mcp", 9.6)]
    core_a = ease_out(prog(t, 12.0, 0.7))
    cx0, cy0, cx1, cy1 = 520, 880, 1400, 990
    for (c, label, sub, color, glyph, start) in nodes:
        na = ease_back(prog(t, start, 0.6))
        if na > 0:
            _node(img, (c[0], c[1] - (1 - min(1, na)) * 40), label, sub, color, min(1, na), glyph)
        if core_a > 0:
            layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
            dd = __import__("PIL.ImageDraw", fromlist=["x"]).Draw(layer)
            dd.line((c[0], c[1] + 100, c[0], cy0), fill=color + (int(180 * core_a),), width=4)
            img.alpha_composite(layer)
    if core_a > 0:
        rounded_rect(img, (cx0, cy0, cx1, cy1), 22, fill=(32, 28, 20), outline=AMBER, width=3, alpha=core_a)
        s = "Saved requests   ·   Variables   ·   History"
        w, _ = text_size(__import__("PIL.ImageDraw", fromlist=["x"]).Draw(img), s, font(38, "semi"))
        put_text(img, (W / 2 - w / 2, cy0 + 30), s, font(38, "semi"), AMBER_SOFT, core_a)
    return img


def ui_scene(name, clip_range, callouts, title, camera=None, extra=None):
    """A floating window playing a recording, with callouts and a slow camera."""

    def render(t, total):
        c = clip(name)
        img = background(t)
        title_chip(img, t, title)
        lo, hi = clip_range
        span = max(0.1, total - 2 * 0.6)
        speed = max(0.85, min(1.6, (hi - lo) / span))
        ct = lo + max(0.0, t - 0.6) * speed
        frame = c.at(min(ct, hi))
        cam = (0.5, 0.5, 1.0)
        if camera:
            # keyframes: (clip_time, cx, cy, scale); smooth between them
            kfs = sorted(camera)
            cam = kfs[0][1:]
            for (t0, *c0), (t1, *c1) in zip(kfs, kfs[1:]):
                if t0 <= ct <= t1:
                    k = ease_in_out((ct - t0) / (t1 - t0))
                    cam = tuple(kit.lerp(a, b, k) for a, b in zip(c0, c1))
                elif ct > t1:
                    cam = tuple(c1)
        frame = zoom_frame(frame, (cam[0], cam[1]), cam[2])
        appear = ease_out(prog(t, 0.15, 0.6))
        scale = (1.02 + 0.06 * appear) * (0.96 + 0.04 * appear)
        center = (W / 2, H / 2 + 48)
        x, y, w, h = paste_window(img, frame, center, scale, alpha=appear)
        # callouts are given in clip coordinates (1164x772); they follow the zoom
        for (s0, s1, (px, py), label, side) in callouts:
            if cam[2] > 1.001:
                cw, ch = frame.width / cam[2], frame.height / cam[2]
                cx = kit.clamp(cam[0] * frame.width, cw / 2, frame.width - cw / 2)
                cy = kit.clamp(cam[1] * frame.height, ch / 2, frame.height - ch / 2)
                vx = (px - (cx - cw / 2)) / cw
                vy = (py - (cy - ch / 2)) / ch
            else:
                vx, vy = px / frame.width, py / frame.height
            if 0 <= vx <= 1 and 0 <= vy <= 1:
                callout(img, t, 0.6 + (s0 - lo) / speed, 0.6 + (s1 - lo) / speed, (x + vx * w, y + vy * h), label, side)
        if extra:
            extra(img, t, total)
        return img

    return render


def s04_window():
    return ui_scene(
        "window", (1.0, 21.0),
        [(8.0, 10.5, (700, 240), "Query params stay in sync with the URL", "right"),
         (13.5, 16.5, (600, 430), "Fold it, search it, copy a path", "left"),
         (19.0, 21.0, (140, 246), "Saved requests live in groups", "right")],
        "The window")


def s05_variables():
    return ui_scene(
        "variables", (1.0, 15.0),
        [(2.4, 5.5, (300, 200), "Variables are shared by every request, the CLI and agents", "right"),
         (9.0, 11.0, (330, 181), "The Uses line shows what a request needs", "right"),
         (11.0, 14.0, (500, 281), "Undefined? Refused, and named", "right")],
        "Variables and secrets")


def s07_workflow():
    return ui_scene(
        "workflow", (0.5, 13.5),
        [(5.0, 7.0, (80, 307), "Step 1 asks the tracker what 'Done' is called", "right"),
         (7.0, 9.5, (180, 395), "…and hands its id to step 2", "right"),
         (9.5, 12.5, (520, 442), "Step 2 moves the issue. All steps passed", "right")],
        "Workflows")


# the terminal's script for the agent scene (real commands, real output, trimmed to what matters)
AGENT_TERM = [
    (1.2, "cmd", 'plunger send "Jira: get issue" \\n  --select $.fields.summary \\n  --select $.fields.status.name'),
    (4.4, "out", '{\n  "ok": true,\n  "status": 200,\n  "elapsed_ms": 704,\n  "selected": {\n    "$.fields.status.name": "Done",\n'
                 '    "$.fields.summary": "Checkout fails on retry"\n  }\n}'),
    (9.4, "cmd", 'plunger send "Jira: search" --select $.total'),
    (12.0, "out", '{\n  "ok": true,\n  "status": 200,\n  "selected": { "$.total": 0 }\n}'),
    (16.4, "cmd", "plunger send --url {{jira_base}}/issue/{{key}}/transitions \\\n  -H \"Authorization: Bearer {{jira_key}}\""),
    (19.4, "out", '{\n  "status": 200,\n  "request": { "url": "{{jira_base}}/issue/{{key}}/transitions" }\n}   # the token was never printed'),
]


def s06_agent(t, total):
    c = clip("agent")
    img = background(t)
    title_chip(img, t, "The agent")
    ct = min(max(0.0, t - 0.6) * 0.95 + 0.6, c.duration - 0.05)
    frame = c.at(ct)
    appear = ease_out(prog(t, 0.15, 0.6))
    paste_window(img, frame, (1420, 590), 0.74 + 0.02 * appear, alpha=appear)
    terminal(img, (50, 150, 930, 1010), max(0.0, t - 0.3), AGENT_TERM, title="your agent's terminal", font_size=24)
    callout(img, t, 5.0, 8.5, (1060, 760), "Every call lands here, tagged CLI", "right")
    callout(img, t, 20.0, 24.0, (1020, 950), "Only the values it asked for come back", "right", color=GREEN)
    return img


SAFE_TERM = [
    (1.0, "cmd", "plunger send --url http://localhost:3000/login -X POST \\\n  --json '{\"user\":\"ann\",\"password\":\"hunter2\"}'"),
    (3.6, "out", '{ "ok": true, "status": 200, "request": { "history_id": 31 } }'),
    (5.6, "cmd", "findstr hunter2 history.sqlite3 audit.log"),
    (7.6, "ok", "(no matches: the password was never written down)"),
    (9.2, "cmd", "plunger audit verify"),
    (10.8, "ok", '{ "ok": true, "entries": 44, "head": "e92dae55…" }'),
]


def s08_safe(t, total):
    img = background(t)
    title_chip(img, t, "Safe by default", GREEN)
    terminal(img, (96, 170, 1260, 760), max(0.0, t - 0.3), SAFE_TERM, title="plunger", font_size=27)
    items = [("Headers", 3.2), ("Query strings", 4.6), ("Request bodies", 6.0), ("Hash-chained log", 8.4)]
    y = 190
    for label, start in items:
        a = ease_back(prog(t, start, 0.5))
        x = 1340 + (1 - min(1, a)) * 80
        rounded_rect(img, (x, y, x + 480, y + 108), 20, fill=(20, 30, 26), outline=GREEN, width=2, alpha=min(1, a))
        layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
        ImageDraw.Draw(layer).line([(x + 32, y + 56), (x + 50, y + 74), (x + 80, y + 34)], fill=GREEN + (int(255 * min(1, a)),), width=8, joint="curve")
        img.alpha_composite(layer)
        put_text(img, (x + 90, y + 28), label, font(38, "semi"), TEXT, min(1, a))
        y += 138
    s = "Masked before anything touches the disk."
    a = ease_out(prog(t, 9.6, 0.6))
    w, _ = text_size(__import__("PIL.ImageDraw", fromlist=["x"]).Draw(img), s, font(48, "semi"))
    put_text(img, (W / 2 - w / 2, 860 + (1 - a) * 20), s, font(48, "semi"), AMBER_SOFT, a)
    return img


def s09_outro(t, total):
    img = background(t)
    a = ease_back(prog(t, 0.5, 0.8))
    plunger_mark(img, (W / 2, 200), 56 * a + 1, alpha=ease_out(prog(t, 0.5, 0.4)))
    d = __import__("PIL.ImageDraw", fromlist=["x"]).Draw(img)
    s = "Plunger"
    w, _ = text_size(d, s, font(110, "bold"))
    put_text(img, (W / 2 - w / 2, 290), s, font(110, "bold"), TEXT, ease_out(prog(t, 0.8, 0.6)))
    lines = [("pip install plunger-cli", 3.0), ("brew install --cask metamug/tap/plunger", 4.4), ("winget install 9P7WRKLN6WHR", 5.8)]
    y = 480
    for s, start in lines:
        a = ease_out(prog(t, start, 0.5))
        f = font(40, mono=True)
        w = d.textlength(s, font=f) + 80
        rounded_rect(img, (W / 2 - w / 2, y, W / 2 + w / 2, y + 76), 16, fill=(12, 14, 18), outline=BORDER, width=2, alpha=a)
        put_text(img, (W / 2 - w / 2 + 40, y + 16), s, f, GREEN, a)
        y += 100
    a = ease_out(prog(t, 9.0, 0.8))
    s = "Unclog your API."
    w, _ = text_size(d, s, font(76, "semi"))
    put_text(img, (W / 2 - w / 2, 800 + (1 - a) * 24), s, font(76, "semi"), AMBER_SOFT, a)
    s = "github.com/metamug/plunger   ·   MIT licensed"
    w, _ = text_size(d, s, font(32))
    put_text(img, (W / 2 - w / 2, 930), s, font(32), WEAK, ease_out(prog(t, 10.2, 0.6)))
    return img


RENDER = {
    "01_hook": s01_hook, "02_problem": s02_problem, "03_meet": s03_meet,
    "04_window": s04_window(), "05_variables": s05_variables(),
    "06_agent": s06_agent, "07_workflow": s07_workflow(), "08_safe": s08_safe, "09_outro": s09_outro,
}


# ---- output -----------------------------------------------------------------------------------------------
def build_audio(path):
    """The narration placed on a timeline: lead-in silence, the speech, the tail, scene after scene."""
    rate = 24000
    chunks = []
    for s in SCENES:
        mp3 = os.path.join(audio_dir, s["id"] + ".mp3")
        raw = subprocess.run([FFMPEG, "-v", "error", "-i", mp3, "-f", "s16le", "-ac", "1", "-ar", str(rate), "-"], capture_output=True).stdout
        speech = np.frombuffer(raw, dtype=np.int16)
        chunks.append(np.zeros(int(LEAD * rate), dtype=np.int16))
        chunks.append(speech)
        chunks.append(np.zeros(int(TAIL * rate), dtype=np.int16))
    pcm = np.concatenate(chunks)
    pcm = mix_music(pcm, rate)
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(pcm.tobytes())


def mix_music(speech, rate):
    """A soft, slow chord bed under the voice (synthesised, so there is nothing to license)."""
    n = len(speech)
    tt = np.arange(n) / rate
    chords = [(220.00, 261.63, 329.63), (174.61, 220.00, 261.63), (261.63, 329.63, 392.00), (196.00, 246.94, 293.66)]
    bar = 6.0
    bed = np.zeros(n)
    for i in range(int(n / rate / bar) + 2):
        a, b = int(i * bar * rate), min(n, int((i + 1) * bar * rate + 2 * rate))
        if a >= n:
            break
        local = tt[a:b] - i * bar
        env = np.clip(local / 2.0, 0, 1) * np.clip((bar + 2.0 - local) / 2.0, 0, 1)
        for f in chords[i % 4]:
            for h, g in ((1, 1.0), (2, 0.35), (3, 0.12)):
                bed[a:b] += env * g * np.sin(2 * np.pi * f * h * tt[a:b] + h) * 0.07
        bed[a:b] += env * 0.05 * np.sin(2 * np.pi * chords[i % 4][0] / 2 * tt[a:b])
    slow = 0.85 + 0.15 * np.sin(2 * np.pi * tt / 9.0)
    total = n / rate
    fade = np.clip(tt / 3.0, 0, 1) * np.clip((total - tt) / 4.0, 0, 1)
    bed = bed * slow * fade * 0.55
    out = speech.astype(np.float64) + bed * 32767 * 0.22
    return np.clip(out, -32768, 32767).astype(np.int16)


def render_scene_frames(sid):
    total = scene_len(sid)
    n = int(round(total * FPS))
    for i in range(n):
        t = i / FPS
        img = RENDER[sid](t, total)
        # a short fade from and to black between scenes
        edge = min(prog(t, 0, 0.25), 1 - prog(t, total - 0.25, 0.25))
        if edge < 1:
            black = Image.new("RGBA", img.size, (0, 0, 0, 255))
            img = Image.blend(black, img, edge)
        yield img.convert("RGB")


def main():
    if "--scene" in sys.argv:
        sid = sys.argv[sys.argv.index("--scene") + 1]
        times = [float(x) for x in sys.argv[sys.argv.index("--at") + 1].split(",")]
        out = os.path.join(WORK, "preview")
        os.makedirs(out, exist_ok=True)
        total = scene_len(sid)
        for t in times:
            RENDER[sid](t, total).convert("RGB").save(os.path.join(out, f"{sid}_{t:05.1f}.png"))
        print("preview", out)
        return
    wav = os.path.join(WORK, "narration.wav")
    build_audio(wav)
    target = os.path.join(WORK, "plunger-demo.mp4")
    cmd = [FFMPEG, "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}", "-r", str(FPS), "-i", "-",
           "-i", wav, "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p",
           "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", "-shortest", target]
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE)
    only = [a for a in sys.argv if a.startswith("--only=")]
    for s in SCENES:
        print("rendering", s["id"], flush=True)
        for frame in render_scene_frames(s["id"]):
            proc.stdin.write(np.asarray(frame).tobytes())
    proc.stdin.close()
    proc.wait()
    print("wrote", target)


if __name__ == "__main__":
    main()
