import React from "react";
import { Composition, registerRoot } from "remotion";
import { Film } from "./Film";
import { FPS, totalFrames } from "./timeline";

const Root: React.FC = () =>
  React.createElement(Composition, {
    id: "Film",
    component: Film,
    durationInFrames: totalFrames(),
    fps: FPS,
    width: 1920,
    height: 1080,
  });

registerRoot(Root);
