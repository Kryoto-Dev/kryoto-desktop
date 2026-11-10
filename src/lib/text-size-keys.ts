/**
 * The text size's keys and Ctrl+wheel, as plain functions (no app imports), so
 * `pnpm test` runs them. The Store's pages get the same rules in their own
 * script (src-tauri lib.rs, BROWSER_STATE_SCRIPT); keep the two alike.
 */

export type TextSizeStep = -1 | 0 | 1

type KeyLike = {
  key: string
  code: string
  ctrlKey: boolean
  metaKey: boolean
  altKey: boolean
  shiftKey: boolean
  isComposing?: boolean
  defaultPrevented?: boolean
}

/**
 * What a key press asks of the text size: 1 larger, -1 smaller, 0 back to
 * 100%, null nothing. Ctrl (Cmd on macOS) with `=`/`+`, `-` or `0`, the number
 * pad's too. Never with Alt: AltGr is Ctrl+Alt, and AltGr+0 types `}` on a
 * German keyboard. Never mid-composition (IME), and never a key something on
 * the page already used.
 */
export function textSizeKey(e: KeyLike, mac = false): TextSizeStep | null {
  if (e.defaultPrevented || e.isComposing || e.altKey) return null
  if (mac ? !e.metaKey || e.ctrlKey : !e.ctrlKey || e.metaKey) return null
  if (e.key === '+' || e.key === '=' || e.code === 'NumpadAdd') return 1
  if (e.shiftKey) return null
  if (e.key === '-' || e.code === 'NumpadSubtract') return -1
  // `Digit0` too: on AZERTY the key without Shift is `à`.
  if (e.key === '0' || e.code === 'Numpad0' || e.code === 'Digit0') return 0
  return null
}

/** How far the wheel has to turn for one step, in pixels; a mouse's notch is about 100. */
const STEP_AT = 50
/** Steps never come faster than this, so a trackpad pinch does not race to the end. */
const SPACING_MS = 140

/**
 * Ctrl+wheel: turning up is larger, down smaller, one step per notch. A
 * trackpad's pinch sends many small turns; they add up to steps the same way.
 */
export class WheelSteps {
  private turned = 0
  private last = -Infinity

  /** `deltaY` and `deltaMode` as the wheel event has them, `now` in ms. */
  push(deltaY: number, deltaMode: number, now: number): TextSizeStep | null {
    const px = deltaY * (deltaMode === 1 ? 40 : deltaMode === 2 ? 800 : 1)
    // Turning back starts over.
    if (Math.sign(px) !== Math.sign(this.turned)) this.turned = 0
    this.turned += px
    if (Math.abs(this.turned) < STEP_AT || now - this.last < SPACING_MS) return null
    const step: TextSizeStep = this.turned < 0 ? 1 : -1
    this.turned = 0
    this.last = now
    return step
  }
}
