import { loadFont as loadInter } from "@remotion/google-fonts/Inter";
import { loadFont as loadMono } from "@remotion/google-fonts/JetBrainsMono";

const inter = loadInter("normal", { weights: ["400", "500", "600", "700", "800"], subsets: ["latin"] });
const mono = loadMono("normal", { weights: ["400", "600"], subsets: ["latin"] });

export const FONT = inter.fontFamily;
export const MONO = mono.fontFamily;

// the app's own palette
export const C = {
  bg: "#0e1015",
  panel: "#16181d",
  card: "#1e2128",
  border: "#34373f",
  text: "#e8e8e8",
  weak: "#969aa4",
  amber: "#f09628",
  amberSoft: "#ffcd82",
  green: "#5ad28c",
  red: "#e6645a",
  blue: "#6ea0f0",
  purple: "#b48ceb",
};
