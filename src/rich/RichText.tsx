import { Fragment, useEffect, useMemo, useState, type CSSProperties, type ReactNode } from 'react'
import { call } from '@/lib/bridge'
import { openExternal, openStorePage } from '@/lib/window'
import { cn } from '@/lib/utils'
import { mentionHref, mentionLabel, type MentionIdentity } from './mentions'
import { canUseRank, emojiFor, emojiPath, isMascotEmoji, rankOfName, stickerFor, stickerPath, type RankId } from './rich-catalog'
import { isEmojiOnly, isPlainRich, parseRich, type RichBlock, type RichInline } from './rich-text'
import type { TextEffectId } from './text-effects'
import { useAssetBase } from './asset-base'
import './rich-text.css'

/**
 * Text people wrote, drawn: Kryoto Desktop's renderer for kryo.to's rich text
 * (./rich-text.ts, copied from kryo.to by `pnpm sync:rich-text`; the site's
 * own renderer is kryo.to/components/rich-text.tsx). Every chat message, in
 * the app, in the web chat and in Kryoto Mobile's framed chat, goes through
 * here. React escapes everything; nothing written can become markup.
 *
 *   <RichText text={m.body} effects={sender.supporter} />
 *
 * Mentions: resolved ones (the public room, from the server) draw as they
 * are. A stored `<@id>` (end-to-end chat, which the server never reads) is
 * named by `people` when the chat knows the person, otherwise asked of kryo.to
 * (`/api/mentions`, which masks anonymous people), and reads "@someone" until
 * then. Emoji and stickers load from kryo.to itself.
 */
export function RichText({
  text,
  className,
  effects = true,
  rank,
  people,
}: {
  text: string
  className?: string
  /**
   * Draw text effects. End-to-end chat cannot strip effects the writer does
   * not own on a server, so it passes whether the sender is a supporter.
   */
  effects?: boolean
  /**
   * The sender's donation rank, for end-to-end chat: rank emoji and stickers
   * above it are drawn as the originals. Left out, `effects` decides.
   */
  rank?: string | null
  /** Names the chat already knows: a person in it, or null to ask kryo.to. */
  people?: (id: string) => MentionIdentity | null
}) {
  const blocks = useMemo(() => (isPlainRich(text) ? null : parseRich(text)), [text])
  const unknown = useMemo(() => (blocks ? rawIds(blocks).filter((id) => !people?.(id)) : NO_IDS), [blocks, people])
  const asked = useMentionLookup(unknown)
  const base = useAssetBase()
  if (!blocks) return <span className={className}>{text}</span>
  const ctx: Ctx = { effects, rank, base, who: (id) => people?.(id) ?? asked[id] ?? null }
  const jumbo = isEmojiOnly(blocks)
  if (blocks.length === 1 && blocks[0]!.t === 'para') {
    return <span className={cn(jumbo && 'kryo-emoji-jumbo', className)}>{inlines(blocks[0]!.c, ctx)}</span>
  }
  return <span className={cn(jumbo && 'kryo-emoji-jumbo', className)}>{blocks.map((b, i) => block(b, i, ctx))}</span>
}

type Ctx = { effects: boolean; rank: string | null | undefined; base: string; who: (id: string) => MentionIdentity | null }

/** Whether a piece of rank art is drawn as written: always when it is not rank art; else by the sender's rank when known, or whether they are a supporter. */
function rankArtOk(needed: RankId | undefined, ctx: Ctx): boolean {
  if (!needed) return true
  return ctx.rank !== undefined ? canUseRank(needed, ctx.rank) : ctx.effects
}

const NO_IDS: string[] = []

/** One piece of text in one effect (a name, or an effect span in text). */
export function FxSpan({ fx, children, className }: { fx: TextEffectId | null | undefined; children: string; className?: string }) {
  if (!fx) return <span className={className}>{children}</span>
  if (fx === 'wave') {
    return (
      <span className={cn('kryo-fx kryo-fx-wave', className)}>
        <span className="sr-only">{children}</span>
        {Array.from(children).map((ch, i) => (
          <span key={i} aria-hidden style={{ '--i': i } as CSSProperties}>
            {ch === ' ' ? ' ' : ch}
          </span>
        ))}
      </span>
    )
  }
  if (fx === 'glitch') {
    return (
      <span className={cn('kryo-fx kryo-fx-glitch', className)}>
        {children}
        <span aria-hidden className="kryo-fx-glitch-a">
          {children}
        </span>
        <span aria-hidden className="kryo-fx-glitch-b">
          {children}
        </span>
      </span>
    )
  }
  return <span className={cn('kryo-fx', `kryo-fx-${fx}`, className)}>{children}</span>
}

function block(b: RichBlock, key: number, ctx: Ctx): ReactNode {
  if (b.t === 'codeblock') {
    return (
      <code key={key} className="kryo-rich-block kryo-codeblock">
        {b.v}
      </code>
    )
  }
  return (
    <span key={key} className={cn('kryo-rich-block', b.t === 'quote' && 'kryo-quote')}>
      {inlines(b.c, ctx)}
    </span>
  )
}

function inlines(nodes: RichInline[], ctx: Ctx): ReactNode {
  return nodes.map((n, i) => <Fragment key={i}>{inline(n, ctx)}</Fragment>)
}

/** kryo.to's own pages open in the Store; anywhere else in the browser. */
function follow(href: string) {
  try {
    const url = new URL(href, 'https://kryo.to')
    const host = url.hostname.toLowerCase()
    if (host === 'kryo.to' || host === 'www.kryo.to') {
      openStorePage(`${url.pathname}${url.search}${url.hash}`)
      return
    }
  } catch {
    return
  }
  void openExternal(href)
}

const LINK =
  'text-foreground underline decoration-1 underline-offset-2 [overflow-wrap:anywhere] hover:text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-primary'

function inline(n: RichInline, ctx: Ctx): ReactNode {
  switch (n.t) {
    case 'text':
      return n.v
    case 'br':
      return <br />
    case 'strong':
      return <strong className="font-bold">{inlines(n.c, ctx)}</strong>
    case 'em':
      return <em className="italic">{inlines(n.c, ctx)}</em>
    case 'del':
      return <del>{inlines(n.c, ctx)}</del>
    case 'spoiler':
      return (
        <span className="kryo-spoiler" tabIndex={0} aria-label="Spoiler, select to show">
          {inlines(n.c, ctx)}
        </span>
      )
    case 'code':
      return <code className="kryo-code">{n.v}</code>
    case 'link': {
      const href = n.href.startsWith('/') ? `https://kryo.to${n.href}` : n.href
      return (
        <a
          href={href}
          rel="noopener noreferrer nofollow ugc"
          className={LINK}
          onClick={(e) => {
            e.preventDefault()
            follow(href)
          }}
        >
          {inlines(n.c, ctx)}
        </a>
      )
    }
    case 'fx':
      return ctx.effects ? <FxSpan fx={n.fx}>{n.v}</FxSpan> : n.v
    case 'mention':
      return <Mention who={n.who ?? (n.id ? ctx.who(n.id) : null)} />
    case 'emoji': {
      // End-to-end chat never reaches a server that could check a writer's rank:
      // rank art above the sender's rank is drawn as the original.
      const name = rankArtOk(emojiFor(n.name)?.rank, ctx) ? n.name : rankOfName(n.name)!.base
      return (
        <img
          src={`${ctx.base}${emojiPath(name)}`}
          alt={`:${name}:`}
          className={cn('kryo-emoji', isMascotEmoji(name) && 'kryo-emoji-mascot')}
          width={22}
          height={22}
          loading="lazy"
          decoding="async"
          draggable={false}
        />
      )
    }
    case 'emojichar':
      return <span className="kryo-emoji-char">{n.v}</span>
    case 'sticker': {
      const id = rankArtOk(stickerFor(n.id)?.rank, ctx) ? n.id : rankOfName(n.id)!.base
      const src = stickerPath(id)
      if (!src) return null
      return <img src={`${ctx.base}${src}`} alt={`Sticker: ${stickerFor(id)?.label ?? id}`} className="kryo-sticker" width={128} height={128} loading="lazy" decoding="async" draggable={false} />
    }
  }
}

/** A profile link for a public account; plain words for anyone else, with no tooltip. */
function Mention({ who }: { who: MentionIdentity | null }) {
  if (!who) return <span className="kryo-mention kryo-mention-quiet">@someone</span>
  const href = mentionHref(who)
  const label = mentionLabel(who)
  if (!href) return <span className="kryo-mention kryo-mention-quiet">{label}</span>
  return (
    <a
      href={`https://kryo.to${href}`}
      className="kryo-mention"
      onClick={(e) => {
        e.preventDefault()
        openStorePage(href)
      }}
    >
      @{label}
    </a>
  )
}

function rawIds(blocks: RichBlock[]): string[] {
  const out: string[] = []
  const walk = (nodes: RichInline[]) => {
    for (const n of nodes) {
      if (n.t === 'mention' && !n.who && n.id && !out.includes(n.id)) out.push(n.id)
      if ('c' in n) walk(n.c)
    }
  }
  for (const b of blocks) if (b.t !== 'codeblock') walk(b.c)
  return out
}

// ---- asking kryo.to about mentions the chat does not know ----------------------------

const known = new Map<string, MentionIdentity>()
const tried = new Set<string>()
const waiting = new Set<string>()
const listeners = new Set<() => void>()
let timer: ReturnType<typeof setTimeout> | null = null

function ask(ids: string[]) {
  for (const id of ids) if (!known.has(id) && !tried.has(id)) waiting.add(id)
  if (!waiting.size || timer) return
  timer = setTimeout(() => {
    timer = null
    const batch = [...waiting].slice(0, 64)
    for (const id of batch) waiting.delete(id)
    void call<{ people?: Record<string, MentionIdentity> }>('chat_mention_lookup', { ids: batch })
      .catch(() => ({ people: {} as Record<string, MentionIdentity> }))
      .then((data) => {
        for (const id of batch) {
          tried.add(id)
          const who = data.people?.[id]
          if (who) known.set(id, who)
        }
        for (const fn of listeners) fn()
        if (waiting.size) ask([])
      })
  }, 30)
}

function useMentionLookup(ids: string[]): Record<string, MentionIdentity> {
  const [, bump] = useState(0)
  const key = ids.join(',')
  useEffect(() => {
    if (!key) return
    const fn = () => bump((n) => n + 1)
    listeners.add(fn)
    ask(key.split(','))
    return () => {
      listeners.delete(fn)
    }
  }, [key])
  const out: Record<string, MentionIdentity> = {}
  for (const id of ids) {
    const who = known.get(id)
    if (who) out[id] = who
  }
  return out
}
