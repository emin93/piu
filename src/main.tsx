import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import {
  visualReviewConnectionRecoveryAdapter,
  visualReviewConversationAdapter,
  visualReviewSendRecoveryAdapter,
} from "./features/conversation/visual-review-adapter";
import "./styles.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("Più root element is missing");
}

const visualReviewState =
  import.meta.env.VITE_PIU_VISUAL_REVIEW_STATE === "loading"
    ? "loading"
    : import.meta.env.VITE_PIU_VISUAL_REVIEW_STATE === "closeConfirmation"
      ? "closeConfirmation"
      : import.meta.env.VITE_PIU_VISUAL_REVIEW_STATE === "connectionRecovery"
        ? "connectionRecovery"
        : import.meta.env.VITE_PIU_VISUAL_REVIEW_STATE === "conversation"
          ? "conversation"
          : import.meta.env.VITE_PIU_VISUAL_REVIEW_STATE === "sendRecovery"
            ? "sendRecovery"
            : undefined;

const visualReviewConversation =
  visualReviewState === "connectionRecovery"
    ? visualReviewConnectionRecoveryAdapter
    : visualReviewState === "conversation"
      ? visualReviewConversationAdapter
      : visualReviewState === "sendRecovery"
        ? visualReviewSendRecoveryAdapter
        : undefined;

createRoot(root).render(
  <StrictMode>
    <App conversationAdapter={visualReviewConversation} visualReviewState={visualReviewState} />
  </StrictMode>,
);
