import { LogicalPosition } from "@tauri-apps/api/dpi";
import { Menu, type MenuItemOptions, type PredefinedMenuItemOptions } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";

export interface NativeContextMenuAction<ActionId extends string> {
  id: ActionId;
  label: string;
  separatorBefore?: boolean;
}

export interface NativeContextMenuPosition {
  x: number;
  y: number;
}

export async function popupNativeContextMenu<ActionId extends string>({
  actions,
  onAction,
  position,
}: {
  actions: readonly NativeContextMenuAction<ActionId>[];
  onAction: (action: ActionId) => void;
  position?: NativeContextMenuPosition;
}) {
  const items = actions.flatMap<MenuItemOptions | PredefinedMenuItemOptions>((action) => [
    ...(action.separatorBefore ? [{ item: "Separator" as const }] : []),
    {
      action: (id) => onAction(id as ActionId),
      id: action.id,
      text: action.label,
    },
  ]);
  const menu = await Menu.new({ items });

  try {
    await menu.popup(
      position ? new LogicalPosition(position.x, position.y) : undefined,
      getCurrentWindow(),
    );
  } finally {
    await menu.close();
  }
}
