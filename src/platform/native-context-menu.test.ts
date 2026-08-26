import { beforeEach, expect, test, vi } from "vitest";

interface TestMenuItem {
  action?: (id: string) => void;
  id?: string;
  item?: string;
  text?: string;
}

interface TestMenu {
  close: () => Promise<void>;
  popup: (position?: unknown, window?: unknown) => Promise<void>;
}

const tauri = vi.hoisted(() => ({
  close: vi.fn().mockResolvedValue(undefined),
  getCurrentWindow: vi.fn(() => ({ label: "main" })),
  popup: vi.fn().mockResolvedValue(undefined),
}));

const menuNew = vi.hoisted(() =>
  vi.fn<(options: { items: TestMenuItem[] }) => Promise<TestMenu>>(),
);

vi.mock("@tauri-apps/api/menu", () => ({
  Menu: { new: menuNew },
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: tauri.getCurrentWindow,
}));

import { popupNativeContextMenu } from "./native-context-menu";

beforeEach(() => {
  vi.clearAllMocks();
  menuNew.mockResolvedValue({ close: tauri.close, popup: tauri.popup });
});

test("builds a native menu with a separator before the destructive action", async () => {
  const onAction = vi.fn();
  await popupNativeContextMenu({
    actions: [
      { id: "rename", label: "Rename chat" },
      { id: "delete", label: "Delete chat", separatorBefore: true },
    ],
    onAction,
    position: { x: 24, y: 48 },
  });

  expect(menuNew).toHaveBeenCalledOnce();
  const items = menuNew.mock.calls[0]?.[0].items ?? [];
  expect(items.map((item) => item.id ?? item.item)).toEqual(["rename", "Separator", "delete"]);
  expect(items[0].text).toBe("Rename chat");
  expect(items[2].text).toBe("Delete chat");
  items[2].action?.("delete");
  expect(onAction).toHaveBeenCalledWith("delete");

  expect(tauri.popup).toHaveBeenCalledWith(
    expect.objectContaining({ type: "Logical", x: 24, y: 48 }),
    { label: "main" },
  );
  expect(tauri.close).toHaveBeenCalledOnce();
});

test("uses the current pointer when no logical position is supplied and always closes", async () => {
  tauri.popup.mockRejectedValueOnce(new Error("popup failed"));

  await expect(
    popupNativeContextMenu({
      actions: [{ id: "rename", label: "Rename chat" }],
      onAction: vi.fn(),
    }),
  ).rejects.toThrow("popup failed");

  expect(tauri.popup).toHaveBeenCalledWith(undefined, { label: "main" });
  expect(tauri.close).toHaveBeenCalledOnce();
});
