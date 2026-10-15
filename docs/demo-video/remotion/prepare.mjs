// Copies the narration and the screen recordings into public/media, and the narration lengths into src/.
// Run after docs/demo-video/narrate.py and record.ps1:   node prepare.mjs [workdir]   (default C:/Temp/demo)
import fs from "node:fs";
import path from "node:path";

const work = process.argv[2] ?? "C:/Temp/demo";
const media = path.join("public", "media");
fs.mkdirSync(media, { recursive: true });
for (const dir of ["audio", "clips"]) {
  for (const f of fs.readdirSync(path.join(work, dir))) {
    if (/\.(mp3|mp4|wav)$/.test(f)) fs.copyFileSync(path.join(work, dir, f), path.join(media, f));
  }
}
fs.copyFileSync(path.join(work, "audio", "durations.json"), path.join("src", "durations.json"));
console.log("media ready:", fs.readdirSync(media).join(", "));
