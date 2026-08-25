import { Profiler, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/profiling";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import { TooltipProvider } from "@/components/ui/tooltip";
import { ProjectDraftController } from "@/features/inbox/draft-controller";
import { ChatActivityController } from "@/features/inbox/chat-activity-controller";
import { InboxWorkspace } from "@/features/inbox/InboxWorkspace";
import { ChatSetupController } from "@/features/inbox/setup-controller";
import type { ConversationAdapter, ConversationEvent } from "@/platform/conversations";
import type { InboxSnapshot } from "@/platform/project-inbox";
import "@/styles.css";
import "./harness.css";

const RESULT_PREFIX = "PIU_ISSUE_5_PERFORMANCE:";
const CHAT_SWITCH_SAMPLES = 30;
const NAVIGATION_SAMPLES = 30;
const INPUT_SAMPLES = 60;
const FRAME_SAMPLES = 120;

type Scenario = "chatSwitch" | "composerInput" | "navigation" | "scrolling" | "streaming";

interface Summary {
  count: number;
  max: number;
  median: number;
  min: number;
  p95: number;
}

interface FrameSummary extends Summary {
  framesOver20ms: number;
  observedFps: number;
}

const readySetup = {
  attempt: 1,
  exitCode: 0,
  failure: null,
  log: "",
  phase: "succeeded" as const,
  signal: null,
};

const performanceAttachments = Array.from({ length: 3 }, (_, index) => ({
  content: "A".repeat(2 * 1024 * 1024),
  id: `performance-attachment-${index}`,
  kind: "image" as const,
  mimeType: "image/png",
  name: `reference-${index + 1}.png`,
  sizeBytes: 2 * 1024 * 1024,
}));

const snapshot: InboxSnapshot = {
  chats: Array.from({ length: 24 }, (_, index) => ({
    branchName: `agent/performance-chat-${index}`,
    createdAtMs: 1_730_000_000_000 - index,
    id: `performance-chat-${index}`,
    mergeState: "unmerged" as const,
    projectId: 1,
    projectName: "Atlas",
    pullRequestNumber: null,
    setup: readySetup,
    title: `Performance conversation ${index}`,
  })),
  drafts: [
    {
      attachments: performanceAttachments,
      projectId: 1,
      prompt: "",
      updatedAtMs: 1_730_000_000_000,
    },
  ],
  projects: [{ availability: "available", id: 1, name: "Atlas", unmergedChatCount: 24 }],
};

function conversationItems(chatId: string) {
  const items = Array.from({ length: 180 }, (_, index) => ({
    id: `${chatId}-message-${index}`,
    kind: "message" as const,
    queued: false,
    role: index % 2 === 0 ? ("user" as const) : ("assistant" as const),
    text: `Representative transcript line ${index}. This text exercises wrapping and progressive transcript rendering in the production conversation surface.`,
  }));
  items.push({
    id: `stream-${chatId}`,
    kind: "message",
    queued: false,
    role: "assistant",
    text: `Conversation marker ${chatId}`,
  });
  return items;
}

function percentile(sorted: readonly number[], fraction: number) {
  if (sorted.length === 0) return 0;
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)];
}

function summarize(samples: readonly number[]): Summary {
  const sorted = [...samples].sort((left, right) => left - right);
  return {
    count: sorted.length,
    max: Number((sorted.at(-1) ?? 0).toFixed(2)),
    median: Number(percentile(sorted, 0.5).toFixed(2)),
    min: Number((sorted[0] ?? 0).toFixed(2)),
    p95: Number(percentile(sorted, 0.95).toFixed(2)),
  };
}

function summarizeFrames(timestamps: readonly number[]): FrameSummary {
  const intervals = timestamps.slice(1).map((timestamp, index) => timestamp - timestamps[index]);
  const summary = summarize(intervals);
  const elapsed = (timestamps.at(-1) ?? 0) - (timestamps[0] ?? 0);
  return {
    ...summary,
    framesOver20ms: intervals.filter((interval) => interval > 20).length,
    observedFps: Number((elapsed > 0 ? (intervals.length * 1_000) / elapsed : 0).toFixed(2)),
  };
}

function nextFrame() {
  return new Promise<number>((resolve) => requestAnimationFrame(resolve));
}

async function afterPaint() {
  await nextFrame();
  await nextFrame();
}

async function waitFor(predicate: () => boolean, description: string, timeoutMs = 5_000) {
  const startedAt = performance.now();
  while (!predicate()) {
    if (performance.now() - startedAt > timeoutMs) {
      throw new Error(`Timed out waiting for ${description}`);
    }
    await nextFrame();
  }
}

function click(selector: string) {
  const target = document.querySelector<HTMLElement>(selector);
  if (!target) throw new Error(`Missing benchmark target: ${selector}`);
  target.click();
}

function visibleText(text: string) {
  return document.querySelector(".conversation-stage")?.textContent?.includes(text) === true;
}

function PerformanceReview() {
  const [selectedChatId, setSelectedChatId] = useState<string | null>("performance-chat-0");
  const [selectedProjectId, setSelectedProjectId] = useState<number | null>(null);
  const [status, setStatus] = useState("Preparing production performance review");
  const currentScenario = useRef<Scenario | undefined>(undefined);
  const commitDurations = useRef<Record<Scenario, number[]>>({
    chatSwitch: [],
    composerInput: [],
    navigation: [],
    scrolling: [],
    streaming: [],
  });
  const eventReceivers = useRef(new Map<string, (event: ConversationEvent) => void>());
  const drafts = useMemo(() => {
    const controller = new ProjectDraftController(() => Promise.resolve(undefined));
    controller.reconcile(snapshot);
    return controller;
  }, []);
  const activities = useMemo(() => {
    const controller = new ChatActivityController();
    controller.reconcile(snapshot.chats.map((chat) => chat.id));
    return controller;
  }, []);
  const setups = useMemo(() => {
    const controller = new ChatSetupController();
    controller.reconcile(snapshot);
    return controller;
  }, []);
  const adapter = useMemo<ConversationAdapter>(
    () => ({
      answerInput: () => Promise.resolve(undefined),
      connect(chatId, onEvent) {
        eventReceivers.current.set(chatId, onEvent);
        return Promise.resolve({
          disconnect: () => {
            eventReceivers.current.delete(chatId);
          },
          snapshot: {
            failure: null,
            inputRequest: null,
            items: conversationItems(chatId),
            phase: "running",
          },
        });
      },
      prompt: () => Promise.resolve(undefined),
      stop: () => Promise.resolve(undefined),
    }),
    [],
  );

  const profile = useCallback(
    (_id: string, _phase: "mount" | "update" | "nested-update", actualDuration: number) => {
      const scenario = currentScenario.current;
      if (scenario) commitDurations.current[scenario].push(actualDuration);
    },
    [],
  );

  const run = useCallback(async () => {
    try {
      await document.fonts.ready;
      await new Promise((resolve) => setTimeout(resolve, 1_500));
      await waitFor(
        () => visibleText("Conversation marker performance-chat-0"),
        "the initial transcript",
      );

      click('[data-chat-id="performance-chat-1"] button');
      await waitFor(
        () => visibleText("Conversation marker performance-chat-1"),
        "the warm chat switch",
      );
      await afterPaint();

      setStatus("Measuring locally available chat switching");
      const chatSwitchSamples: number[] = [];
      for (let index = 0; index < CHAT_SWITCH_SAMPLES; index += 1) {
        const chatId = `performance-chat-${index % 2}`;
        currentScenario.current = "chatSwitch";
        const startedAt = performance.now();
        click(`[data-chat-id="${chatId}"] button`);
        await waitFor(() => visibleText(`Conversation marker ${chatId}`), `chat switch ${index}`);
        await nextFrame();
        chatSwitchSamples.push(performance.now() - startedAt);
        currentScenario.current = undefined;
      }

      setStatus("Measuring project/composer navigation");
      const navigationSamples: number[] = [];
      for (let index = 0; index < NAVIGATION_SAMPLES; index += 1) {
        currentScenario.current = "navigation";
        const startedAt = performance.now();
        click(".project-row-select");
        await waitFor(
          () => Boolean(document.querySelector('textarea[aria-label="Draft for Atlas"]')),
          `composer navigation ${index}`,
        );
        await nextFrame();
        navigationSamples.push(performance.now() - startedAt);
        currentScenario.current = undefined;
        click('[data-chat-id="performance-chat-0"] button');
        await waitFor(
          () => visibleText("Conversation marker performance-chat-0"),
          `chat restore ${index}`,
        );
      }

      click(".project-row-select");
      await waitFor(
        () => Boolean(document.querySelector('textarea[aria-label="Draft for Atlas"]')),
        "the composer",
      );
      setStatus("Measuring composer input");
      const inputSamples: number[] = [];
      const textarea = document.querySelector<HTMLTextAreaElement>(
        'textarea[aria-label="Draft for Atlas"]',
      );
      if (!textarea) throw new Error("Composer textarea is unavailable");
      const setTextareaValue = Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value",
      )?.set?.bind(textarea);
      if (!setTextareaValue) throw new Error("Native textarea value setter is unavailable");
      let value = "";
      for (let index = 0; index < INPUT_SAMPLES; index += 1) {
        value += String.fromCharCode(97 + (index % 26));
        currentScenario.current = "composerInput";
        const startedAt = performance.now();
        setTextareaValue(value);
        textarea.dispatchEvent(
          new InputEvent("input", { bubbles: true, data: value.at(-1), inputType: "insertText" }),
        );
        await nextFrame();
        inputSamples.push(performance.now() - startedAt);
        currentScenario.current = undefined;
      }

      click('[data-chat-id="performance-chat-0"] button');
      await waitFor(
        () => visibleText("Conversation marker performance-chat-0"),
        "the transcript for scrolling",
      );
      await afterPaint();
      const viewport = document.querySelector<HTMLElement>(".conversation-transcript-scroll");
      if (!viewport) throw new Error("Transcript viewport is unavailable");

      setStatus("Measuring transcript scrolling");
      const scrollFrames: number[] = [];
      currentScenario.current = "scrolling";
      for (let index = 0; index < FRAME_SAMPLES; index += 1) {
        const timestamp = await nextFrame();
        scrollFrames.push(timestamp);
        const progress = index / Math.max(1, FRAME_SAMPLES - 1);
        viewport.scrollTop = (viewport.scrollHeight - viewport.clientHeight) * progress;
      }
      currentScenario.current = undefined;

      setStatus("Measuring simulated Pi streaming");
      const receive = eventReceivers.current.get("performance-chat-0");
      if (!receive) throw new Error("Conversation event receiver is unavailable");
      const streamFrames: number[] = [];
      currentScenario.current = "streaming";
      for (let index = 0; index < FRAME_SAMPLES; index += 1) {
        const timestamp = await nextFrame();
        streamFrames.push(timestamp);
        receive({ delta: " token", itemId: "stream-performance-chat-0", type: "text-delta" });
      }
      currentScenario.current = undefined;
      await afterPaint();

      const report = {
        browser: navigator.userAgent,
        chatSwitchMs: summarize(chatSwitchSamples),
        composerInputNextFrameMs: summarize(inputSamples),
        method: "packaged WKWebView, production bundle with react-dom/profiling",
        navigationNextFrameMs: summarize(navigationSamples),
        reactCommitMs: Object.fromEntries(
          Object.entries(commitDurations.current).map(([scenario, durations]) => [
            scenario,
            summarize(durations),
          ]),
        ),
        scrollingFrames: summarizeFrames(scrollFrames),
        streamingFrames: summarizeFrames(streamFrames),
        viewport: { height: window.innerHeight, width: window.innerWidth },
      };
      await writeText(`${RESULT_PREFIX}${JSON.stringify(report)}`);
      setStatus("Performance review complete");
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      await writeText(`${RESULT_PREFIX}${JSON.stringify({ error: message })}`);
      setStatus(`Performance review failed: ${message}`);
    }
  }, []);

  return (
    <TooltipProvider>
      <Profiler id="issue-5-production-ui" onRender={profile}>
        <InboxWorkspace
          activities={activities}
          actionError={undefined}
          conversationAdapter={adapter}
          conversationRevision={0}
          drafts={drafts}
          onCancelSetup={() => Promise.resolve(undefined)}
          onCreateChat={() => Promise.resolve(undefined)}
          onOpenRepository={() => undefined}
          onOpenSettings={() => undefined}
          onOpenTerminal={() => Promise.resolve(undefined)}
          onQueryChange={() => undefined}
          onRemoveProject={() => Promise.resolve(undefined)}
          onRenameChat={() => Promise.resolve(undefined)}
          onRequestCodexSignIn={() => undefined}
          onRetrySetup={() => Promise.resolve(undefined)}
          onSelectChat={setSelectedChatId}
          onSelectProject={(projectId) => {
            setSelectedProjectId(projectId);
            setSelectedChatId(null);
          }}
          query=""
          selectedChatId={selectedChatId}
          selectedProjectId={selectedProjectId}
          setups={setups}
          snapshot={snapshot}
        />
      </Profiler>
      <output className="performance-review-status">{status}</output>
      <button className="sr-only" onClick={() => void run()} type="button">
        Run performance review
      </button>
      <RunOnce run={run} />
    </TooltipProvider>
  );
}

function RunOnce({ run }: { run: () => Promise<void> }) {
  useEffect(() => {
    void run();
  }, [run]);
  return null;
}

const root = document.getElementById("root");
if (!root) throw new Error("Più performance root is missing");
createRoot(root).render(<PerformanceReview />);
