/**
 * Text effects: a perk that dresses your name, or a few words of anything you
 * write, in an effect the whole site draws (components/text-fx.tsx, the
 * `.kryo-fx-*` styles in app/text-effects.css).
 *
 * In text, an effect wraps the words it applies to:
 *
 *     gg {rainbow}well played{/rainbow}, see you {glitch}tomorrow{/glitch}
 *
 * Who may use which effect is decided when the text is SAVED, never when it is
 * shown: `stripFx` removes the tags for effects the author does not have and
 * keeps the words, so a comment written while a perk was on keeps its effect
 * after the perk ends, and nobody can paste in an effect they never had.
 *
 *   * The six standard effects come with the `text_effects` perk (Backer and
 *     up, or bought with Kryos).
 *   * The exclusive ones (fire, aurora, prism, hologram, eclipse) are their own
 *     Kryos purchase (`exclusive_effects` in lib/reputation-shared.ts).
 *
 * Shared by the server and the pages, so it imports nothing server-only.
 */

export const STANDARD_TEXT_EFFECTS = ['rainbow', 'neon', 'gold', 'frost', 'glitch', 'wave'] as const
export const EXCLUSIVE_TEXT_EFFECTS = ['fire', 'aurora', 'prism', 'hologram', 'eclipse'] as const
export const TEXT_EFFECTS = [...STANDARD_TEXT_EFFECTS, ...EXCLUSIVE_TEXT_EFFECTS] as const
export type TextEffectId = (typeof TEXT_EFFECTS)[number]

export function isTextEffectId(v: unknown): v is TextEffectId {
  return typeof v === 'string' && (TEXT_EFFECTS as readonly string[]).includes(v)
}

export const TEXT_EFFECT_LABEL: Record<TextEffectId, string> = {
  rainbow: 'Rainbow',
  neon: 'Neon',
  gold: 'Gold',
  frost: 'Frost',
  glitch: 'Glitch',
  wave: 'Wave',
  fire: 'Fire',
  aurora: 'Aurora',
  prism: 'Prism',
  hologram: 'Hologram',
  eclipse: 'Eclipse',
}

/** At most this many effect spans in one text. Past it, the extra tags are dropped. */
export const MAX_FX_SPANS = 8

const FX_RE = new RegExp(`\\{(${TEXT_EFFECTS.join('|')})\\}([\\s\\S]*?)\\{\\/\\1\\}`, 'g')

export type FxSegment = { text: string; fx: TextEffectId | null }

/** Text as plain and effect segments, in order. Unclosed or unknown tags stay plain text. */
export function parseFx(input: string): FxSegment[] {
  const out: FxSegment[] = []
  let last = 0
  let spans = 0
  for (const m of input.matchAll(FX_RE)) {
    const at = m.index ?? 0
    if (spans >= MAX_FX_SPANS) break
    if (at > last) out.push({ text: input.slice(last, at), fx: null })
    const inner = m[2] ?? ''
    if (inner) out.push({ text: inner, fx: m[1] as TextEffectId })
    last = at + m[0].length
    spans++
  }
  if (last < input.length) out.push({ text: input.slice(last), fx: null })
  return out.length ? out : [{ text: '', fx: null }]
}

/** Does this text carry any effect tag at all? Cheap check before parsing. */
export function hasFx(input: string): boolean {
  FX_RE.lastIndex = 0
  const found = FX_RE.test(input)
  FX_RE.lastIndex = 0
  return found
}

/** Any opening or closing effect tag, for spotting one inside another. */
const TAG_RE = new RegExp(`\\{\\/?(${TEXT_EFFECTS.join('|')})\\}`)

/**
 * The text with every effect the author may not use unwrapped to its words,
 * and anything past MAX_FX_SPANS unwrapped too. What is stored.
 *
 * Repeated until nothing changes: unwrapping `{fire}{fire}hi{/fire}{/fire}`
 * once leaves `{fire}hi{/fire}`, a fresh tag the first pass never judged. A
 * span with another tag inside it is unwrapped as well, so nothing nests.
 */
export function stripFx(input: string, allowed: ReadonlySet<TextEffectId>): string {
  let text = input
  for (let pass = 0; pass < 32; pass++) {
    let spans = 0
    const next = text.replace(FX_RE, (whole, id: string, inner: string) => {
      const keep = isTextEffectId(id) && allowed.has(id) && spans < MAX_FX_SPANS && inner && !TAG_RE.test(inner)
      if (keep) {
        spans++
        return whole
      }
      return inner
    })
    if (next === text) return text
    text = next
  }
  return plainFx(text)
}

/** The words without any effect tags, for places that show plain text (titles, notifications, search). */
export function plainFx(input: string): string {
  return input.replace(FX_RE, (_whole, _id: string, inner: string) => inner)
}

/**
 * Every string inside a value, with effect tags taken out (nested ones too).
 * For text leaving the site: Discord shows `{rainbow}gg{/rainbow}` literally,
 * so whatever the bot is handed goes through this (lib/bot-auth.ts botJson).
 */
export function plainFxDeep<T>(value: T): T {
  if (typeof value === 'string') {
    let text: string = value
    for (let pass = 0; pass < 8 && hasFx(text); pass++) text = plainFx(text)
    return text as T
  }
  if (Array.isArray(value)) return value.map((v) => plainFxDeep(v)) as T
  if (value && typeof value === 'object' && Object.getPrototypeOf(value) === Object.prototype) {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plainFxDeep(v)])) as T
  }
  return value
}

/** The effects somebody may use: the standard six with the perk, the exclusive five with their own purchase. */
export function allowedEffects(input: { perk: boolean; exclusive: boolean }): Set<TextEffectId> {
  const set = new Set<TextEffectId>()
  if (input.perk) for (const e of STANDARD_TEXT_EFFECTS) set.add(e)
  if (input.exclusive) for (const e of EXCLUSIVE_TEXT_EFFECTS) set.add(e)
  return set
}
