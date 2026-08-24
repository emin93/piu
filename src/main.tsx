import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import "./styles.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("Più root element is missing");
}

const visualReviewStartup =
  import.meta.env.VITE_PIU_VISUAL_REVIEW_STATE === "loading" ? "loading" : undefined;

createRoot(root).render(
  <StrictMode>
    <App visualReviewStartup={visualReviewStartup} />
  </StrictMode>,
);
