// Adapted from AI Elements tool.tsx at commit 6a9d5b1822ffb10bba4bd97175f01edd7d8651cd.
// Copyright 2023 Vercel, Inc. Licensed under Apache-2.0.
import type { ComponentProps } from "react";

import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

export type ToolProps = ComponentProps<typeof Collapsible>;

export function Tool({ className, ...props }: ToolProps) {
  return <Collapsible className={cn("ai-tool", className)} data-ai-element="tool" {...props} />;
}

export type ToolHeaderProps = ComponentProps<typeof CollapsibleTrigger>;

export function ToolHeader({ className, ...props }: ToolHeaderProps) {
  return (
    <CollapsibleTrigger
      className={cn("ai-tool-header", className)}
      data-ai-element="tool-header"
      {...props}
    />
  );
}

export type ToolContentProps = ComponentProps<typeof CollapsibleContent>;

export function ToolContent({ className, ...props }: ToolContentProps) {
  return (
    <CollapsibleContent
      className={cn("ai-tool-content", className)}
      data-ai-element="tool-content"
      {...props}
    />
  );
}
