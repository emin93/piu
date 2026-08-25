import {
  forwardRef,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
  type ComponentType,
  type CSSProperties,
  type Key,
  type ReactNode,
} from "react";

import { mockVirtuosoAutoscrollToBottom } from "./mock-virtuoso-state";

interface MockTranscriptComponentProps {
  children?: ReactNode;
  context: unknown;
  item?: string;
  style?: CSSProperties;
}

interface MockVirtuosoProps {
  "aria-label"?: string;
  atBottomStateChange?: (atBottom: boolean) => void;
  atBottomThreshold?: number;
  className?: string;
  components?: {
    Footer?: ComponentType<MockTranscriptComponentProps>;
    Header?: ComponentType<MockTranscriptComponentProps>;
    Item?: ComponentType<MockTranscriptComponentProps>;
    List?: ComponentType<MockTranscriptComponentProps>;
  };
  computeItemKey?: (index: number, item: string, context: unknown) => Key;
  context: unknown;
  data: readonly string[];
  followOutput?: boolean | string | ((atBottom: boolean) => boolean | string);
  initialTopMostItemIndex?:
    number | { align?: "center" | "end" | "start"; index: number; offset?: number };
  itemContent: (index: number, item: string, context: unknown) => ReactNode;
  rangeChanged?: (range: { endIndex: number; startIndex: number }) => void;
  restoreStateFrom?: { scrollTop: number };
  role?: string;
  scrollerRef?: (ref: HTMLElement | Window | null) => void;
}

const VISIBLE_ITEM_COUNT = 8;

export const MockVirtuoso = forwardRef<
  { autoscrollToBottom: () => void; getState: (callback: (state: unknown) => void) => void },
  MockVirtuosoProps
>(function MockVirtuoso(
  {
    "aria-label": ariaLabel,
    atBottomStateChange,
    atBottomThreshold = 0,
    className,
    components,
    computeItemKey,
    context,
    data,
    followOutput,
    initialTopMostItemIndex,
    itemContent,
    rangeChanged,
    restoreStateFrom,
    role,
    scrollerRef,
  },
  ref,
) {
  const tailStart = Math.max(0, data.length - VISIBLE_ITEM_COUNT);
  const requestedInitialIndex =
    typeof initialTopMostItemIndex === "number"
      ? initialTopMostItemIndex
      : initialTopMostItemIndex?.align === "start"
        ? initialTopMostItemIndex.index
        : undefined;
  const initialWindowStartRef = useRef(requestedInitialIndex ?? tailStart);
  const scrollerElementRef = useRef<HTMLDivElement>(null);
  const [atBottom, setAtBottom] = useState(true);
  const followsBottom =
    typeof followOutput === "function" ? followOutput(atBottom) : Boolean(followOutput);
  const windowStart = followsBottom
    ? tailStart
    : Math.min(initialWindowStartRef.current, tailStart);

  useImperativeHandle(
    ref,
    () => ({
      autoscrollToBottom() {
        mockVirtuosoAutoscrollToBottom();
        setAtBottom(true);
      },
      getState(callback) {
        callback({ ranges: [], scrollTop: scrollerElementRef.current?.scrollTop ?? 0 });
      },
    }),
    [],
  );
  useLayoutEffect(() => () => scrollerRef?.(null), [scrollerRef]);
  useLayoutEffect(() => {
    rangeChanged?.({
      endIndex: Math.min(data.length - 1, windowStart + VISIBLE_ITEM_COUNT - 1),
      startIndex: windowStart,
    });
  }, [data.length, rangeChanged, windowStart]);

  const Header = components?.Header;
  const Footer = components?.Footer;
  const Item = components?.Item;
  const List = components?.List;
  const itemNodes = data
    .slice(windowStart, windowStart + VISIBLE_ITEM_COUNT)
    .map((item, offset) => {
      const index = windowStart + offset;
      const key = computeItemKey?.(index, item, context) ?? item;
      const content = itemContent(index, item, context);
      return Item ? (
        <Item
          context={context}
          data-index={index}
          data-item-index={index}
          data-known-size={84}
          item={item}
          key={key}
        >
          {content}
        </Item>
      ) : (
        <div key={key}>{content}</div>
      );
    });

  return (
    <div
      aria-label={ariaLabel}
      className={className}
      data-start-offset={
        typeof initialTopMostItemIndex === "object" ? initialTopMostItemIndex.offset : undefined
      }
      data-start-index={requestedInitialIndex}
      data-restored-scroll-top={restoreStateFrom?.scrollTop}
      onScroll={(event) => {
        const scroller = event.currentTarget;
        const nextAtBottom =
          scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= atBottomThreshold;
        setAtBottom(nextAtBottom);
        atBottomStateChange?.(nextAtBottom);
      }}
      ref={(element) => {
        scrollerElementRef.current = element;
        scrollerRef?.(element);
      }}
      role={role}
    >
      {Header ? <Header context={context} /> : null}
      {List ? <List context={context}>{itemNodes}</List> : itemNodes}
      {Footer ? <Footer context={context} /> : null}
    </div>
  );
});
