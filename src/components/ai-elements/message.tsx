// Adapted from AI Elements message.tsx at commit 6a9d5b1822ffb10bba4bd97175f01edd7d8651cd.
// Copyright 2023 Vercel, Inc. Licensed under Apache-2.0.
import type { HTMLAttributes } from "react";

import { cn } from "@/lib/utils";

export type MessageRole = "assistant" | "user";

export type MessageProps = HTMLAttributes<HTMLDivElement> & {
  from: MessageRole;
};

export function Message({ className, from, ...props }: MessageProps) {
  return (
    <div
      className={cn("ai-message", className)}
      data-ai-element="message"
      data-role={from}
      {...props}
    />
  );
}

export type MessageContentProps = HTMLAttributes<HTMLDivElement>;

export function MessageContent({ children, className, ...props }: MessageContentProps) {
  return (
    <div
      className={cn("ai-message-content", className)}
      data-ai-element="message-content"
      {...props}
    >
      {children}
    </div>
  );
}
