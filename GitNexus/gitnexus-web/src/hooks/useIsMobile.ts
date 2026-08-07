import { useEffect, useState } from 'react';

const DEFAULT_BREAKPOINT = 768;

/**
 * Responsive breakpoint hook — returns true when the viewport is narrower than
 * `breakpoint` (default 768px). Uses matchMedia so it reacts to live resizes
 * and is SSR-safe.
 */
export function useIsMobile(breakpoint: number = DEFAULT_BREAKPOINT): boolean {
  const query = `(max-width: ${breakpoint - 1}px)`;
  const [isMobile, setIsMobile] = useState<boolean>(() => {
    if (typeof window === 'undefined') return false;
    return window.matchMedia(query).matches;
  });

  useEffect(() => {
    const mql = window.matchMedia(query);
    const handler = (event: MediaQueryListEvent) => setIsMobile(event.matches);
    setIsMobile(mql.matches);
    mql.addEventListener('change', handler);
    return () => mql.removeEventListener('change', handler);
  }, [query]);

  return isMobile;
}
