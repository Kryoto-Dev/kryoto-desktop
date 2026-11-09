import { useEffect, useState } from 'react'
import { CircleCheck, CircleX, Flame } from '@/ui/pixel-icons'
import { Caption } from '@/ui'
import { catalogApiUrl } from '@/lib/endpoint'
import { viaStore } from '@/lib/friends'
import { isTauri, isWeb, call } from '@/lib/bridge'
import { cn } from '@/lib/utils'
import { useAccount } from '@/hooks/useAccount'
import { RichText } from '@/rich/RichText'
import { EmojiButton } from '@/rich/RichEditor'
import { emojiPath } from '@/rich/rich-catalog'
import { useAssetBase } from '@/rich/asset-base'

/**
 * What players thought of a game, in its About card on the Library page: the
 * two most helpful write-ups from kryo.to, each with its reactions, which you
 * can add to or take back right here (kryo.to's own reactions, the same as on
 * the game's page: lib/reactions-db.ts decides everything). The rest, and
 * writing your own, are on the game's page ("Write it up" above).
 */
type Writeup = {
  id: string
  body: string
  author: { username: string | null; displayName: string | null }
  writeup?: { recommends: boolean | null; topOfWeek?: boolean } | null
  helpful?: { count: number }
  createdAt: string
}
type Reaction = { emoji: string; count: number; mine: boolean; people: { name: string; username: string | null }[] }

const SHOWN = 2

/** A reading or a reaction through the Store's kryo.to page, signed in as the player (lib.rs `friend_route_allowed`). */
function signed(method: 'GET' | 'POST', path: string, body?: Record<string, unknown>): Promise<Record<string, unknown>> {
  const f = { method, path, body }
  return isTauri() && !isWeb() ? viaStore(f) : call<Record<string, unknown>>('friends_fetch', f)
}

export function LibraryWriteups({ slug, onWeb, own = false }: { slug: string; onWeb: (path: string) => void; own?: boolean }) {
  const account = useAccount()
  const signedIn = !!account && !account.guest
  const rank = account?.rank !== undefined ? account.rank : account?.supporter ? 'supporter' : null
  const base = useAssetBase()
  const [list, setList] = useState<Writeup[]>([])
  const [reactions, setReactions] = useState<Record<string, Reaction[]>>({})
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let live = true
    void (async () => {
      const res = await fetch(await catalogApiUrl(`/api/games/${encodeURIComponent(slug)}/writeups`)).catch(() => null)
      const json = res?.ok ? ((await res.json().catch(() => null)) as { writeups?: Writeup[] } | null) : null
      const top = (json?.writeups ?? [])
        .filter((w) => w && typeof w.body === 'string' && w.body.trim())
        .sort((a, b) => Number(!!b.writeup?.topOfWeek) - Number(!!a.writeup?.topOfWeek) || (b.helpful?.count ?? 0) - (a.helpful?.count ?? 0) || b.createdAt.localeCompare(a.createdAt))
        .slice(0, SHOWN)
      if (!live) return
      setList(top)
      if (!top.length) return
      const t = top.map((w) => `comment:${w.id}`).join(',')
      // Signed in, the Store page asks, so your own reactions show as yours.
      const got = signedIn
        ? await signed('GET', `/api/reactions?t=${t}`).catch(() => null)
        : await fetch(await catalogApiUrl(`/api/reactions?t=${t}`)).then((r) => (r.ok ? r.json() : null)).catch(() => null)
      if (live && got && typeof got.reactions === 'object') setReactions(got.reactions as Record<string, Reaction[]>)
    })()
    return () => {
      live = false
    }
  }, [slug, signedIn])

  async function react(id: string, emoji: string, on: boolean) {
    if (!signedIn || busy) return
    setBusy(true)
    setError(null)
    try {
      const got = await signed('POST', '/api/reactions', { target: `comment:${id}`, emoji, on })
      if (Array.isArray(got.reactions)) setReactions((r) => ({ ...r, [`comment:${id}`]: got.reactions as Reaction[] }))
    } catch (e) {
      setError(e instanceof Error ? e.message : 'That could not be saved.')
    } finally {
      setBusy(false)
    }
  }

  if (!list.length) return null
  // Under the game's description in its About card, or as its own card when it has none.
  return (
    <div className={cn('grid gap-3', own ? 'border border-border bg-card p-5' : 'mt-4 border-t border-border pt-3')}>
      <div className="flex items-center justify-between gap-2">
        <Caption>What players thought</Caption>
        <button type="button" onClick={() => onWeb(`/game/${encodeURIComponent(slug)}#writeups`)} className="text-[11px] text-muted-foreground underline-offset-2 hover:text-foreground hover:underline">
          All write-ups
        </button>
      </div>
      {list.map((w) => {
        const name = w.author.displayName || w.author.username || 'Anonymous'
        const rs = reactions[`comment:${w.id}`] ?? []
        return (
          <article key={w.id} className="grid gap-1.5">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
              {/* A name opens its profile; an anonymous writer has neither name nor link. */}
              {w.author.username ? (
                <button type="button" onClick={() => onWeb(`/user/${encodeURIComponent(w.author.username!)}`)} className="font-bold text-foreground hover:underline">
                  {name}
                </button>
              ) : (
                <span className="font-bold text-foreground">{w.author.displayName || 'Anonymous'}</span>
              )}
              {w.writeup?.topOfWeek ? (
                <span className="inline-flex items-center gap-1 font-bold uppercase tracking-wider text-primary">
                  <Flame className="size-3" aria-hidden /> Top write-up this week
                </span>
              ) : null}
              {w.writeup?.recommends === true ? (
                <span className="inline-flex items-center gap-1 font-bold uppercase tracking-wider text-primary">
                  <CircleCheck className="size-3" aria-hidden /> Recommends it
                </span>
              ) : w.writeup?.recommends === false ? (
                <span className="inline-flex items-center gap-1 font-bold uppercase tracking-wider text-destructive">
                  <CircleX className="size-3" aria-hidden /> Does not recommend it
                </span>
              ) : null}
            </div>
            <p className="line-clamp-3 break-words text-sm leading-relaxed text-foreground/90">
              <RichText text={w.body} />
            </p>
            <div className="flex flex-wrap items-center gap-1" role="group" aria-label="Reactions">
              {rs.map((r) => (
                <button
                  key={r.emoji}
                  type="button"
                  disabled={!signedIn || busy}
                  aria-pressed={r.mine}
                  title={`${r.people.map((p) => p.name).join(', ') || 'Somebody'} reacted with :${r.emoji}:`}
                  onClick={() => void react(w.id, r.emoji, !r.mine)}
                  className={cn(
                    'kryo-pill inline-flex h-7 items-center gap-1 border px-1.5 text-xs tabular-nums transition-colors disabled:cursor-default',
                    r.mine ? 'border-primary bg-primary/15 text-foreground' : 'border-border text-muted-foreground enabled:hover:text-foreground',
                  )}
                >
                  <img src={`${base}${emojiPath(r.emoji)}`} alt={`:${r.emoji}:`} width={18} height={18} className="size-[18px]" />
                  {r.count}
                </button>
              ))}
              {signedIn ? <EmojiButton stickers={false} rank={rank} align="left" onPick={(token) => void react(w.id, token.slice(1, -1), true)} /> : null}
            </div>
          </article>
        )
      })}
      {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
    </div>
  )
}
