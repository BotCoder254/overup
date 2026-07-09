export const SLUG_MAX = 50;

/**
 * Client-side slug preview ONLY — pure user feedback while typing. The
 * backend independently generates the canonical slug (with reserved-word
 * and uniqueness handling) and never accepts one from the browser.
 */
export function slugPreview(name: string): string {
  return name
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, SLUG_MAX)
    .replace(/-+$/g, '');
}
