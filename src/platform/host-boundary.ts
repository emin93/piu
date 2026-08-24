import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { HostRoundTripRequest } from "../generated/HostRoundTripRequest";
import type { HostRoundTripResponse } from "../generated/HostRoundTripResponse";

const HOST_ROUND_TRIP_EVENT = "host://round-trip-completed";
const EVENT_TIMEOUT_MS = 2_000;

export interface HostBoundaryVerification {
  correlationId: string;
  latencyMs: number;
  schemaVersion: number;
}

export async function verifyHostBoundary(): Promise<HostBoundaryVerification> {
  const request: HostRoundTripRequest = {
    correlationId: crypto.randomUUID(),
    sentAtMs: Date.now(),
  };
  let resolveEvent: (response: HostRoundTripResponse) => void = () => undefined;
  const matchingEvent = new Promise<HostRoundTripResponse>((resolve) => {
    resolveEvent = resolve;
  });
  const unlisten = await listen<HostRoundTripResponse>(HOST_ROUND_TRIP_EVENT, ({ payload }) => {
    if (payload.correlationId === request.correlationId) {
      resolveEvent(payload);
    }
  });
  let eventTimeout: number | undefined;

  try {
    const response = await invoke<HostRoundTripResponse>("host_round_trip", { request });
    const event = await Promise.race([
      matchingEvent,
      new Promise<never>((_, reject) => {
        eventTimeout = window.setTimeout(
          () => reject(new Error("Più host event timed out")),
          EVENT_TIMEOUT_MS,
        );
      }),
    ]);
    if (
      event.correlationId !== response.correlationId ||
      event.schemaVersion !== response.schemaVersion
    ) {
      throw new Error("Più host command and event did not match");
    }
    return {
      correlationId: response.correlationId,
      latencyMs: Math.max(0, response.receivedAtMs - response.sentAtMs),
      schemaVersion: response.schemaVersion,
    };
  } finally {
    if (eventTimeout !== undefined) window.clearTimeout(eventTimeout);
    unlisten();
  }
}
