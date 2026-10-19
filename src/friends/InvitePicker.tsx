import { useEffect, useMemo, useState } from 'react'
import { Gamepad2, Search } from 'lucide-react'
import { artSrc } from '@/lib/art'
import { errorText } from '@/lib/bridge'
import { chatSendInvite, onlineLobby } from '@/lib/chat'
import { Caption, inputCls, Modal } from '@/ui'

export type InviteGame = { id: string; slug: string; title: string; cover: string | null; running: boolean }

/**
 * Pick a game from your library to invite someone to. A game you are playing
 * comes first, and when Kryoto Online reports the Steam lobby you are in, the
 * invite carries it so "Join" puts them straight into it.
 */
export function InvitePicker({
  peer,
  games,
  onClose,
  onSent,
}: {
  peer: { id: string; name: string }
  games: InviteGame[]
  onClose: () => void
  onSent: () => void
}) {
  const [query, setQuery] = useState('')
  const [lobbies, setLobbies] = useState<Record<string, { lobby: string; hostSteamId: string }>>({})
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    for (const g of games.filter((g) => g.running)) {
      void onlineLobby(g.id)
        .then((l) => l && setLobbies((m) => ({ ...m, [g.id]: l })))
        .catch(() => {})
    }
  }, [games])

  const list = useMemo(() => {
    const q = query.trim().toLowerCase()
    return games
      .filter((g) => !q || g.title.toLowerCase().includes(q))
      .sort((a, b) => Number(b.running) - Number(a.running) || a.title.localeCompare(b.title))
      .slice(0, 60)
  }, [games, query])

  const send = async (g: InviteGame) => {
    setBusy(true)
    setError(null)
    try {
      const lobby = lobbies[g.id]
      await chatSendInvite(peer.id, { slug: g.slug, title: g.title, steamLobby: lobby?.lobby ?? '', hostSteamId: lobby?.hostSteamId ?? '' })
      onSent()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal title={`Invite ${peer.name} to play`} onClose={onClose}>
      <label className="relative block">
        <Search className="pointer-events-none absolute left-3 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" aria-hidden />
        <input
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search your library"
          aria-label="Search your library"
          className={`${inputCls} pl-9`}
        />
      </label>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
      {list.length === 0 ? (
        <p className="py-6 text-center text-xs text-muted-foreground">
          {games.length === 0 ? 'Games you add to your library show up here.' : 'Nothing matches.'}
        </p>
      ) : (
        <ul className="grid max-h-80 gap-1 overflow-auto">
          {list.map((g) => (
            <li key={g.id}>
              <button
                type="button"
                disabled={busy}
                onClick={() => void send(g)}
                className="kryo-radius flex w-full items-center gap-3 px-2 py-1.5 text-left text-xs text-foreground transition-colors hover:bg-secondary disabled:opacity-50"
              >
                {g.cover ? (
                  <img decoding="async" src={artSrc(g.cover) ?? undefined} alt="" className="kryo-radius h-10 w-7 shrink-0 object-cover" />
                ) : (
                  <span className="kryo-radius grid h-10 w-7 shrink-0 place-items-center bg-secondary">
                    <Gamepad2 className="size-3.5 text-muted-foreground" aria-hidden />
                  </span>
                )}
                <span className="grid min-w-0">
                  <span className="truncate">{g.title}</span>
                  {g.running ? (
                    <span className="text-[10px] text-primary">
                      {lobbies[g.id] ? 'Playing now - they can join your lobby' : 'Playing now'}
                    </span>
                  ) : null}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <Caption>They get a card with Join: it starts the game, or opens its page so they can get it first.</Caption>
    </Modal>
  )
}
