/**
 * Motion reduced: the system asks for it, or Kryoto's own Reduce motion
 * setting is on (lib/settings.ts puts `data-motion="reduced"` on the page).
 * One copy, so Big Picture's opening, the start screen's scene and the
 * screensavers all go by the same two.
 *
 * Imports nothing and touches the page only when asked, so the screensaver
 * engine can use it and still run under `pnpm test`.
 */
export function reducedMotion(): boolean {
  if (typeof window === 'undefined') return false
  return !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches || document.documentElement.dataset.motion === 'reduced'
}
