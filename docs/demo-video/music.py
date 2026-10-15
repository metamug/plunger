"""Writes <workdir>/audio/music.wav: a soft, slow chord bed (synthesised, so there is nothing to license).

    python docs/demo-video/music.py [workdir] [seconds]
"""
import sys
import wave

import numpy as np

work = sys.argv[1] if len(sys.argv) > 1 else "C:/Temp/demo"
seconds = float(sys.argv[2]) if len(sys.argv) > 2 else 300
rate = 44100
n = int(seconds * rate)
tt = np.arange(n) / rate
# A minor, F, C, G: warm and unhurried
chords = [(220.00, 261.63, 329.63), (174.61, 220.00, 261.63), (261.63, 329.63, 392.00), (196.00, 246.94, 293.66)]
bar = 6.0
left = np.zeros(n)
right = np.zeros(n)
for i in range(int(seconds / bar) + 2):
    a, b = int(i * bar * rate), min(n, int((i + 1) * bar * rate + 2 * rate))
    if a >= n:
        break
    local = tt[a:b] - i * bar
    env = np.clip(local / 2.2, 0, 1) * np.clip((bar + 2.0 - local) / 2.2, 0, 1)
    for k, f in enumerate(chords[i % 4]):
        for h, g in ((1, 1.0), (2, 0.30), (3, 0.10)):
            wave_l = np.sin(2 * np.pi * f * h * tt[a:b] * 1.0007 + h + k)
            wave_r = np.sin(2 * np.pi * f * h * tt[a:b] * 0.9993 + h + k * 2)
            left[a:b] += env * g * wave_l * 0.06
            right[a:b] += env * g * wave_r * 0.06
    bass = env * 0.06 * np.sin(2 * np.pi * chords[i % 4][0] / 2 * tt[a:b])
    left[a:b] += bass
    right[a:b] += bass
# a gentle pulse so it moves
pulse = 0.82 + 0.18 * np.sin(2 * np.pi * tt * 0.5) ** 2
fade = np.clip(tt / 3.0, 0, 1) * np.clip((seconds - tt) / 5.0, 0, 1)
stereo = np.stack([left, right], axis=1) * (pulse * fade)[:, None]
stereo = stereo / np.max(np.abs(stereo)) * 0.6
pcm = (stereo * 32767).astype(np.int16)
with wave.open(f"{work}/audio/music.wav", "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(rate)
    w.writeframes(pcm.tobytes())
print("music.wav", seconds, "s")
