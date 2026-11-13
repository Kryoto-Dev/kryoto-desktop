import type { Pose } from './engine'

/**
 * The engine's `download` animation draws the download as arrived above 97%:
 * arms straight out, a box in hand, nothing moving. That is a moment, not a
 * phase, and on a 100 GB game the checksum and the unpacking sit up there for
 * many minutes, so Telly stood frozen in that pose (KB-0307). While the work
 * is still going the body gets at most 97%, so it keeps its working loop, and
 * the screen still shows the real number. The cheer when it is installed is
 * `success`, played by the page.
 */
export const DONE_AT = 0.97

export function workingPose(p: number): { progress: number; pose: Pose } {
  const v = Math.max(0, Math.min(1, Number.isFinite(p) ? p : 0))
  return { progress: Math.min(v, DONE_AT), pose: { screen: { mode: 'bar', v, icon: 'down' } } }
}
