import { useLayoutEffect, useState, type RefObject } from "react";
import type { LayoutSize } from "./model";

export function measureAgentLayoutSize(
  element: HTMLElement | null,
): LayoutSize {
  if (!element?.isConnected) return { width: 0, height: 0 };
  const bounds = element.getBoundingClientRect();
  const style = getComputedStyle(element);
  const pixels = (value: string) => Number.parseFloat(value) || 0;
  return {
    width: Math.max(
      0,
      bounds.width -
        pixels(style.paddingLeft) -
        pixels(style.paddingRight) -
        pixels(style.borderLeftWidth) -
        pixels(style.borderRightWidth),
    ),
    height: Math.max(
      0,
      bounds.height -
        pixels(style.paddingTop) -
        pixels(style.paddingBottom) -
        pixels(style.borderTopWidth) -
        pixels(style.borderBottomWidth),
    ),
  };
}

export default function useAgentLayoutSize(
  stage: RefObject<HTMLElement | null>,
) {
  const [size, setSize] = useState(() => measureAgentLayoutSize(stage.current));
  useLayoutEffect(() => {
    const element = stage.current;
    let current = true;
    const measure = () => {
      if (!current) return;
      const next = measureAgentLayoutSize(element);
      setSize((previous) =>
        previous.width === next.width && previous.height === next.height
          ? previous
          : next,
      );
    };
    const observer = new ResizeObserver(measure);
    if (element) observer.observe(element);
    window.addEventListener("resize", measure);
    measure();
    return () => {
      current = false;
      observer.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, [stage]);
  return size;
}
