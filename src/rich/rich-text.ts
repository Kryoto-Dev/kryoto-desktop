/**
 * Kryoto rich text: what every piece of text people write may carry, read
 * into a small tree each app draws with its own components (kryo.to:
 * components/rich-text.tsx). Nothing here produces markup: the tree holds
 * plain strings and checked addresses, and the renderers hand those to React,
 * which escapes them. Raw HTML in the text is just text.
 *
 * What it reads, in a comment, a bio, a chat message or anywhere else:
 *
 *     **bold**  *italic* or _italic_  ~~struck~~  ||spoiler||  `code`
 *     [words](https://address)  and bare addresses, http and https only
 *     > a quoted line          ``` a code block ```
 *     {rainbow}text effects{/rainbow}   (lib/text-effects.ts, the perk)
 *     <@123> mentions          (lib/mentions.ts; resolved ones too)
 *     :telly_wave: emoji       [sticker:telly-says-hi] stickers (lib/rich-catalog.ts)
 *     😂 standard emoji, which stay text and are marked to be drawn as big as ours
 *
 * A backslash keeps the next mark as itself (`\*not italic\*`). Long-form
 * pages (blog posts, the changelog) keep lib/markdown.ts, which reads the
 * same emoji, effects and mentions inside full markdown.
 *
 * Pure and dependency-free beyond its siblings: this file, text-effects.ts,
 * autolink.ts, mentions.ts and rich-catalog.ts are copied byte for byte into
 * Kryoto Desktop and Kryoto Mobile (`pnpm sync:rich-text`).
 */

import { splitLinks, safeHref } from './autolink'
import { MAX_MENTIONS, mentionLabel, parseResolved, type MentionIdentity } from './mentions'
import { STANDARD_EMOJI, isEmojiName, stickerFor } from './rich-catalog'
import { MAX_FX_SPANS, TEXT_EFFECTS, type TextEffectId } from './text-effects'

export type RichInline =
  | { t: 'text'; v: string }
  | { t: 'br' }
  | { t: 'strong' | 'em' | 'del' | 'spoiler'; c: RichInline[] }
  | { t: 'code'; v: string }
  | { t: 'link'; href: string; c: RichInline[] }
  | { t: 'fx'; fx: TextEffectId; v: string }
  /** `who` is null for a stored `<@id>` nobody resolved (end-to-end chat): the app looks it up. */
  | { t: 'mention'; id: string | null; who: MentionIdentity | null }
  | { t: 'emoji'; name: string }
  /** A standard emoji (rich-catalog.ts STANDARD_EMOJI), marked so it is drawn the size of ours. */
  | { t: 'emojichar'; v: string }
  | { t: 'sticker'; id: string }

export type RichBlock =
  | { t: 'para'; c: RichInline[] }
  | { t: 'quote'; c: RichInline[] }
  | { t: 'codeblock'; v: string }

/** Past this many characters the rest is read as plain text. */
export const RICH_PARSE_LIMIT = 20_000
/** Most emoji and stickers drawn from one text; the rest stay their words. */
export const MAX_EMOJI = 60
export const MAX_STICKERS = 3
/** Standard emoji marked in one text; past this they are drawn as plain text. */
export const MAX_EMOJI_CHARS = 300
/** Most emoji a text of nothing but emoji may have to be drawn big. */
export const JUMBO_MAX = 3

/** Marks a backslash can keep as themselves. */
const ESCAPABLE = '\\`*_~|[]{}<>:#'

const FX_AT = new RegExp(`\\{(${TEXT_EFFECTS.join('|')})\\}([\\s\\S]*?)\\{\\/\\1\\}`, 'y')
const FX_TAG = new RegExp(`\\{\\/?(${TEXT_EFFECTS.join('|')})\\}`)
const RAW_AT = /<@(\d{1,20})>/y
const RESOLVED_AT = /\uE000@([^\uE001]{0,200})\uE001/y
const EMOJI_AT = /:([a-z0-9_]{1,40}):/y
const STICKER_AT = /\[sticker:([a-z0-9-]{1,40})\]/y
const CHAR_AT = new RegExp(`(?:${STANDARD_EMOJI})`, 'uy')
const ANY_CHAR = new RegExp(STANDARD_EMOJI, 'u')
const LINK_AT = /\[([^[\]\n]{1,300})\]\(([^()\s]{1,2048})\)/y
const ADDRESS_AT = /(?:https?:\/\/|www\.)[^\s<>"]+/iy
const FENCE = /^ {0,3}```/
const QUOTE = /^ {0,3}>(?: |$)/

type Ctx = { fx: number; mentions: number; emoji: number; chars: number; stickers: number; inLink: boolean }

/** Where a written link may go: our own pages by path, or an http(s) address. */
function linkHref(url: string): string | null {
  if (url.startsWith('/') && !url.startsWith('//') && !url.startsWith('/\\')) return url
  return safeHref(url)
}

function at(re: RegExp, src: string, i: number): RegExpExecArray | null {
  re.lastIndex = i
  return re.exec(src)
}

const isSpace = (ch: string | undefined) => ch === undefined || /\s/.test(ch)
const isWord = (ch: string | undefined) => ch !== undefined && /[\p{L}\p{N}]/u.test(ch)

/** Where a `**`-style mark closes, or -1: not empty, not opened or closed against a space. */
function closing(src: string, mark: string, from: number, miss: Map<string, number>): number {
  if (isSpace(src[from])) return -1
  // Once a search found no close, none after it can either: one search per mark, not one per open.
  if (from >= (miss.get(mark) ?? Infinity)) return -1
  let i = src.indexOf(mark, from)
  while (i !== -1) {
    if (i > from && !isSpace(src[i - 1])) {
      // A longer run closes at its end: `**bold *italic***`.
      while (src[i + mark.length] === mark[0]) i++
      return i
    }
    i = src.indexOf(mark, i + 1)
  }
  miss.set(mark, Math.min(from, miss.get(mark) ?? Infinity))
  return -1
}

/** A single `*` or `_`: italic only when it is not inside a word (snake_case stays put). */
function italicClose(src: string, mark: '*' | '_', open: number, miss: Map<string, number>): number {
  if (mark === '_' && isWord(src[open - 1])) return -1
  if (src[open + 1] === mark || isSpace(src[open + 1])) return -1
  if (open + 2 >= (miss.get(mark) ?? Infinity)) return -1
  for (let i = src.indexOf(mark, open + 2); i !== -1; i = src.indexOf(mark, i + 1)) {
    if (src[i + 1] === mark) {
      i++
      continue
    }
    if (isSpace(src[i - 1])) continue
    if (mark === '_' && isWord(src[i + 1])) continue
    if (src.slice(open + 1, i).includes('\n')) return -1
    return i
  }
  miss.set(mark, Math.min(open + 2, miss.get(mark) ?? Infinity))
  return -1
}

function pushText(out: RichInline[], text: string, ctx: Ctx) {
  if (!text) return
  if (ctx.inLink) {
    out.push({ t: 'text', v: text })
    return
  }
  for (const p of splitLinks(text)) {
    if (p.href) out.push({ t: 'link', href: p.href, c: [{ t: 'text', v: p.text }] })
    else out.push({ t: 'text', v: p.text })
  }
}

const PAIRS: { mark: string; t: 'strong' | 'del' | 'spoiler' }[] = [
  { mark: '**', t: 'strong' },
  { mark: '__', t: 'strong' },
  { mark: '~~', t: 'del' },
  { mark: '||', t: 'spoiler' },
]

function inline(src: string, depth: number, ctx: Ctx): RichInline[] {
  const out: RichInline[] = []
  const miss = new Map<string, number>()
  let buf = ''
  const flush = () => {
    pushText(out, buf, ctx)
    buf = ''
  }
  let i = 0
  scan: while (i < src.length) {
    const ch = src[i]!
    if (ch === '\\' && i + 1 < src.length && ESCAPABLE.includes(src[i + 1]!)) {
      buf += src[i + 1]
      i += 2
      continue
    }
    if (ch === '\n') {
      flush()
      out.push({ t: 'br' })
      i++
      continue
    }
    // An address is text (linked later), so marks inside it stay put.
    if ((ch === 'h' || ch === 'H' || ch === 'w' || ch === 'W') && !isWord(src[i - 1])) {
      const m = at(ADDRESS_AT, src, i)
      if (m) {
        buf += m[0]
        i += m[0].length
        continue
      }
    }
    if (ch === '`') {
      const close = src.indexOf('`', i + 1)
      if (close > i + 1 && !src.slice(i + 1, close).includes('\n')) {
        flush()
        out.push({ t: 'code', v: src.slice(i + 1, close) })
        i = close + 1
        continue
      }
    }
    if (ch === '{' && ctx.fx < MAX_FX_SPANS) {
      const m = at(FX_AT, src, i)
      if (m && m[2] && !FX_TAG.test(m[2])) {
        flush()
        out.push({ t: 'fx', fx: m[1] as TextEffectId, v: m[2] })
        ctx.fx++
        i += m[0].length
        continue
      }
    }
    if (ch === '<' && ctx.mentions < MAX_MENTIONS) {
      const m = at(RAW_AT, src, i)
      if (m) {
        flush()
        out.push({ t: 'mention', id: m[1]!, who: null })
        ctx.mentions++
        i += m[0].length
        continue
      }
    }
    if (ch === '\uE000') {
      const m = at(RESOLVED_AT, src, i)
      if (m) {
        flush()
        const who = parseResolved(m[1]!)
        out.push({ t: 'mention', id: who.state === 'user' ? who.id : null, who })
        ctx.mentions++
        i += m[0].length
        continue
      }
      // A stray private-use character is nothing anyone wrote.
      i++
      continue
    }
    if (ch === ':' && ctx.emoji < MAX_EMOJI) {
      const m = at(EMOJI_AT, src, i)
      if (m && isEmojiName(m[1]!)) {
        flush()
        out.push({ t: 'emoji', name: m[1]! })
        ctx.emoji++
        i += m[0].length
        continue
      }
    }
    // A standard emoji starts past U+2000, or is a keycap (#, *, a digit, then U+FE0F or U+20E3).
    if (ctx.chars < MAX_EMOJI_CHARS && (src.charCodeAt(i) > 0x2000 || src[i + 1] === '️' || src[i + 1] === '⃣')) {
      const m = at(CHAR_AT, src, i)
      if (m) {
        flush()
        out.push({ t: 'emojichar', v: m[0] })
        ctx.chars++
        i += m[0].length
        continue
      }
    }
    if (ch === '[') {
      const s = ctx.stickers < MAX_STICKERS ? at(STICKER_AT, src, i) : null
      if (s && stickerFor(s[1]!)) {
        flush()
        out.push({ t: 'sticker', id: s[1]! })
        ctx.stickers++
        i += s[0].length
        continue
      }
      const m = !ctx.inLink ? at(LINK_AT, src, i) : null
      const href = m ? linkHref(m[2]!) : null
      if (m && href) {
        flush()
        out.push({ t: 'link', href, c: inline(m[1]!, depth + 1, { ...ctx, inLink: true }) })
        i += m[0].length
        continue
      }
    }
    if (depth < 4) {
      for (const p of PAIRS) {
        if (!src.startsWith(p.mark, i)) continue
        if (p.mark === '__' && isWord(src[i - 1])) continue
        const close = closing(src, p.mark, i + 2, miss)
        if (close === -1) continue
        flush()
        out.push({ t: p.t, c: inline(src.slice(i + 2, close), depth + 1, ctx) })
        i = close + 2
        continue scan
      }
      if (ch === '*' || ch === '_') {
        const close = italicClose(src, ch, i, miss)
        if (close !== -1) {
          flush()
          out.push({ t: 'em', c: inline(src.slice(i + 1, close), depth + 1, ctx) })
          i = close + 1
          continue
        }
      }
    }
    buf += ch
    i++
  }
  flush()
  return out
}

/** Read text into blocks. Cheap on plain text; never throws. */
export function parseRich(text: string): RichBlock[] {
  const source = text.replace(/\r\n?/g, '\n')
  const head = source.slice(0, RICH_PARSE_LIMIT)
  const tail = source.slice(RICH_PARSE_LIMIT)
  const ctx: Ctx = { fx: 0, mentions: 0, emoji: 0, chars: 0, stickers: 0, inLink: false }
  const lines = head.split('\n')
  const blocks: RichBlock[] = []
  let para: string[] = []
  const endPara = () => {
    if (!para.length) return
    blocks.push({ t: 'para', c: inline(para.join('\n'), 0, ctx) })
    para = []
  }
  for (let n = 0; n < lines.length; n++) {
    const line = lines[n]!
    if (FENCE.test(line)) {
      const end = lines.findIndex((l, k) => k > n && FENCE.test(l))
      if (end !== -1) {
        endPara()
        blocks.push({ t: 'codeblock', v: lines.slice(n + 1, end).join('\n') })
        n = end
        continue
      }
    }
    if (QUOTE.test(line)) {
      endPara()
      const quoted: string[] = []
      while (n < lines.length && QUOTE.test(lines[n]!)) quoted.push(lines[n++]!.replace(QUOTE, ''))
      n--
      blocks.push({ t: 'quote', c: inline(quoted.join('\n'), 0, ctx) })
      continue
    }
    para.push(line)
  }
  endPara()
  if (tail) blocks.push({ t: 'para', c: [{ t: 'text', v: tail }] })
  return blocks.length ? blocks : [{ t: 'para', c: [] }]
}

/** Text with nothing to read in it, which can be drawn as it is. */
export function isPlainRich(text: string): boolean {
  return !/[`*_~|[{<:\\\n>\uE000]|https?:\/\/|www\./i.test(text) && !ANY_CHAR.test(text)
}

/**
 * Text that is only one to three emoji (ours or standard, and spaces): drawn
 * big ("jumbo"), as Discord does, since an emoji alone at line height is a speck.
 */
export function isEmojiOnly(blocks: RichBlock[]): boolean {
  if (blocks.length !== 1 || blocks[0]!.t !== 'para') return false
  const c = blocks[0]!.c
  const isEmoji = (n: RichInline) => n.t === 'emoji' || n.t === 'emojichar'
  const emoji = c.filter(isEmoji).length
  return emoji > 0 && emoji <= JUMBO_MAX && c.every((n) => isEmoji(n) || (n.t === 'text' && !n.v.trim()))
}

function plainInline(nodes: RichInline[]): string {
  return nodes
    .map((n) => {
      switch (n.t) {
        case 'text':
        case 'code':
        case 'fx':
        case 'emojichar':
          return n.v
        case 'br':
          return '\n'
        // A quote or a notification must not give away what was hidden.
        case 'spoiler':
          return '[spoiler]'
        case 'mention':
          if (!n.who) return '@someone'
          return n.who.state === 'user' ? `@${mentionLabel(n.who)}` : mentionLabel(n.who)
        case 'emoji':
          return `:${n.name}:`
        case 'sticker':
          return `[${stickerFor(n.id)?.label ?? 'Sticker'}]`
        default:
          return plainInline(n.c)
      }
    })
    .join('')
}

/**
 * At most `max` characters of text, never cut inside a mention, an effect or
 * a sticker (a half token would show as its raw marks); `...` when cut.
 */
export function clipRich(text: string, max: number): string {
  if (text.length <= max) return text
  let cut = text.slice(0, Math.max(0, max - 3))
  const open = Math.max(cut.lastIndexOf('<@'), cut.lastIndexOf('\uE000'), cut.lastIndexOf('[sticker:'), cut.lastIndexOf('{'))
  if (open !== -1) {
    const ch = cut[open]
    const closed = ch === '<' ? cut.indexOf('>', open) : ch === '\uE000' ? cut.indexOf('\uE001', open) : ch === '[' ? cut.indexOf(']', open) : cut.indexOf('{/', open)
    if (closed === -1) cut = cut.slice(0, open)
  }
  return `${cut.trimEnd()}...`
}

/**
 * The words alone, for places that cannot draw any of it: notification
 * titles, Discord, a page's description, search.
 */
export function plainRich(text: string): string {
  return parseRich(text)
    .map((b) => (b.t === 'codeblock' ? b.v : plainInline(b.c)))
    .join('\n')
}
