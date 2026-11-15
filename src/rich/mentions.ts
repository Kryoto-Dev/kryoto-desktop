/**
 * Mentions in text people write: how they are stored, and how a resolved one
 * travels to the page.
 *
 *   * Typed: `@username`, usually picked from the `@` list in the editor.
 *   * Stored: `<@123456789>`, the account's id. Never a name: a name changes,
 *     and a person who turns anonymous must stop being named by what other
 *     people wrote about them before.
 *   * Shown: the server swaps each stored id for a RESOLVED token, built here
 *     from the account as it is at that moment, just before the text leaves
 *     for the page (lib/mentions-db.ts `resolveMentions`). An anonymous or
 *     deleted account becomes a token that carries no id, name or address.
 *
 * A resolved token is wrapped in two private-use characters (U+E000, U+E001)
 * that nobody's text can carry: `cleanMentionInput` takes them out of
 * everything written, so a token can only ever come from the server.
 *
 * Pure and dependency-free: copied byte for byte into Kryoto Desktop and
 * Kryoto Mobile (`pnpm sync:rich-text`).
 */

/** The canonical username rule (lib/accounts.ts USERNAME_RE). */
export const USERNAME_CHARS = 'A-Za-z0-9_.-'
const USERNAME_RE = /^[A-Za-z0-9_.-]{3,32}$/

/** Most mentions one text may carry; past it the rest stay plain text. */
export const MAX_MENTIONS = 20

/**
 * How many people an account may mention, enforced on the server in
 * lib/mentions-db.ts (`prepareRichText` for the text, `notifyMentions` for who
 * is told). Here so the editors can say so before anything is sent. Past
 * `perMessage` the rest stay plain words; past the rates a mention still shows
 * but tells nobody.
 */
export const MENTION_LIMITS = {
  member: { perMessage: 5, perHour: 15, perDay: 40 },
  staff: { perMessage: MAX_MENTIONS, perHour: 60, perDay: 200 },
} as const

export type MentionTier = keyof typeof MENTION_LIMITS

/** The same writer telling the same person, per hour. */
export const MENTION_PAIR_PER_HOUR = 3
/** Mention notifications anyone receives per hour, from everybody together. */
export const MENTION_RECEIVE_PER_HOUR = 20
/** Hitting a limit this many times in a day stops the account's mentions notifying. */
export const MENTION_STRIKES = 3
export const MENTION_MUTE_HOURS = 24

/** People a draft mentions, once each: typed names and stored ids. */
export function mentionCount(text: string): number {
  const typed = typedMentions(text).map((m) => m.username.toLowerCase())
  return new Set([...typed, ...mentionIds(text)]).size
}

/** What an editor says when a draft names more people than one message may. */
export function mentionLimitNote(perMessage: number): string {
  return `You can mention up to ${perMessage} people in one message. Anyone past that stays plain text and is not told.`
}

const OPEN = '\uE000'
const CLOSE = '\uE001'
const SEP = '\u001F'

/** A stored mention. */
export const RAW_MENTION = /<@(\d{1,20})>/g
/** A resolved one (see the header). */
export const RESOLVED_MENTION = /\uE000@([^\uE001]{0,200})\uE001/g

export type MentionIdentity =
  | { state: 'user'; id: string; username: string; displayName: string | null }
  | { state: 'anonymous' }
  | { state: 'deleted' }

/** The stored form of a mention of this account. */
export function rawMention(id: string): string {
  return `<@${id}>`
}

/** Text without the two characters only the server may write. */
export function cleanMentionInput(text: string): string {
  return text.replace(/[\uE000\uE001]/g, '')
}

/** Every id mentioned in stored text, once each, in order. */
export function mentionIds(text: string): string[] {
  const out: string[] = []
  for (const m of text.matchAll(RAW_MENTION)) {
    const id = m[1]!
    if (!out.includes(id)) out.push(id)
  }
  return out
}

/**
 * `@username` words a writer typed, found so the server can turn them into
 * stored mentions. Not inside an email address or a link (`a@b.c`,
 * `kryo.to/@x`), and a dot or dash ending a sentence is not part of the name.
 */
export function typedMentions(text: string): { at: number; length: number; username: string }[] {
  const out: { at: number; length: number; username: string }[] = []
  const re = new RegExp(`(^|[^${USERNAME_CHARS}@/\\\\])@([${USERNAME_CHARS}]{3,40})`, 'g')
  for (const m of text.matchAll(re)) {
    const name = m[2]!.replace(/[.-]+$/, '')
    if (!USERNAME_RE.test(name)) continue
    out.push({ at: (m.index ?? 0) + m[1]!.length, length: name.length + 1, username: name })
  }
  return out
}

/** Strip anything that could break out of a token from a name before it goes in one. */
function tokenSafe(value: string): string {
  let out = ''
  for (const ch of value) {
    const code = ch.codePointAt(0)!
    if (code >= 0x20 && code !== 0x7f && code !== 0xe000 && code !== 0xe001) out += ch
  }
  return out.slice(0, 64)
}

/** The resolved token for an account as it is now. */
export function resolvedToken(who: MentionIdentity): string {
  if (who.state === 'anonymous') return `${OPEN}@?${CLOSE}`
  if (who.state === 'deleted') return `${OPEN}@-${CLOSE}`
  return `${OPEN}@${who.id}${SEP}${tokenSafe(who.username)}${SEP}${tokenSafe(who.displayName ?? '')}${CLOSE}`
}

/** Read a resolved token's inside (what RESOLVED_MENTION captured). */
export function parseResolved(inner: string): MentionIdentity {
  if (inner === '?') return { state: 'anonymous' }
  const [id, username, displayName] = inner.split(SEP)
  if (!id || !/^\d{1,20}$/.test(id) || !username || !USERNAME_RE.test(username)) return { state: 'deleted' }
  return { state: 'user', id, username, displayName: displayName || null }
}

/** What a mention reads as. An anonymous account is "Anonymous", never its name. */
export function mentionLabel(who: MentionIdentity): string {
  if (who.state === 'anonymous') return 'Anonymous'
  if (who.state === 'deleted') return 'deleted account'
  return who.displayName || who.username
}

/** The profile a mention opens: only a public, existing account has one. */
export function mentionHref(who: MentionIdentity): string | null {
  return who.state === 'user' ? `/user/${encodeURIComponent(who.username)}` : null
}

/**
 * Text as its writer edits it again: resolved mentions back to `@username`,
 * which the server turns into the same stored mention on save. A mention of
 * somebody who is anonymous now cannot be written back without naming them,
 * so it becomes the plain word.
 */
export function editableText(text: string): string {
  return text
    .replace(RESOLVED_MENTION, (_m, inner: string) => {
      const who = parseResolved(inner)
      return who.state === 'user' ? `@${who.username}` : mentionLabel(who)
    })
    .replace(RESOLVED_GAME, (_m, inner: string) => {
      const g = parseResolvedGame(inner)
      return g.state === 'game' ? rawGame(g.slug, g.release?.id, g.minutes) : ''
    })
}

/** Mentions as words, for text that cannot hold a link (notifications, Discord, titles). */
export function plainMentions(text: string): string {
  return text
    .replace(RESOLVED_MENTION, (_m, inner: string) => {
      const who = parseResolved(inner)
      return who.state === 'user' ? `@${mentionLabel(who)}` : mentionLabel(who)
    })
    .replace(RAW_MENTION, '@someone')
    .replace(RESOLVED_GAME, (_m, inner: string) => gameMentionLabel(parseResolvedGame(inner)))
    .replace(RAW_GAME, 'a game')
}

/*
 * Game mentions: "GoreBox27 b1, played for 12 hours", a link that opens the
 * game with that build picked. Stored by the game's slug (its id here) as
 * `[game:<slug>]`, with
 * `:v<release id>` for a build and `:p<minutes>` for the writer's play time
 * (the server writes the minutes from the writer's own sessions, never what
 * was typed). Resolved on the server like `<@id>`, into a token between the
 * same two private-use characters, marked `#`.
 */

/** Most games one text may mention. */
export const MAX_GAME_MENTIONS = 10
/** A stored game mention. */
export const RAW_GAME = /\[game:([a-z0-9-]{1,120})(?::v(\d{1,12}))?(?::p(\d{1,7}))?\]/g
/** A resolved one. */
export const RESOLVED_GAME = /#([^]{0,400})/g

export type GameMention =
  | { state: 'game'; slug: string; title: string; release: { id: string; label: string } | null; minutes: number | null }
  /** Gone, hidden or not out: nothing about it is shown. */
  | { state: 'gone' }

/** The stored form. */
export function rawGame(slug: string, release?: string | null, minutes?: number | null): string {
  return `[game:${slug}${release ? `:v${release}` : ''}${minutes && minutes > 0 ? `:p${Math.round(minutes)}` : ''}]`
}

/** Every stored game mention: ids, release ids and minutes, in order. */
export function gameMentions(text: string): { slug: string; release: string | null; minutes: number | null }[] {
  return [...text.matchAll(RAW_GAME)].map((m) => ({ slug: m[1]!, release: m[2] ?? null, minutes: m[3] ? Number(m[3]) : null }))
}

/** Whether a text has anything the server resolves: people or games. */
export function hasMentionTokens(text: string): boolean {
  return text.includes('<@') || text.includes('[game:')
}

/** The resolved token for a game as it is now. */
export function resolvedGameToken(g: GameMention): string {
  if (g.state === 'gone') return `${OPEN}#-${CLOSE}`
  const slug = /^[a-z0-9-]{1,120}$/.test(g.slug) ? g.slug : ''
  return `${OPEN}#${slug}${SEP}${tokenSafe(g.title)}${SEP}${g.release?.id ?? ''}${SEP}${tokenSafe(g.release?.label ?? '')}${SEP}${g.minutes ?? ''}${CLOSE}`
}

/** Read a resolved game token's inside (what RESOLVED_GAME captured). */
export function parseResolvedGame(inner: string): GameMention {
  const [slug, title, releaseId, releaseLabel, minutes] = inner.split(SEP)
  if (!slug || !/^[a-z0-9-]{1,120}$/.test(slug) || !title) return { state: 'gone' }
  const release = releaseId && /^\d{1,12}$/.test(releaseId) ? { id: releaseId, label: releaseLabel || '' } : null
  const m = minutes && /^\d{1,7}$/.test(minutes) ? Number(minutes) : null
  return { state: 'game', slug, title, release, minutes: m && m > 0 ? m : null }
}

/** Play time in words: "45 minutes", "1 hour", "12 hours". */
export function playedFor(minutes: number): string {
  if (minutes < 60) return `${minutes} minute${minutes === 1 ? '' : 's'}`
  const h = minutes < 600 ? Math.round(minutes / 6) / 10 : Math.round(minutes / 60)
  return `${h} hour${h === 1 ? '' : 's'}`
}

/** What a game mention reads as: "GoreBox27 b1, played for 12 hours". */
export function gameMentionLabel(g: GameMention): string {
  if (g.state === 'gone') return 'a game that is not on Kryoto'
  const name = g.release?.label ? `${g.title} ${g.release.label}` : g.title
  return g.minutes ? `${name}, played for ${playedFor(g.minutes)}` : name
}

/** The page a game mention opens: the game, with its build picked. */
export function gameMentionHref(g: GameMention): string | null {
  if (g.state === 'gone') return null
  const path = `/game/${encodeURIComponent(g.slug)}`
  return g.release ? `${path}?release=${encodeURIComponent(g.release.id)}&download=1` : path
}
