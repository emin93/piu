import { useCallback, useEffect, useRef } from "react";

const DEFAULT_WIDTH = 256;
const MIN_WIDTH = 208;
const MAX_WIDTH = 360;
const KEYBOARD_STEP = 16;

function clampWidth(width: number) {
  const quantizedWidth = Math.round(width / 4) * 4;
  return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, quantizedWidth));
}

export function SidebarResizeHandle({ disabled = false }: { disabled?: boolean }) {
  const handleRef = useRef<HTMLDivElement>(null);
  const widthRef = useRef(DEFAULT_WIDTH);
  const dragRef = useRef<{ pointerId: number; startX: number; startWidth: number } | undefined>(
    undefined,
  );

  const applyWidth = useCallback((width: number) => {
    const nextWidth = clampWidth(width);
    widthRef.current = nextWidth;
    document.documentElement.style.setProperty("--inbox-sidebar-width", `${nextWidth}px`);
    handleRef.current?.setAttribute("aria-valuenow", String(nextWidth));
  }, []);

  useEffect(() => {
    applyWidth(DEFAULT_WIDTH);
    return () => {
      document.documentElement.style.removeProperty("--inbox-sidebar-width");
    };
  }, [applyWidth]);

  return (
    <div
      aria-label="Resize inbox"
      aria-disabled={disabled || undefined}
      aria-orientation="vertical"
      aria-valuemax={MAX_WIDTH}
      aria-valuemin={MIN_WIDTH}
      aria-valuenow={DEFAULT_WIDTH}
      className="sidebar-resize-handle"
      onKeyDown={(event) => {
        if (disabled) return;
        if (event.key === "ArrowLeft") applyWidth(widthRef.current - KEYBOARD_STEP);
        else if (event.key === "ArrowRight") applyWidth(widthRef.current + KEYBOARD_STEP);
        else if (event.key === "Home") applyWidth(MIN_WIDTH);
        else if (event.key === "End") applyWidth(MAX_WIDTH);
        else return;
        event.preventDefault();
      }}
      onPointerDown={(event) => {
        if (disabled) return;
        dragRef.current = {
          pointerId: event.pointerId,
          startX: event.clientX,
          startWidth: widthRef.current,
        };
        event.currentTarget.setPointerCapture?.(event.pointerId);
      }}
      onPointerMove={(event) => {
        const drag = dragRef.current;
        if (!drag || drag.pointerId !== event.pointerId) return;
        applyWidth(drag.startWidth + event.clientX - drag.startX);
      }}
      onPointerUp={(event) => {
        if (dragRef.current?.pointerId !== event.pointerId) return;
        dragRef.current = undefined;
        event.currentTarget.releasePointerCapture?.(event.pointerId);
      }}
      ref={handleRef}
      role="separator"
      tabIndex={disabled ? -1 : 0}
    />
  );
}
