import React from "react";
import {
  AbsoluteFill,
  Easing,
  interpolate,
  OffthreadVideo,
  spring,
  staticFile,
  useCurrentFrame,
  useVideoConfig,
} from "remotion";
import { C, FONT, MONO } from "./theme";
import { s } from "./timeline";

export const clamp = { extrapolateLeft: "clamp", extrapolateRight: "clamp" } as const;

/** 0 -> 1 with a soft spring, starting at `from` frames. */
export const useSpring = (from: number, config: { damping?: number; mass?: number; stiffness?: number } = {}) => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();
  return spring({ frame: frame - from, fps, config: { damping: 18, mass: 0.8, stiffness: 120, ...config } });
};

/** Near-black with two slow glows and a faint grid: the film's backdrop. */
export const Backdrop: React.FC<{ accent?: string; children?: React.ReactNode }> = ({ accent = C.amber, children }) => {
  const frame = useCurrentFrame();
  const drift = Math.sin(frame / 90) * 40;
  return (
    <AbsoluteFill style={{ background: C.bg, fontFamily: FONT, overflow: "hidden" }}>
      <AbsoluteFill
        style={{
          background: `radial-gradient(900px 700px at ${1500 + drift}px ${120 - drift / 2}px, ${accent}33, transparent 70%),
                       radial-gradient(800px 600px at ${150 - drift}px ${1000 + drift / 3}px, ${C.blue}22, transparent 70%)`,
        }}
      />
      <AbsoluteFill
        style={{
          backgroundImage: `linear-gradient(${C.text}08 1px, transparent 1px), linear-gradient(90deg, ${C.text}08 1px, transparent 1px)`,
          backgroundSize: "64px 64px",
          maskImage: "radial-gradient(ellipse at center, black 30%, transparent 80%)",
          WebkitMaskImage: "radial-gradient(ellipse at center, black 30%, transparent 80%)",
        }}
      />
      {children}
    </AbsoluteFill>
  );
};

/** The scene's label, top left. */
export const Chip: React.FC<{ label: string; color?: string; delay?: number }> = ({ label, color = C.amber, delay = 6 }) => {
  const p = useSpring(delay);
  return (
    <div
      style={{
        position: "absolute",
        left: 72,
        top: 56,
        padding: "12px 26px",
        borderRadius: 999,
        border: `2px solid ${color}`,
        background: `${color}22`,
        color,
        fontSize: 28,
        fontWeight: 600,
        letterSpacing: 0.3,
        opacity: p,
        transform: `translateY(${(1 - p) * -20}px)`,
      }}
    >
      {label}
    </div>
  );
};

/** Text that rises in with a little blur, word group by word group. */
export const Rise: React.FC<{
  children: React.ReactNode;
  at: number;
  style?: React.CSSProperties;
}> = ({ children, at, style }) => {
  const p = useSpring(at, { damping: 20, stiffness: 90 });
  return (
    <div
      style={{
        opacity: p,
        transform: `translateY(${(1 - p) * 40}px)`,
        filter: `blur(${(1 - p) * 10}px)`,
        ...style,
      }}
    >
      {children}
    </div>
  );
};

/** A pulsing marker with a label card beside it, shown between `from` and `to` (frames). */
export const Callout: React.FC<{
  from: number;
  to: number;
  x: number;
  y: number;
  label: string;
  side?: "left" | "right";
  color?: string;
}> = ({ from, to, x, y, label, side = "right", color = C.amber }) => {
  const frame = useCurrentFrame();
  if (frame < from - 1 || frame > to + 12) return null;
  const inP = spring({ frame: frame - from, fps: 30, config: { damping: 16, stiffness: 140 } });
  const out = interpolate(frame, [to, to + 10], [1, 0], clamp);
  const a = Math.min(inP, out);
  const pulse = ((frame - from) / 22) % 1;
  return (
    <>
      <div
        style={{
          position: "absolute",
          left: x - 30,
          top: y - 30,
          width: 60,
          height: 60,
          borderRadius: "50%",
          border: `3px solid ${color}`,
          opacity: (1 - pulse) * a,
          transform: `scale(${0.4 + pulse * 1.1})`,
        }}
      />
      <div
        style={{
          position: "absolute",
          left: x - 9,
          top: y - 9,
          width: 18,
          height: 18,
          borderRadius: "50%",
          background: color,
          boxShadow: `0 0 24px ${color}`,
          opacity: a,
          transform: `scale(${a})`,
        }}
      />
      <div
        style={{
          position: "absolute",
          top: y - 34,
          ...(side === "right" ? { left: x + 46 } : { right: 1920 - x + 46 }),
          padding: "14px 24px",
          borderRadius: 16,
          background: "rgba(22,24,30,0.94)",
          border: `2px solid ${color}`,
          boxShadow: "0 18px 40px rgba(0,0,0,0.45)",
          color: C.text,
          fontSize: 30,
          fontWeight: 600,
          whiteSpace: "nowrap",
          opacity: a,
          transform: `translateX(${(1 - inP) * (side === "right" ? 30 : -30)}px)`,
        }}
      >
        {label}
      </div>
    </>
  );
};

export type CameraKey = { at: number; x: number; y: number; scale: number }; // at: clip seconds; x,y: 0..1 of the clip

/** The real app, playing a screen recording, as a floating window with a shadow and a slow camera. */
export const AppWindow: React.FC<{
  src: string;
  /** clip seconds where this scene starts, and how fast it plays */
  startAt: number;
  rate?: number;
  /** top-left of the window on the canvas and its width (the height follows) */
  left: number;
  top: number;
  width: number;
  camera?: CameraKey[];
  appearAt?: number;
}> = ({ src, startAt, rate = 1, left, top, width, camera = [], appearAt = 0 }) => {
  const frame = useCurrentFrame();
  const CLIP_W = 1164; // the window inside the 1180x780 capture (8 px of desktop each side)
  const CLIP_H = 772;
  const k = width / CLIP_W;
  const height = CLIP_H * k;
  const clipT = startAt + (frame / 30) * rate;
  const cam = cameraAt(camera, clipT);
  const p = spring({ frame: frame - appearAt, fps: 30, config: { damping: 20, stiffness: 90 } });
  return (
    <div
      style={{
        position: "absolute",
        left,
        top,
        width,
        height,
        borderRadius: 14,
        overflow: "hidden",
        boxShadow: "0 40px 90px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,255,255,0.08)",
        opacity: p,
        transform: `translateY(${(1 - p) * 40}px) scale(${0.96 + 0.04 * p})`,
      }}
    >
      <div
        style={{
          position: "absolute",
          left: 0,
          top: 0,
          width,
          height,
          transformOrigin: `${cam.x * 100}% ${cam.y * 100}%`,
          transform: `scale(${cam.scale})`,
        }}
      >
        <OffthreadVideo
          src={staticFile(`media/${src}`)}
          startFrom={Math.round(startAt * 30)}
          playbackRate={rate}
          muted
          style={{ position: "absolute", left: -8 * k, top: 0, width: 1180 * k, height: 780 * k }}
        />
      </div>
    </div>
  );
};

export const cameraAt = (keys: CameraKey[], t: number) => {
  if (keys.length === 0) return { x: 0.5, y: 0.5, scale: 1 };
  if (t <= keys[0].at) return keys[0];
  for (let i = 0; i < keys.length - 1; i++) {
    const a = keys[i];
    const b = keys[i + 1];
    if (t >= a.at && t <= b.at) {
      const e = Easing.inOut(Easing.cubic)((t - a.at) / (b.at - a.at));
      return { at: t, x: a.x + (b.x - a.x) * e, y: a.y + (b.y - a.y) * e, scale: a.scale + (b.scale - a.scale) * e };
    }
  }
  return keys[keys.length - 1];
};

/** Where a point of the clip (in its 1164x772 pixels) ends up on the canvas, camera included. */
export const clipPoint = (
  px: number,
  py: number,
  win: { left: number; top: number; width: number },
  camera: CameraKey[],
  clipT: number,
) => {
  const k = win.width / 1164;
  const h = 772 * k;
  const cam = cameraAt(camera, clipT);
  const ox = cam.x * win.width;
  const oy = cam.y * h;
  const x = ox + (px * k - ox) * cam.scale;
  const y = oy + (py * k - oy) * cam.scale;
  return { x: win.left + x, y: win.top + y };
};

export type TermLine = { at: number; kind: "cmd" | "out" | "ok" | "err"; text: string };

/** A terminal that types its commands (about 40 characters a second) and prints real output. */
export const Terminal: React.FC<{
  left: number;
  top: number;
  width: number;
  height: number;
  script: TermLine[];
  title?: string;
  fontSize?: number;
  appearAt?: number;
}> = ({ left, top, width, height, script, title = "terminal", fontSize = 24, appearAt = 0 }) => {
  const frame = useCurrentFrame();
  const t = frame / 30;
  const p = spring({ frame: frame - appearAt, fps: 30, config: { damping: 20, stiffness: 90 } });
  const lineH = fontSize * 1.5;
  const rows: { text: string; color: string; prompt: "" | "$" | " "; caret: boolean }[] = [];
  for (const line of script) {
    if (t < line.at) break;
    if (line.kind === "cmd") {
      const n = Math.floor((t - line.at) * 40);
      const shown = line.text.slice(0, Math.max(0, n));
      const typing = shown.length < line.text.length;
      const parts = shown.split("\n");
      parts.forEach((part, i) => rows.push({ text: part, color: C.text, prompt: i === 0 ? "$" : " ", caret: typing && i === parts.length - 1 }));
    } else {
      const color = line.kind === "ok" ? C.green : line.kind === "err" ? C.red : C.weak;
      for (const ln of line.text.split("\n")) rows.push({ text: ln, color, prompt: "", caret: false });
    }
  }
  const maxRows = Math.floor((height - 60 - 36) / lineH);
  const visible = rows.slice(-maxRows);
  const blink = Math.floor(frame / 15) % 2 === 0;
  return (
    <div
      style={{
        position: "absolute",
        left,
        top,
        width,
        height,
        borderRadius: 18,
        background: "#0b0d11",
        border: `1px solid ${C.border}`,
        boxShadow: "0 40px 90px rgba(0,0,0,0.55)",
        overflow: "hidden",
        opacity: p,
        transform: `translateY(${(1 - p) * 40}px)`,
      }}
    >
      <div style={{ height: 52, background: "#1c1e25", display: "flex", alignItems: "center", padding: "0 20px", gap: 10 }}>
        {["#ff5f56", "#ffbd2e", "#27c93f"].map((c) => (
          <div key={c} style={{ width: 15, height: 15, borderRadius: "50%", background: c }} />
        ))}
        <div style={{ flex: 1, textAlign: "center", color: C.weak, fontSize: 20, marginRight: 70 }}>{title}</div>
      </div>
      <div style={{ padding: "22px 28px", fontFamily: MONO, fontSize, lineHeight: `${lineH}px` }}>
        {visible.map((r, i) => (
          <div key={i} style={{ whiteSpace: "pre", color: r.color }}>
            {r.prompt === "$" && <span style={{ color: C.amber }}>$ </span>}
            {r.prompt === " " && <span>{"  "}</span>}
            {r.text}
            {r.caret && blink && (
              <span style={{ display: "inline-block", width: fontSize * 0.55, height: fontSize * 1.05, background: C.amber, verticalAlign: "middle", marginLeft: 2 }} />
            )}
          </div>
        ))}
      </div>
    </div>
  );
};

/** The plunger mark (docs/images/plunger.svg). */
export const Logo: React.FC<{ size: number; style?: React.CSSProperties }> = ({ size, style }) => (
  <img src={staticFile("plunger.svg")} style={{ width: size, height: size * 1.3, ...style }} />
);

export { s };
