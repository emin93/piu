import type { Event } from "@tauri-apps/api/event";
import { beforeEach, expect, test, vi } from "vitest";

import type { HostRoundTripRequest } from "../generated/HostRoundTripRequest";
import type { HostRoundTripResponse } from "../generated/HostRoundTripResponse";
import { verifyHostBoundary } from "./host-boundary";

const boundary = vi.hoisted(() => ({
  handler: undefined as ((event: Event<HostRoundTripResponse>) => void) | undefined,
  order: [] as string[],
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    (
      _eventName: string,
      handler: (event: Event<HostRoundTripResponse>) => void,
    ): Promise<() => void> => {
      boundary.order.push("listen");
      boundary.handler = handler;
      return Promise.resolve(() => boundary.order.push("unlisten"));
    },
  ),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(
    (
      _command: string,
      { request }: { request: HostRoundTripRequest },
    ): Promise<HostRoundTripResponse> => {
      boundary.order.push("invoke");
      const response: HostRoundTripResponse = {
        ...request,
        receivedAtMs: request.sentAtMs + 3,
        schemaVersion: 1,
      };
      boundary.handler?.({ payload: response } as Event<HostRoundTripResponse>);
      return Promise.resolve(response);
    },
  ),
}));

beforeEach(() => {
  boundary.handler = undefined;
  boundary.order.length = 0;
});

test("the generated client observes a matching production command and event", async () => {
  const result = await verifyHostBoundary();

  expect(result.schemaVersion).toBe(1);
  expect(result.latencyMs).toBeGreaterThanOrEqual(0);
  expect(boundary.order).toEqual(["listen", "invoke", "unlisten"]);
});
