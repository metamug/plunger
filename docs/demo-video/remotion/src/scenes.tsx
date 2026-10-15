import React from "react";
import { AbsoluteFill, interpolate, spring, useCurrentFrame } from "remotion";
import { AppWindow, Backdrop, CameraKey, Callout, Chip, clamp, clipPoint, Logo, Rise, s, Terminal, TermLine, useSpring } from "./components";
import { C, MONO } from "./theme";
import { LEAD } from "./timeline";

// Times inside a scene are seconds from the scene's start; the narrator starts at LEAD.
const at = (seconds: number) => s(LEAD + seconds);

// ---- 1. the hook ------------------------------------------------------------------------------------------
export const Hook: React.FC = () => (
  <Backdrop>
    <AbsoluteFill style={{ justifyContent: "center", alignItems: "center", textAlign: "center", gap: 18 }}>
      <Rise at={at(0)} style={{ fontSize: 84, fontWeight: 600, color: C.text }}>
        Your AI agent just called an API.
      </Rise>
      <Rise at={at(2.8)} style={{ fontSize: 104, fontWeight: 800, color: C.amberSoft, letterSpacing: -1 }}>
        What did it send?
      </Rise>
      <Rise at={at(4.6)} style={{ fontSize: 84, fontWeight: 600, color: C.text }}>
        And where did your token go?
      </Rise>
    </AbsoluteFill>
  </Backdrop>
);

// ---- 2. the problem ---------------------------------------------------------------------------------------
const ProblemCard: React.FC<{ label: string; from: number; top: number }> = ({ label, from, top }) => {
  const p = useSpring(from, { damping: 14, stiffness: 140 });
  return (
    <div
      style={{
        position: "absolute",
        left: 1110,
        top,
        width: 720,
        height: 120,
        borderRadius: 20,
        background: "rgba(40,24,26,0.85)",
        border: `2px solid ${C.red}`,
        display: "flex",
        alignItems: "center",
        gap: 26,
        padding: "0 32px",
        opacity: Math.min(1, p),
        transform: `translateX(${(1 - p) * 140}px)`,
        boxShadow: "0 20px 50px rgba(0,0,0,0.4)",
      }}
    >
      <svg width="48" height="48" viewBox="0 0 48 48">
        <circle cx="24" cy="24" r="21" fill="none" stroke={C.red} strokeWidth="4" />
        <path d="M15 15 L33 33 M33 15 L15 33" stroke={C.red} strokeWidth="4" strokeLinecap="round" />
      </svg>
      <div style={{ fontSize: 36, fontWeight: 600, color: C.text }}>{label}</div>
    </div>
  );
};

export const Problem: React.FC = () => {
  const frame = useCurrentFrame();
  const flash = frame > at(5.6) ? 0.55 + 0.45 * Math.sin((frame - at(5.6)) / 4) : 0;
  const box = useSpring(at(0.2));
  return (
    <Backdrop accent={C.red}>
      <Chip label="The problem" color={C.red} />
      <div
        style={{
          position: "absolute",
          left: 96,
          top: 200,
          width: 930,
          padding: "40px 40px",
          borderRadius: 20,
          background: "#0b0d11",
          border: `1px solid ${C.border}`,
          boxShadow: "0 40px 90px rgba(0,0,0,0.55)",
          fontFamily: MONO,
          fontSize: 27,
          lineHeight: "50px",
          color: C.text,
          whiteSpace: "pre",
          opacity: box,
          transform: `translateY(${(1 - box) * 30}px)`,
        }}
      >
        <div>
          <span style={{ color: C.amber }}>$ </span>curl https://api.example.com/orders \
        </div>
        <div>
          {'    -H "Authorization: Bearer '}
          <span style={{ outline: `3px solid rgba(230,100,90,${flash})`, borderRadius: 6, padding: "0 4px" }}>sk_live_9f2a71c0b4e8</span>
          {'"'}
        </div>
        <div>{`    -d '{"user": "ann", "password": "hunter2"}'`}</div>
        <div style={{ color: C.weak, marginTop: 30 }}># and no record of it anywhere</div>
      </div>
      <ProblemCard label="No record of what was sent" from={at(3.9)} top={210} />
      <ProblemCard label="Secrets end up in logs" from={at(6.3)} top={360} />
      <ProblemCard label="Undefined variables go out as text" from={at(8.4)} top={510} />
      <Rise at={at(11.6)} style={{ position: "absolute", top: 820, width: "100%", textAlign: "center", fontSize: 50, fontWeight: 700, color: C.amberSoft }}>
        Nobody can say what was actually sent.
      </Rise>
    </Backdrop>
  );
};

// ---- 3. meet Plunger --------------------------------------------------------------------------------------
const Node: React.FC<{ x: number; label: string; sub: string; color: string; from: number; glyph: React.ReactNode }> = ({ x, label, sub, color, from, glyph }) => {
  const p = useSpring(from, { damping: 14, stiffness: 130 });
  return (
    <div
      style={{
        position: "absolute",
        left: x - 250,
        top: 560,
        width: 500,
        height: 180,
        borderRadius: 24,
        background: "rgba(26,28,34,0.92)",
        border: `3px solid ${color}`,
        display: "flex",
        alignItems: "center",
        gap: 26,
        padding: "0 34px",
        opacity: Math.min(1, p),
        transform: `translateY(${(1 - p) * 60}px) scale(${0.9 + 0.1 * Math.min(1, p)})`,
        boxShadow: `0 24px 60px rgba(0,0,0,0.45), 0 0 40px ${color}22`,
      }}
    >
      <div style={{ color }}>{glyph}</div>
      <div>
        <div style={{ fontSize: 34, fontWeight: 700, color: C.text, whiteSpace: "nowrap" }}>{label}</div>
        <div style={{ fontSize: 26, color: C.weak, fontFamily: label.includes("command") ? MONO : undefined }}>{sub}</div>
      </div>
    </div>
  );
};

export const Meet: React.FC = () => {
  const frame = useCurrentFrame();
  const logo = useSpring(at(0), { damping: 10, stiffness: 120 });
  const core = useSpring(at(11.2));
  const draw = interpolate(frame, [at(11.0), at(12.2)], [0, 1], clamp);
  const nodes = [
    { x: 420, color: C.blue },
    { x: 960, color: C.green },
    { x: 1500, color: C.amber },
  ];
  return (
    <Backdrop>
      <div style={{ position: "absolute", top: 120, width: "100%", display: "flex", justifyContent: "center", alignItems: "center", gap: 40 }}>
        <div style={{ transform: `scale(${logo}) rotate(${(1 - logo) * -25}deg)` }}>
          <Logo size={120} />
        </div>
        <div>
          <Rise at={at(0.2)} style={{ fontSize: 150, fontWeight: 800, color: C.text, letterSpacing: -3, lineHeight: 1 }}>
            Plunger
          </Rise>
          <Rise at={at(1.6)} style={{ fontSize: 44, color: C.weak, marginTop: 14 }}>
            An HTTP client for you, and for your agent.
          </Rise>
        </div>
      </div>
      <svg style={{ position: "absolute", left: 0, top: 0 }} width={1920} height={1080}>
        {nodes.map((n) => (
          <line key={n.x} x1={n.x} y1={740} x2={n.x} y2={740 + 150 * draw} stroke={n.color} strokeWidth={4} strokeLinecap="round" opacity={0.8} />
        ))}
      </svg>
      <Node
        x={420}
        label="The window"
        sub="see it, edit it"
        color={C.blue}
        from={at(6.6)}
        glyph={
          <svg width="64" height="54" viewBox="0 0 64 54">
            <rect x="3" y="3" width="58" height="48" rx="8" fill="none" stroke="currentColor" strokeWidth="5" />
            <line x1="3" y1="17" x2="61" y2="17" stroke="currentColor" strokeWidth="4" />
          </svg>
        }
      />
      <Node
        x={960}
        label="The command line"
        sub="plunger send …"
        color={C.green}
        from={at(7.8)}
        glyph={
          <svg width="64" height="54" viewBox="0 0 64 54">
            <path d="M8 10 L28 27 L8 44" fill="none" stroke="currentColor" strokeWidth="6" strokeLinecap="round" strokeLinejoin="round" />
            <line x1="34" y1="44" x2="58" y2="44" stroke="currentColor" strokeWidth="6" strokeLinecap="round" />
          </svg>
        }
      />
      <Node
        x={1500}
        label="The MCP server"
        sub="for your agent"
        color={C.amber}
        from={at(9.4)}
        glyph={
          <svg width="64" height="64" viewBox="0 0 64 64">
            <circle cx="32" cy="32" r="26" fill="none" stroke="currentColor" strokeWidth="5" />
            <circle cx="32" cy="32" r="10" fill="currentColor" />
          </svg>
        }
      />
      <div
        style={{
          position: "absolute",
          left: 360,
          top: 890,
          width: 1200,
          height: 110,
          borderRadius: 24,
          background: "rgba(48,36,20,0.9)",
          border: `3px solid ${C.amber}`,
          display: "flex",
          justifyContent: "center",
          alignItems: "center",
          gap: 50,
          fontSize: 40,
          fontWeight: 600,
          color: C.amberSoft,
          opacity: core,
          transform: `scale(${0.92 + 0.08 * core})`,
          boxShadow: `0 0 60px ${C.amber}33`,
        }}
      >
        <span>Saved requests</span>
        <span style={{ opacity: 0.5 }}>·</span>
        <span>Variables</span>
        <span style={{ opacity: 0.5 }}>·</span>
        <span>History</span>
      </div>
    </Backdrop>
  );
};

// ---- the real window, with callouts that follow the camera ------------------------------------------------
type Point = { from: number; to: number; px: number; py: number; label: string; side?: "left" | "right"; color?: string };

const AppScene: React.FC<{
  title: string;
  src: string;
  startAt: number;
  rate: number;
  camera: CameraKey[];
  points: Point[]; // from/to are clip seconds; px/py clip pixels
  win?: { left: number; top: number; width: number };
}> = ({ title, src, startAt, rate, camera, points, win = { left: 290, top: 170, width: 1340 } }) => {
  const frame = useCurrentFrame();
  const clipT = startAt + (frame / 30) * rate;
  // the clip starts playing at the scene start; scene frames for a clip time:
  const sceneFrame = (clipSeconds: number) => Math.round(((clipSeconds - startAt) / rate) * 30);
  return (
    <Backdrop>
      <Chip label={title} />
      <AppWindow src={src} startAt={startAt} rate={rate} camera={camera} {...win} />
      {points.map((p, i) => {
        const pos = clipPoint(p.px, p.py, win, camera, clipT);
        return <Callout key={i} from={sceneFrame(p.from)} to={sceneFrame(p.to)} x={pos.x} y={pos.y} label={p.label} side={p.side} color={p.color} />;
      })}
    </Backdrop>
  );
};

export const WindowScene: React.FC = () => (
  <AppScene
    title="The window"
    src="window.mp4"
    startAt={0.6}
    rate={1.25}
    camera={[
      { at: 0, x: 0.5, y: 0.5, scale: 1 },
      { at: 2.0, x: 0.55, y: 0.18, scale: 1.45 },
      { at: 8.6, x: 0.55, y: 0.25, scale: 1.45 },
      { at: 10.5, x: 0.62, y: 0.55, scale: 1.25 },
      { at: 16.5, x: 0.62, y: 0.55, scale: 1.25 },
      { at: 18.0, x: 0.5, y: 0.5, scale: 1 },
    ]}
    points={[
      { from: 7.6, to: 9.8, px: 520, py: 240, label: "Query params stay in sync with the URL" },
      { from: 13.8, to: 16.6, px: 600, py: 460, label: "Fold it, search it, copy a path", side: "left" },
      { from: 18.4, to: 21.0, px: 150, py: 250, label: "Saved requests, in groups" },
    ]}
  />
);

export const BodiesScene: React.FC = () => (
  <AppScene
    title="Any body"
    src="bodies.mp4"
    startAt={0.4}
    rate={1.35}
    camera={[
      { at: 0, x: 0.5, y: 0.5, scale: 1 },
      { at: 6.0, x: 0.5, y: 0.5, scale: 1 },
      { at: 8.0, x: 0.75, y: 0.28, scale: 1.3 },
      { at: 14.6, x: 0.75, y: 0.28, scale: 1.3 },
      { at: 16.2, x: 0.6, y: 0.5, scale: 1.12 },
    ]}
    points={[
      { from: 6.8, to: 8.6, px: 1000, py: 189, label: "JSON, form, multipart or raw", side: "left" },
      { from: 12.4, to: 14.6, px: 1112, py: 189, label: "Format tidies it up", side: "left" },
      { from: 16.4, to: 21.0, px: 470, py: 470, label: "Exactly what the server received" },
    ]}
  />
);

export const SaveScene: React.FC = () => (
  <AppScene
    title="Saved, and organised"
    src="save.mp4"
    startAt={0.6}
    rate={1.0}
    camera={[
      { at: 0, x: 0.1, y: 0.4, scale: 1.2 },
      { at: 12, x: 0.1, y: 0.45, scale: 1.2 },
    ]}
    points={[
      { from: 2.2, to: 4.4, px: 80, py: 166, label: "Ctrl+S, then name it" },
      { from: 7.6, to: 9.8, px: 330, py: 444, label: "A new group, from the menu" },
      { from: 11.2, to: 13.6, px: 80, py: 600, label: "Filed under Orders" },
    ]}
  />
);

export const HistoryScene: React.FC = () => (
  <AppScene
    title="History, and a light theme"
    src="history.mp4"
    startAt={0.4}
    rate={1.05}
    camera={[{ at: 0, x: 0.5, y: 0.5, scale: 1 }]}
    points={[
      { from: 2.4, to: 5.0, px: 70, py: 372, label: "Sent by your agent, tagged CLI" },
      { from: 5.0, to: 7.4, px: 600, py: 587, label: "How it went, and who sent it" },
      { from: 9.0, to: 12.4, px: 300, py: 300, label: "One click to the light theme" },
    ]}
  />
);

export const VariablesScene: React.FC = () => (
  <AppScene
    title="Variables, never secrets in plain sight"
    src="variables.mp4"
    startAt={0.6}
    rate={1.0}
    camera={[
      { at: 0, x: 0.5, y: 0.5, scale: 1 },
      { at: 7.0, x: 0.5, y: 0.5, scale: 1 },
      { at: 9.0, x: 0.45, y: 0.22, scale: 1.5 },
    ]}
    points={[
      { from: 2.6, to: 5.6, px: 520, py: 210, label: "Shared by every request, the CLI and agents" },
      { from: 9.4, to: 11.4, px: 340, py: 181, label: "Each request shows the values it uses" },
      { from: 11.6, to: 15.0, px: 520, py: 281, label: "Undefined? Refused, and named" },
    ]}
  />
);

export const WorkflowScene: React.FC = () => (
  <AppScene
    title="Workflows"
    src="workflow.mp4"
    startAt={0.3}
    rate={0.95}
    camera={[
      { at: 0, x: 0.5, y: 0.5, scale: 1 },
      { at: 2.0, x: 0.5, y: 0.5, scale: 1 },
      { at: 3.6, x: 0.05, y: 0.6, scale: 1.22 },
    ]}
    points={[
      { from: 4.6, to: 7.0, px: 60, py: 300, label: "Step 1 asks the tracker for the 'Done' transition", side: "right" },
      { from: 7.2, to: 9.6, px: 180, py: 395, label: "…and hands its id to step 2" },
      { from: 9.8, to: 12.6, px: 545, py: 442, label: "All steps passed", color: C.green },
    ]}
  />
);

// ---- the agent: a terminal and the window, side by side ---------------------------------------------------
const AGENT_TERM: TermLine[] = [
  { at: 1.2, kind: "cmd", text: 'plunger send "Jira: get issue" \\\n  --select $.fields.summary \\\n  --select $.fields.status.name' },
  {
    at: 4.4,
    kind: "out",
    text: '{\n  "ok": true,\n  "status": 200,\n  "elapsed_ms": 704,\n  "selected": {\n    "$.fields.status.name": "Done",\n    "$.fields.summary": "Checkout fails on retry"\n  }\n}',
  },
  { at: 9.4, kind: "cmd", text: 'plunger send "Jira: search" --select $.total' },
  { at: 11.6, kind: "out", text: '{ "ok": true, "status": 200, "selected": { "$.total": 0 } }' },
  { at: 15.2, kind: "cmd", text: 'plunger send --url {{jira_base}}/issue/{{key}}/transitions \\\n  -H "Authorization: Bearer {{jira_key}}"' },
  { at: 18.4, kind: "out", text: '{ "ok": true, "status": 200 }' },
  { at: 18.9, kind: "ok", text: "# the token was used, and never printed" },
];

export const AgentScene: React.FC = () => {
  const frame = useCurrentFrame();
  return (
    <Backdrop>
      <Chip label="Your agent" />
      <Terminal left={60} top={170} width={900} height={840} script={AGENT_TERM} title="the agent's terminal" fontSize={24} appearAt={2} />
      <AppWindow src="agent.mp4" startAt={0.2} rate={1.0} left={1000} top={250} width={880} appearAt={10} />
      <Callout from={s(6.0)} to={s(9.0)} x={1050} y={780} label="Every call lands in your history, tagged CLI" side="right" />
      <Callout from={s(20.0)} to={s(24.0)} x={500} y={540} label="Only the values it asked for" side="right" color={C.green} />
      {frame < 0 && null}
    </Backdrop>
  );
};

// ---- safe by default --------------------------------------------------------------------------------------
const SAFE_TERM: TermLine[] = [
  { at: 0.8, kind: "cmd", text: "plunger send --url http://localhost:3000/login -X POST \\\n  --json '{\"user\":\"ann\",\"password\":\"hunter2\"}'" },
  { at: 3.6, kind: "out", text: '{ "ok": true, "status": 200, "request": { "history_id": 31 } }' },
  { at: 5.0, kind: "cmd", text: "findstr hunter2 history.sqlite3 audit.log" },
  { at: 6.6, kind: "ok", text: "(no matches: the password was never written down)" },
  { at: 7.8, kind: "cmd", text: "plunger audit verify" },
  { at: 8.8, kind: "ok", text: '{ "ok": true, "entries": 44, "head": "e92dae55…" }' },
];

const Check: React.FC<{ label: string; from: number; top: number }> = ({ label, from, top }) => {
  const p = useSpring(from, { damping: 13, stiffness: 140 });
  const frame = useCurrentFrame();
  const draw = interpolate(frame, [from + 4, from + 16], [40, 0], clamp);
  return (
    <div
      style={{
        position: "absolute",
        left: 1330,
        top,
        width: 510,
        height: 112,
        borderRadius: 22,
        background: "rgba(20,34,28,0.92)",
        border: `2px solid ${C.green}`,
        display: "flex",
        alignItems: "center",
        gap: 24,
        padding: "0 30px",
        opacity: Math.min(1, p),
        transform: `translateX(${(1 - p) * 90}px)`,
      }}
    >
      <svg width="46" height="40" viewBox="0 0 46 40">
        <path d="M4 22 L17 34 L42 6" fill="none" stroke={C.green} strokeWidth="6" strokeLinecap="round" strokeLinejoin="round" strokeDasharray="40" strokeDashoffset={draw} />
      </svg>
      <div style={{ fontSize: 38, fontWeight: 600, color: C.text }}>{label}</div>
    </div>
  );
};

export const SafeScene: React.FC = () => (
  <Backdrop accent={C.green}>
    <Chip label="Safe by default" color={C.green} />
    <Terminal left={96} top={180} width={1180} height={600} script={SAFE_TERM} title="plunger" fontSize={26} />
    <Check label="Headers" from={at(3.0)} top={200} />
    <Check label="Query strings" from={at(3.9)} top={340} />
    <Check label="Request bodies" from={at(4.8)} top={480} />
    <Check label="Hash-chained log" from={at(7.0)} top={620} />
    <Rise at={at(8.4)} style={{ position: "absolute", top: 860, width: "100%", textAlign: "center", fontSize: 52, fontWeight: 700, color: C.amberSoft }}>
      Masked before anything touches the disk.
    </Rise>
  </Backdrop>
);

// ---- the end ----------------------------------------------------------------------------------------------
const Install: React.FC<{ text: string; from: number }> = ({ text, from }) => {
  const p = useSpring(from);
  return (
    <div
      style={{
        fontFamily: MONO,
        fontSize: 40,
        color: C.green,
        padding: "16px 40px",
        borderRadius: 16,
        background: "#0b0d11",
        border: `1px solid ${C.border}`,
        opacity: p,
        transform: `translateY(${(1 - p) * 24}px)`,
      }}
    >
      <span style={{ color: C.amber }}>$ </span>
      {text}
    </div>
  );
};

export const Outro: React.FC = () => {
  const frame = useCurrentFrame();
  const logo = spring({ frame: frame - at(0), fps: 30, config: { damping: 10, stiffness: 110 } });
  return (
    <Backdrop>
      <AbsoluteFill style={{ alignItems: "center", paddingTop: 90, gap: 26 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 30, transform: `scale(${0.8 + 0.2 * logo})`, opacity: logo }}>
          <Logo size={86} />
          <div style={{ fontSize: 120, fontWeight: 800, color: C.text, letterSpacing: -3 }}>Plunger</div>
        </div>
        <Rise at={at(0.6)} style={{ fontSize: 34, color: C.weak, marginBottom: 20 }}>
          Open source · MIT licensed
        </Rise>
        <Install text="pip install plunger-cli" from={at(2.4)} />
        <Install text="brew install --cask metamug/tap/plunger" from={at(3.4)} />
        <Install text="winget install 9P7WRKLN6WHR" from={at(4.4)} />
        <Install text="plunger install" from={at(6.0)} />
        <Rise at={at(8.6)} style={{ fontSize: 84, fontWeight: 800, color: C.amberSoft, marginTop: 30 }}>
          Unclog your API.
        </Rise>
        <Rise at={at(9.4)} style={{ fontSize: 32, color: C.weak }}>
          github.com/metamug/plunger
        </Rise>
      </AbsoluteFill>
    </Backdrop>
  );
};
