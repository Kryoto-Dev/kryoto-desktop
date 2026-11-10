import { useEffect, useState } from 'react'
import { call, isTauri, on } from '@/lib/bridge'
import { textSizeKey, WheelSteps, type TextSizeStep } from '@/lib/text-size-keys'

/**
 * Text size: everything Kryoto draws, larger or smaller (src-tauri
 * text_size.rs zooms every view). Settings > General picks one; Ctrl + and
 * Ctrl - step through them anywhere, Ctrl 0 goes back to 100%, and so does
 * Ctrl with the mouse wheel.
 */

export const TEXT_SIZES = [90, 100, 110, 125, 150] as const

export const IS_MAC = typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform)
/** The key the shortcuts are held with, as the keyboard names it. */
export const ZOOM_MODIFIER = IS_MAC ? 'Cmd' : 'Ctrl'

export const textSizeApi = {
  get: () => call<number>('text_size_get'),
  set: (percent: number) => call<number>('text_size_set', { percent }),
  step: (step: TextSizeStep) => call<number>('text_size_step', { step }),
}

/** The size changed, from Settings or a key in any view. */
export function onTextSize(fn: (percent: number) => void) {
  return on<number>('text-size', fn)
}

/** The size in use, kept up to date; null until it is known. */
export function useTextSize(): [number | null, (percent: number) => void] {
  const [size, setSize] = useState<number | null>(null)
  useEffect(() => {
    let live = true
    void textSizeApi.get().then((p) => live && setSize(p)).catch(() => live && setSize(100))
    const off = onTextSize((p) => live && setSize(p))
    return () => {
      live = false
      void off.then((f) => f())
    }
  }, [])
  const pick = (percent: number) => {
    setSize(percent)
    void textSizeApi.set(percent).then(setSize).catch(() => void textSizeApi.get().then(setSize).catch(() => {}))
  }
  return [size, pick]
}

/**
 * The keys and Ctrl+wheel, in the shell and the menu view. After the page's
 * own handlers (bubbling, not capture), so a field that uses one of these keys
 * for itself and says so (`preventDefault`) keeps it.
 */
export function installTextSizeKeys() {
  if (!isTauri()) return
  let busy = false
  const step = (s: TextSizeStep) => {
    // One at a time: a held key repeats faster than a save.
    if (busy) return
    busy = true
    void textSizeApi.step(s).catch(() => {}).finally(() => (busy = false))
  }
  window.addEventListener('keydown', (e) => {
    const s = textSizeKey(e, IS_MAC)
    if (s === null) return
    e.preventDefault()
    step(s)
  })
  const wheel = new WheelSteps()
  window.addEventListener(
    'wheel',
    (e) => {
      if (!(IS_MAC ? e.metaKey : e.ctrlKey) || e.altKey) return
      // Never the page's own pinch zoom.
      e.preventDefault()
      const s = wheel.push(e.deltaY, e.deltaMode, e.timeStamp)
      if (s !== null) step(s)
    },
    { passive: false },
  )
}
