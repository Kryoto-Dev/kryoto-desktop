/**
 * Kryoto's own animated emoji and stickers: the brand Discord art in
 * `public/brand/discord/` (see its README), free for everyone and served by
 * the website itself, never a filehost.
 *
 * Only the animated set: the still emoji stay on Discord. In text they are
 *
 *     :telly_wave:            an emoji, drawn the height of the line
 *     [sticker:telly-says-hi] a sticker, drawn big on its own
 *
 * and a name that is not listed here stays the text it is.
 *
 * Pure and dependency-free: this file is copied byte for byte into Kryoto
 * Desktop and Kryoto Mobile (`pnpm sync:rich-text`), which prefix the paths
 * with the site's address. Add art by adding its file and a line here.
 */

export type EmojiEntry = {
  /** What goes between the colons. */
  name: string
  /** Words the picker searches, besides the name. */
  label: string
  group: EmojiGroup
  /** A supporter rank's own variant: only that rank and the ones above may use it. */
  rank?: RankId
}

export type EmojiGroup = 'telly' | 'kubi' | 'yuki' | 'blip' | 'kryo' | RankId

export const EMOJI_GROUPS: { id: EmojiGroup; label: string }[] = [
  { id: 'telly', label: 'Telly' },
  { id: 'kubi', label: 'Kubi' },
  { id: 'yuki', label: 'Yuki' },
  { id: 'blip', label: 'Blip' },
  { id: 'kryo', label: 'Kryoto' },
]

// ---- Rank sets -------------------------------------------------------------
//
// Each donation rank has its own colour of every mascot emoji and every
// sticker (scripts/rank-emoji/build.py makes them from the art above). A rank
// may use its own set and every set below it, while its support is current.
// The server takes them out of anything else (`withoutRankArt`, called by
// prepareRichText in lib/mentions-db.ts), turning each back into the
// original, the way it unwraps text effects a writer does not own.

/** Lowest first, the same ladder as lib/donation-tiers.ts. */
export const RANKS = ['supporter', 'backer', 'patron', 'sponsor'] as const
export type RankId = (typeof RANKS)[number]

export const RANK_LABEL: Record<RankId, string> = { supporter: 'Supporter', backer: 'Backer', patron: 'Patron', sponsor: 'Sponsor' }

/** May somebody at `held` (null: no current support) use art made for `needed`? */
export function canUseRank(needed: RankId | null | undefined, held: string | null | undefined): boolean {
  if (!needed) return true
  const have = RANKS.indexOf(held as RankId)
  return have >= 0 && have >= RANKS.indexOf(needed)
}

const RANK_SUFFIX = new RegExp(`^(.+?)[_-](${RANKS.join('|')})$`)

/** The rank a token's name was made for, and the original it was made from. */
export function rankOfName(name: string): { base: string; rank: RankId } | null {
  const m = RANK_SUFFIX.exec(name)
  return m ? { base: m[1]!, rank: m[2] as RankId } : null
}

const MOODS = ['wave', 'happy', 'love', 'sleep', 'dance', 'think', 'cry', 'angry', 'gaming', 'victory'] as const
const MASCOTS = ['telly', 'kubi', 'yuki', 'blip'] as const

/** The matrix's state words, plus the K and the coin. */
const KRYO_EMOJI = [
  'k',
  'working',
  'syncing',
  'downloading',
  'checking',
  'connecting',
  'waiting',
  'queued',
  'processing',
  'done',
  'complete',
  'failed',
  'note',
  'new',
  'update',
] as const

export const EMOJI: EmojiEntry[] = [
  ...MASCOTS.flatMap((m) => MOODS.map((mood): EmojiEntry => ({ name: `${m}_${mood}`, label: `${m} ${mood}`, group: m }))),
  { name: 'kryos', label: 'kryos coin', group: 'kryo' },
  ...KRYO_EMOJI.map((w): EmojiEntry => ({ name: `kryo_${w}`, label: `kryo ${w}`, group: 'kryo' })),
  ...RANKS.flatMap((rank) =>
    MASCOTS.flatMap((m) => MOODS.map((mood): EmojiEntry => ({ name: `${m}_${mood}_${rank}`, label: `${m} ${mood} ${rank}`, group: rank, rank }))),
  ),
]

export const EMOJI_RANK_GROUPS: { id: RankId; label: string }[] = RANKS.map((r) => ({ id: r, label: `${RANK_LABEL[r]} set` }))

const EMOJI_BY_NAME = new Map(EMOJI.map((e) => [e.name, e]))

export function isEmojiName(name: string): boolean {
  return EMOJI_BY_NAME.has(name)
}

export function emojiFor(name: string): EmojiEntry | null {
  return EMOJI_BY_NAME.get(name) ?? null
}

/** Site-relative address of an emoji's animation. */
export function emojiPath(name: string): string {
  return EMOJI_BY_NAME.get(name)?.rank ? `/brand/rank/emoji/${name}_a.gif` : `/brand/discord/emoji/animated/${name}_a.gif`
}

export type StickerEntry = {
  /** What goes after `sticker:`. */
  id: string
  label: string
  file: string
  rank?: RankId
}

const BASE_STICKERS: StickerEntry[] = [
  { id: 'telly-says-hi', label: 'Telly Says Hi', file: '01-telly-says-hi.png' },
  { id: 'telly-all-good', label: 'Telly All Good', file: '02-telly-all-good.png' },
  { id: 'telly-game-on', label: 'Telly Game On', file: '03-telly-game-on.png' },
  { id: 'telly-loading', label: 'Telly Loading', file: '04-telly-loading.png' },
  { id: 'telly-taco-break', label: 'Telly Taco Break', file: '05-telly-taco-break.png' },
  { id: 'telly-k', label: 'Telly K//', file: '06-telly-k.png' },
  { id: 'kubi-meltdown', label: 'Kubi Meltdown', file: '07-kubi-meltdown.png' },
  { id: 'kubi-hype', label: 'Kubi Hype', file: '08-kubi-hype.png' },
  { id: 'yuki-love', label: 'Yuki Love', file: '09-yuki-love.png' },
  { id: 'yuki-goodnight', label: 'Yuki Goodnight', file: '10-yuki-goodnight.png' },
  { id: 'yuki-pounce', label: 'Yuki Pounce', file: '11-yuki-pounce.png' },
  { id: 'blip-dance', label: 'Blip Dance', file: '12-blip-dance.png' },
  { id: 'blip-victory', label: 'Blip Victory', file: '13-blip-victory.png' },
  { id: 'blip-idea', label: 'Blip Idea', file: '14-blip-idea.png' },
  { id: 'kryos', label: 'Kryos', file: '15-kryos.png' },
]

export const STICKERS: StickerEntry[] = [
  ...BASE_STICKERS,
  ...RANKS.flatMap((rank) =>
    BASE_STICKERS.map((s): StickerEntry => ({ id: `${s.id}-${rank}`, label: `${s.label} (${RANK_LABEL[rank]})`, file: `${s.id}-${rank}.png`, rank })),
  ),
]

const STICKER_BY_ID = new Map(STICKERS.map((s) => [s.id, s]))

export function stickerFor(id: string): StickerEntry | null {
  return STICKER_BY_ID.get(id) ?? null
}

/** Site-relative address of a sticker's animation (an APNG). */
export function stickerPath(id: string): string | null {
  const s = STICKER_BY_ID.get(id)
  if (!s) return null
  return s.rank ? `/brand/rank/stickers/${s.file}` : `/brand/discord/stickers/${s.file}`
}

/**
 * Text with every rank emoji and sticker its writer may not use turned back
 * into the original (`held` is the writer's current rank, or null). The server
 * runs it on everything saved; a chat the server cannot read runs it when the
 * message is drawn.
 */
export function withoutRankArt(text: string, held: string | null | undefined): string {
  if (!text.includes(':')) return text
  return text
    .replace(/:([a-z0-9_]{1,60}):/g, (whole, name: string) => {
      const e = EMOJI_BY_NAME.get(name)
      return e?.rank && !canUseRank(e.rank, held) ? `:${rankOfName(name)!.base}:` : whole
    })
    .replace(/\[sticker:([a-z0-9-]{1,60})\]/g, (whole, id: string) => {
      const s = STICKER_BY_ID.get(id)
      return s?.rank && !canUseRank(s.rank, held) ? `[sticker:${rankOfName(id)!.base}]` : whole
    })
}

/** The text that puts an emoji or a sticker in. */
export const emojiToken = (name: string) => `:${name}:`
export const stickerToken = (id: string) => `[sticker:${id}]`

/** Emoji whose name or label has every word of the query, best first. */
export function searchEmoji(query: string): EmojiEntry[] {
  const words = query.toLowerCase().split(/[\s_:]+/).filter(Boolean)
  if (!words.length) return EMOJI
  return EMOJI.filter((e) => words.every((w) => e.label.includes(w) || e.name.includes(w)))
}
