import { useCallback, useRef, useState } from 'react'
import { Clock } from '@/ui/pixel-icons'
import { Button, Modal } from '@/ui'
import { isTauri } from '@/lib/bridge'
import { viaStore } from '@/lib/friends'
import { playtime, type LibraryGame, type PlaytimeOffer } from '@/lib/library'
import { logWarn } from '@/lib/log'

/** How long Play waits for kryo.to's count before going ahead without it. */
const ACCOUNT_WAIT_MS = 2500

/** "12h 30m", the way the question says it. */
export function hoursAndMinutes(seconds: number): string {
  // Round to the minute first, so 59 min 40 s reads "1h" and never "60m".
  const total = Math.round(seconds / 60)
  const h = Math.floor(total / 60)
  const m = total % 60
  if (h === 0) return `${m}m`
  return m ? `${h}h ${m}m` : `${h}h`
}

/** The account's play time for one game, or null (a guest, offline, no answer in time). */
async function accountSeconds(slug: string): Promise<number | null> {
  const ask = viaStore({ method: 'GET', path: `/api/account/playtime?slugs=${encodeURIComponent(slug)}` }).then((data) => {
    const v = data[slug]
    return typeof v === 'number' && Number.isFinite(v) ? v : null
  })
  const late = new Promise<null>((resolve) => window.setTimeout(() => resolve(null), ACCOUNT_WAIT_MS))
  return Promise.race([ask, late]).catch(() => null)
}

/**
 * Play, with one question first when this game has hours this PC does not
 * count: the game was installed again into a folder that remembers them, or
 * kryo.to's account has more (another PC, an older install). Yes keeps the
 * larger count, Later asks next time, No never asks again for this game.
 * Every answer starts the game; nothing is added without a yes.
 */
export function usePlaytimeGate({ signedIn, onChanged }: { signedIn: boolean; onChanged: (g: LibraryGame) => void }) {
  const [asking, setAsking] = useState<{ game: LibraryGame; offer: PlaytimeOffer; go: () => void } | null>(null)
  const busy = useRef(new Set<string>())

  const gate = useCallback(
    (game: LibraryGame, go: () => void) => {
      if (game.playtimePrompt === 'never' || busy.current.has(game.id)) return go()
      busy.current.add(game.id)
      void (async () => {
        try {
          // The account's count comes through the Store's page, which only the app has.
          const account = isTauri() && signedIn && game.slug ? await accountSeconds(game.slug) : null
          const offer = await playtime.offer(game.id, account)
          if (offer) setAsking({ game, offer, go })
          else go()
        } catch (e) {
          logWarn('playtime', `offer for ${game.id}: ${String(e)}`)
          go()
        } finally {
          busy.current.delete(game.id)
        }
      })()
    },
    [signedIn],
  )

  const answer = (choice: 'yes' | 'later' | 'no') => {
    const a = asking
    if (!a) return
    setAsking(null)
    void playtime
      .answer(a.game.id, choice, a.offer.seconds)
      .then(onChanged)
      .catch((e) => logWarn('playtime', `answer for ${a.game.id}: ${String(e)}`))
      .finally(a.go)
  }

  const modal = asking ? (
    <Modal
      title="Welcome back"
      onClose={() => answer('later')}
      footer={
        <>
          <Button variant="ghost" onClick={() => answer('no')}>
            No
          </Button>
          <Button onClick={() => answer('later')}>Later</Button>
          <Button variant="primary" onClick={() => answer('yes')}>
            <Clock className="size-3.5" />
            Restore
          </Button>
        </>
      }
    >
      <p className="text-sm leading-relaxed text-foreground">
        We detected this isn&apos;t your first time playing {asking.game.title}. Restore your playtime ({hoursAndMinutes(asking.offer.seconds)})?
      </p>
      <p className="text-xs leading-relaxed text-muted-foreground">
        {asking.offer.source === 'folder'
          ? 'The game folder remembers it from before.'
          : asking.offer.source === 'account'
            ? 'Your kryo.to account counted it, on this PC or another.'
            : 'The game folder and your kryo.to account both remember it; the larger count is used.'}{' '}
        The game starts either way. No means this game is not asked again.
      </p>
    </Modal>
  ) : null

  return { gate, modal }
}
