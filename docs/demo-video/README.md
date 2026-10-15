# The demo film

A narrated product demo of Plunger, built from real screen recordings of the app, Amazon Polly narration and
motion design in [Remotion](https://www.remotion.dev). Nothing here is part of Plunger's build.

Remotion is free for individuals and companies of up to three people; larger companies need a
[company licence](https://www.remotion.dev/license) to render it.

## Pieces

| File | What it does |
|---|---|
| `script.py` | The words, one entry per scene. Change them here. |
| `narrate.py` | Speaks each scene with Polly's generative voice and measures it (`audio/*.mp3`, `durations.json`). |
| `music.py` | A soft synthesised chord bed (`audio/music.wav`), so there is nothing to license. |
| `record.ps1`, `ui.ps1` | Drives the real Plunger window on Windows and records each scene at 30 fps (`clips/*.mp4`). |
| `echo_server.py` | Answers every request with what it received (the "Any body" scene). |
| `remotion/` | The film: scenes, callouts, the terminal, transitions. Scene lengths follow the narration. |
| `build.py`, `kit.py` | The first, Python-only version of the film (kept for reference). |

## Making it

Work folder: `C:/Temp/demo`. Needs Python with `boto3`, `botocore[crt]`, `imageio-ffmpeg`, `numpy`, `pillow`;
Node 18+; AWS credentials allowed to call `polly:SynthesizeSpeech`; a release build of Plunger.

```bash
python docs/demo-video/narrate.py C:/Temp/demo
python docs/demo-video/music.py C:/Temp/demo 300
```

The recordings need the fake tracker and the echo server running, and a fresh demo data folder:

```bash
TRACKER_DELAY=0.7 python scripts/tracker-api.py 18096
python docs/demo-video/echo_server.py 18111
```

Then, with the data folder `C:/Temp/demoapp` set up (variables `jira_base`, `jira_key`, `key`; a few saved
requests in groups; the "Jira: close issue" workflow), record each scene in this order, since each one
starts where the last left the app:

```powershell
foreach ($s in "window","bodies","save","variables","agent","workflow","history") { powershell -File docs/demo-video/record.ps1 -Scene $s }
```

And render:

```bash
cd docs/demo-video/remotion
npm install
node prepare.mjs C:/Temp/demo
npx remotion studio src/index.ts          # preview and scrub
npx remotion render src/index.ts Film out/plunger-demo-raw.mp4 --codec h264 --crf 17 --pixel-format yuv420p
ffmpeg -i out/plunger-demo-raw.mp4 -c:v copy -af loudnorm=I=-14:TP=-1.5:LRA=11 -c:a aac -b:a 192k out/plunger-demo.mp4
```

The last step brings the sound to YouTube's loudness (-14 LUFS).
