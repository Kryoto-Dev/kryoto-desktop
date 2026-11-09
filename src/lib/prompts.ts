import { useEffect, useState, useSyncExternalStore } from 'react'
import { dialogCount, watchDialogs } from '@/ui/dialog-focus'

/**
 * What opens by itself once the client is up, most pressing first. One at a
 * time, and never over a dialog the player opened: each waits for its turn
 * here. A question keeps its turn until it is answered; the notice is a card
 * in the corner, so it steps aside for anything else and comes back after.
 */
const ORDER = ['update', 'setup', 'intro', 'found', 'collection', 'notice', 'writeup'] as const
type StartPrompt = (typeof ORDER)[number]
/** Cards in the corner rather than questions: they step aside for anything else and come back after. */
const CARDS: ReadonlySet<StartPrompt> = new Set(['notice', 'writeup'])

const wanting = new Set<StartPrompt>()
const listeners = new Set<() => void>()
let turn: StartPrompt | null = null
let held = false

function settle() {
  const keeps = !held && turn !== null && !CARDS.has(turn) && wanting.has(turn)
  const next = keeps ? turn : held || dialogCount() ? null : (ORDER.find((p) => wanting.has(p)) ?? null)
  if (next === turn) return
  turn = next
  listeners.forEach((listener) => listener())
}

watchDialogs(settle)

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

/** Nothing opens by itself while held: Big Picture covers the whole window. */
export function holdPrompts(on: boolean) {
  held = on
  settle()
}

/** Whether this prompt, with something to show, may show it now. */
export function usePromptTurn(prompt: StartPrompt, wanted: boolean): boolean {
  useEffect(() => {
    if (!wanted) return
    wanting.add(prompt)
    settle()
    return () => {
      wanting.delete(prompt)
      settle()
    }
  }, [prompt, wanted])
  return useSyncExternalStore(subscribe, () => turn) === prompt && wanted
}

/** How many dialogs are open, as it changes. */
export const useDialogCount = () => useSyncExternalStore(watchDialogs, dialogCount)

/** True once no dialog has been open for `ms`, and from then on. */
export function useQuiet(ms: number, active = true): boolean {
  const dialogs = useDialogCount()
  const [quiet, setQuiet] = useState(false)
  useEffect(() => {
    if (!active || quiet || dialogs) return
    const t = window.setTimeout(() => setQuiet(true), ms)
    return () => window.clearTimeout(t)
  }, [active, quiet, dialogs, ms])
  return quiet
}
