/**
 * The emoji this browser (or app) picks most, for the "Frequently used" row at
 * the top of every emoji picker (components/rich-editor.tsx, Desktop and Mobile
 * src/rich/RichEditor.tsx) and the order of our own emoji in the `:` list.
 *
 * Kept on the device, and for a signed-in person also with their account
 * (`syncEmojiUsage`), so the row follows them between the site and the apps.
 * Nobody else ever sees it. Each pick counts once; older picks fade so a new
 * favourite rises within a few days of use. Storage that is blocked or full
 * simply means no row.
 *
 * Pure apart from localStorage, and copied into Kryoto Desktop and Kryoto
 * Mobile (`pnpm sync:rich-text`).
 */
import { isEmojiName } from './rich-catalog'

const KEY = 'kryo.emoji.used'
/** How many emoji the row and the tally keep. */
export const FREQUENT_SHOWN = 16
const KEPT = 40
/** Every pick shrinks the others a little, so the row follows what you use now. */
const FADE = 0.97

/**
 * A tally as it may be kept: only our emoji, sensible counts, the most used
 * KEPT of them. The server keeps the account's copy with this too.
 */
export function cleanTally(raw: unknown): Record<string, number> {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {}
  const entries = Object.entries(raw as Record<string, unknown>)
    .filter((e): e is [string, number] => typeof e[1] === 'number' && Number.isFinite(e[1]) && e[1] > 0 && isEmojiName(e[0]))
    .map(([name, n]): [string, number] => [name, Math.min(Math.round(n * 1000) / 1000, 100_000)])
    .sort((a, b) => b[1] - a[1])
    .slice(0, KEPT)
  return Object.fromEntries(entries)
}

/** Two tallies as one: each emoji at the larger of its two counts. */
export function mergeTallies(a: Record<string, number>, b: Record<string, number>): Record<string, number> {
  const out = { ...a }
  for (const [name, n] of Object.entries(b)) out[name] = Math.max(out[name] ?? 0, n)
  return cleanTally(out)
}

function read(): Record<string, number> {
  try {
    return cleanTally(JSON.parse(localStorage.getItem(KEY) ?? '{}'))
  } catch {
    return {}
  }
}

function write(tally: Record<string, number>): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(cleanTally(tally)))
  } catch {
    // Blocked or full storage: no row, nothing breaks.
  }
}

/** How each app reads and saves the account's copy (`/api/account/emoji-usage`), with its own way of being signed in. */
export type EmojiUsageSync = {
  load: () => Promise<Record<string, number> | null>
  save: (tally: Record<string, number>) => Promise<unknown>
}

let sync: EmojiUsageSync | null = null
let saveTimer: ReturnType<typeof setTimeout> | null = null

/**
 * Follow the signed-in account: its tally is merged into this device's now,
 * and picks are saved back a few seconds after the last one. Null (signed
 * out) stops saving. Failures are quiet: the row still works on the device.
 */
export function syncEmojiUsage(io: EmojiUsageSync | null): void {
  sync = io
  if (!io) return
  void io
    .load()
    .then((remote) => {
      if (!remote || sync !== io) return
      const local = read()
      const merged = mergeTallies(local, cleanTally(remote))
      write(merged)
      // This device knew something the account did not: send it up.
      if (Object.entries(merged).some(([k, n]) => (remote[k] ?? 0) < n)) scheduleSave()
    })
    .catch(() => {})
}

function scheduleSave(): void {
  if (!sync) return
  const io = sync
  if (saveTimer) clearTimeout(saveTimer)
  saveTimer = setTimeout(() => {
    saveTimer = null
    if (sync === io) void io.save(read()).catch(() => {})
  }, 4000)
}

/** Our emoji names, most used first. */
export function frequentEmoji(limit = FREQUENT_SHOWN): string[] {
  return Object.entries(read())
    .sort((a, b) => b[1] - a[1])
    .slice(0, limit)
    .map(([name]) => name)
}

/** Count one pick of `:name:` (anything that is not one of our emoji is ignored). */
export function noteEmojiUse(name: string): void {
  if (!isEmojiName(name)) return
  const tally = read()
  for (const k of Object.keys(tally)) tally[k] = tally[k]! * FADE
  tally[name] = (tally[name] ?? 0) + 1
  write(tally)
  scheduleSave()
}
