import React from "react";
import { AbsoluteFill, Audio, interpolate, Sequence, staticFile, useVideoConfig } from "remotion";
import { linearTiming, TransitionSeries } from "@remotion/transitions";
import { fade } from "@remotion/transitions/fade";
import { AgentScene, BodiesScene, HistoryScene, Hook, Meet, Outro, Problem, SafeScene, SaveScene, VariablesScene, WindowScene, WorkflowScene } from "./scenes";
import { LEAD, s, SCENE_IDS, SceneId, sceneFrames, XFADE } from "./timeline";

const SCENES: Record<SceneId, React.FC> = {
  "01_hook": Hook,
  "02_problem": Problem,
  "03_meet": Meet,
  "04_window": WindowScene,
  "04b_bodies": BodiesScene,
  "04c_save": SaveScene,
  "05_variables": VariablesScene,
  "06_agent": AgentScene,
  "07_workflow": WorkflowScene,
  "07b_history": HistoryScene,
  "08_safe": SafeScene,
  "09_outro": Outro,
};

/** One scene with its narration on top. */
const Narrated: React.FC<{ id: SceneId }> = ({ id }) => {
  const Scene = SCENES[id];
  return (
    <AbsoluteFill>
      <Scene />
      <Sequence from={s(LEAD)}>
        <Audio src={staticFile(`media/${id}.mp3`)} />
      </Sequence>
    </AbsoluteFill>
  );
};

export const Film: React.FC = () => {
  const { durationInFrames } = useVideoConfig();
  return (
    <AbsoluteFill style={{ background: "#000" }}>
      <TransitionSeries>
        {SCENE_IDS.flatMap((id, i) => {
          const items = [
            <TransitionSeries.Sequence key={id} durationInFrames={sceneFrames(id)}>
              <Narrated id={id} />
            </TransitionSeries.Sequence>,
          ];
          if (i < SCENE_IDS.length - 1) {
            items.push(<TransitionSeries.Transition key={`${id}-t`} presentation={fade()} timing={linearTiming({ durationInFrames: XFADE })} />);
          }
          return items;
        })}
      </TransitionSeries>
      {/* a quiet bed under the voice; it fades in and out with the film */}
      <Audio
        src={staticFile("media/music.wav")}
        volume={(f) =>
          interpolate(f, [0, 60, durationInFrames - 120, durationInFrames], [0, 0.11, 0.11, 0], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          })
        }
      />
    </AbsoluteFill>
  );
};
