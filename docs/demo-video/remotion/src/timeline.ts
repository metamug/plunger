import durations from "./durations.json";

export const FPS = 30;
export const LEAD = 0.7; // seconds of picture before the narrator starts
export const TAIL = 0.9; // and after they stop
export const XFADE = 15; // frames two scenes overlap in a transition

export const SCENE_IDS = [
  "01_hook",
  "02_problem",
  "03_meet",
  "04_window",
  "04b_bodies",
  "04c_save",
  "05_variables",
  "06_agent",
  "07_workflow",
  "07b_history",
  "08_safe",
  "09_outro",
] as const;
export type SceneId = (typeof SCENE_IDS)[number];

const d = durations as Record<string, number>;

/** Frames a scene lasts: its narration, a lead-in and a tail. */
export const sceneFrames = (id: SceneId) => Math.round((LEAD + d[id] + TAIL) * FPS);
export const narrationSeconds = (id: SceneId) => d[id];

export const totalFrames = () =>
  SCENE_IDS.reduce((sum, id) => sum + sceneFrames(id), 0) - XFADE * (SCENE_IDS.length - 1);

/** seconds to frames */
export const s = (seconds: number) => Math.round(seconds * FPS);
