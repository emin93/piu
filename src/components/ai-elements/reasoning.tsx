// Adapted from AI Elements reasoning.tsx at commit 6a9d5b1822ffb10bba4bd97175f01edd7d8651cd.
// Copyright 2023 Vercel, Inc. Licensed under Apache-2.0.
import type { ComponentProps } from "react";

import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

export type ReasoningProps = ComponentProps<typeof Collapsible>;

export function Reasoning({ className, ...props }: ReasoningProps) {
  return (
    <Collapsible className={cn("ai-reasoning", className)} data-ai-element="reasoning" {...props} />
  );
}

export type ReasoningTriggerProps = ComponentProps<typeof CollapsibleTrigger>;

export function ReasoningTrigger({ className, ...props }: ReasoningTriggerProps) {
  return (
    <CollapsibleTrigger
      className={cn("ai-reasoning-trigger", className)}
      data-ai-element="reasoning-trigger"
      {...props}
    />
  );
}

export type ReasoningContentProps = ComponentProps<typeof CollapsibleContent>;

export function ReasoningContent({ className, ...props }: ReasoningContentProps) {
  return (
    <CollapsibleContent
      className={cn("ai-reasoning-content", className)}
      data-ai-element="reasoning-content"
      {...props}
    />
  );
}
