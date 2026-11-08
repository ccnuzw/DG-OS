import { useEffect, useRef, useState, type RefObject } from 'react';

/** Presentation only: measure a stable container, never persist its automatic layout. */
export function useLayoutBudget<T extends HTMLElement>(minimum: (root: HTMLElement) => number): { root: RefObject<T | null>; narrow: boolean } {
  const root = useRef<T>(null);
  const budget = useRef(minimum);
  budget.current = minimum;
  const [narrow, setNarrow] = useState(false);
  const measuredNarrow = useRef(false);
  useEffect(() => {
    const element = root.current;
    if (!element) return;
    let frame: number | null = null;
    let disposed = false;
    const measure = () => {
      frame = null;
      if (disposed || element.clientWidth <= 0) return;
      const next = element.clientWidth < budget.current(element);
      if (measuredNarrow.current !== next) {
        measuredNarrow.current = next;
        setNarrow(next);
      }
    };
    const schedule = () => {
      if (!disposed && frame === null) frame = requestAnimationFrame(measure);
    };
    const resize = new ResizeObserver(schedule);
    resize.observe(element);
    // Density and UI typography can change the budget without resizing this container.
    const appearance = new MutationObserver(schedule);
    for (let ancestor: HTMLElement | null = element; ancestor; ancestor = ancestor.parentElement) {
      appearance.observe(ancestor, { attributes: true, attributeFilter: ['style', 'data-density', 'class'] });
    }
    const fonts = document.fonts;
    fonts?.addEventListener('loadingdone', schedule);
    fonts?.addEventListener('loadingerror', schedule);
    void fonts?.ready?.then(schedule);
    window.addEventListener('resize', schedule);
    schedule();
    return () => {
      disposed = true;
      if (frame !== null) cancelAnimationFrame(frame);
      resize.disconnect();
      appearance.disconnect();
      fonts?.removeEventListener('loadingdone', schedule);
      fonts?.removeEventListener('loadingerror', schedule);
      window.removeEventListener('resize', schedule);
    };
  }, []);
  return { root, narrow };
}
