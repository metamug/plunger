"""Speaks every scene of script.py with Amazon Polly and measures it.

    python docs/demo-video/narrate.py [workdir]      (default C:/Temp/demo)

Writes <workdir>/audio/<id>.mp3 and <workdir>/audio/durations.json. Needs AWS credentials with
polly:SynthesizeSpeech (the usual environment or profile).
"""
import json
import os
import re
import subprocess
import sys

import boto3
import imageio_ffmpeg

sys.path.insert(0, os.path.dirname(__file__))
from script import ENGINE, REGION, SCENES, VOICE  # noqa: E402

work = sys.argv[1] if len(sys.argv) > 1 else "C:/Temp/demo"
audio = os.path.join(work, "audio")
os.makedirs(audio, exist_ok=True)
polly = boto3.client("polly", region_name=REGION)
ffmpeg = imageio_ffmpeg.get_ffmpeg_exe()


def ssml(text: str) -> str:
    # "A I", "U R L": letters read one at a time. Written spaced out in the script; say-as makes it deliberate.
    text = re.sub(r"\b((?:[A-Z] ){1,4}[A-Z])\b", lambda m: f"<say-as interpret-as='characters'>{m.group(1).replace(' ', '')}</say-as>", text)
    return f"<speak>{text}</speak>"


def duration(path: str) -> float:
    out = subprocess.run([ffmpeg, "-i", path, "-f", "null", "-"], capture_output=True, text=True).stderr
    m = re.findall(r"time=(\d+):(\d+):([\d.]+)", out)[-1]
    return int(m[0]) * 3600 + int(m[1]) * 60 + float(m[2])


durations = {}
for scene in SCENES:
    target = os.path.join(audio, scene["id"] + ".mp3")
    response = polly.synthesize_speech(
        Text=ssml(scene["say"]), TextType="ssml", VoiceId=VOICE, Engine=ENGINE, OutputFormat="mp3", SampleRate="24000"
    )
    with open(target, "wb") as f:
        f.write(response["AudioStream"].read())
    durations[scene["id"]] = round(duration(target), 2)
    print(f'{scene["id"]}: {durations[scene["id"]]}s')

with open(os.path.join(audio, "durations.json"), "w") as f:
    json.dump(durations, f, indent=1)
print("total", round(sum(durations.values()), 1), "s")
