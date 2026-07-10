import { useEffect, useState } from 'react';

/**
 * Reactive `window.matchMedia` subscription. Returns false on the first
 * server-side / pre-mount evaluation, then tracks the media query live.
 */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState<boolean>(() => {
    if (typeof window === 'undefined' || !window.matchMedia) return false;
    return window.matchMedia(query).matches;
  });

  useEffect(() => {
    if (typeof window === 'undefined' || !window.matchMedia) return;
    const list = window.matchMedia(query);
    const onChange = (event: MediaQueryListEvent) => setMatches(event.matches);
    setMatches(list.matches);
    list.addEventListener('change', onChange);
    return () => list.removeEventListener('change', onChange);
  }, [query]);

  return matches;
}

/** Matches Tailwind's `lg` breakpoint — the sidebar/desktop layout switch. */
export function useIsDesktop(): boolean {
  return useMediaQuery('(min-width: 1024px)');
}
