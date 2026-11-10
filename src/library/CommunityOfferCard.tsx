import { useCallback, useEffect, useState } from 'react'
import { Camera, Check as CheckIcon, CloudUpload, Download, ImagePlus } from '@/ui/pixel-icons'
import { AsciiBar, Button, Caption, Card, Check, Label } from '@/ui'
import { call, errorText, on } from '@/lib/bridge'
import { pickPath } from '@/lib/pick'
import type { LibraryGame } from '@/lib/library'
import { CommunityBuildInfo } from './CommunityBuildInfo'

/** src-tauri community_offer.rs `OfferState`. */
type OfferState = {
  shown: boolean
  toVersion: string
  kryotoVersion: string | null
  playedSeconds: number
  needSeconds: number
  exitCode: number | null
  shots: string[]
  autoShots: boolean
  offerId: string | null
  offerState: string | null
  reasons: string[]
}

type Progress = { id: string; stage: string; done: number; total: number }

/** src-tauri community_update.rs `UpdateCheck`. */
type UpdateCheck = { available: boolean; steamBuildId: string | null; kryotoBuildId: string | null; reason: string | null }

const OFFER_STATE: Record<string, string> = {
  offered: 'Offered, waiting for the upload.',
  uploading: 'Uploading.',
  review: 'Sent. Staff will look at it; you will get a notification either way.',
  rejected_auto: 'kryo.to turned it away.',
  approved: 'Accepted, and live on kryo.to, credited to you.',
  rejected: 'Staff turned it down.',
  withdrawn: 'Withdrawn.',
}

function minutes(s: number) {
  return `${Math.floor(s / 60)} min`
}

/**
 * Community updates on a game's page (kryo.to flag `community_updates`), in
 * two steps:
 *   - "Your copy can be updated: try it": Steam has a newer build than
 *     Kryoto's release of this gbe_fork game; Kryoto updates this copy
 *     (src-tauri community_update.rs).
 *   - "Your copy is newer than Kryoto's: offer it": after that update; the
 *     playtest, the screenshot and both confirmations come before the button
 *     does anything (community_offer.rs).
 */
export function CommunityOfferCard({ game, running = false }: { game: LibraryGame; running?: boolean }) {
  const [state, setState] = useState<OfferState | null>(null)
  const [works, setWorks] = useState(false)
  const [version, setVersion] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [progress, setProgress] = useState<Progress | null>(null)
  const [check, setCheck] = useState<UpdateCheck | null>(null)

  const load = useCallback(() => {
    call<OfferState>('community_offer_state', { id: game.id }).then(
      (s) => {
        setState(s)
        if (!s.shown) call<UpdateCheck>('community_update_check', { id: game.id }).then(setCheck, () => setCheck(null))
      },
      () => setState(null),
    )
  }, [game.id])
  // Again after each session: the playtest and the screenshots change then.
  useEffect(() => {
    if (!running) load()
  }, [load, running])
  useEffect(() => {
    const off = on<Progress>('community-offer', (p) => {
      if (p.id === game.id) setProgress(p)
    })
    return () => {
      void off.then((f) => f())
    }
  }, [game.id])

  async function tryUpdate() {
    setBusy(true)
    setError(null)
    try {
      setState(await call<OfferState>('community_update_try', { id: game.id }))
      setCheck(null)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
      setProgress(null)
    }
  }

  if (!state?.shown) {
    if (!check?.available) return null
    return (
      <Card className="grid gap-3">
        <div className="flex items-center justify-between gap-2">
          <Label>Your copy can be updated</Label>
          <Download className="size-4 text-primary" aria-hidden />
        </div>
        <p className="text-xs leading-relaxed text-muted-foreground">
          Steam has a newer build{check.steamBuildId ? <> ({check.steamBuildId})</> : null} than Kryoto&apos;s
          {check.kryotoBuildId ? <> ({check.kryotoBuildId})</> : null}. Try it: Kryoto updates this copy from Steam, keeps its
          emulator, and lets you undo it. If it works, you can offer it to everyone.
        </p>
        <CommunityBuildInfo />
        {progress ? (
          <div className="grid gap-1">
            <Caption>{progress.stage}</Caption>
            {progress.total ? <AsciiBar fraction={progress.done / progress.total} /> : null}
          </div>
        ) : null}
        <Button variant="primary" disabled={busy || running} onClick={() => tryUpdate()}>
          <Download className="size-3.5" /> {busy ? 'Updating…' : 'Try it'}
        </Button>
        {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
      </Card>
    )
  }
  const played = state.playedSeconds >= state.needSeconds
  const cleanExit = state.exitCode == null || state.exitCode === 0
  const hasShot = state.shots.length > 0
  const sent = state.offerId != null

  async function addShot() {
    setError(null)
    try {
      const path = await pickPath({ title: 'Choose a screenshot of the game running', extensions: ['png', 'jpg', 'jpeg'] })
      if (typeof path === 'string' && path) setState(await call<OfferState>('community_offer_add_shot', { id: game.id, path }))
    } catch (e) {
      setError(errorText(e))
    }
  }

  async function send() {
    setBusy(true)
    setError(null)
    try {
      setState(await call<OfferState>('community_offer_send', { id: game.id, confirmedWorks: works, confirmedVersion: version }))
    } catch (e) {
      setError(errorText(e))
      // The offer may exist by now: show it, with the way to send it again.
      load()
    } finally {
      setBusy(false)
      setProgress(null)
    }
  }

  // Stops a send under way, or takes back an offer that is waiting.
  async function cancel() {
    setError(null)
    try {
      setState(await call<OfferState>('community_offer_cancel', { id: game.id }))
    } catch (e) {
      setError(errorText(e))
    }
  }

  const bar = progress ? (
    <div className="grid gap-1">
      <Caption>{progress.stage}</Caption>
      {progress.total ? <AsciiBar fraction={progress.done / progress.total} /> : null}
    </div>
  ) : null
  const unfinished = state.offerState === 'offered' || state.offerState === 'uploading'

  return (
    <Card className="grid gap-3">
      <div className="flex items-center justify-between gap-2">
        <Label>Your copy is newer than Kryoto&apos;s</Label>
        <CloudUpload className="size-4 text-primary" aria-hidden />
      </div>
      <p className="text-xs leading-relaxed text-muted-foreground">
        It is {state.toVersion || 'a newer build'}
        {state.kryotoVersion ? <>; kryo.to has {state.kryotoVersion}</> : null}. You can offer it, so everyone gets it. It is opt-in, and
        Kryoto checks every file first.
      </p>
      <CommunityBuildInfo />

      {sent ? (
        <div className="grid gap-1.5">
          <p className="text-xs text-foreground">{OFFER_STATE[state.offerState ?? ''] ?? 'Sent.'}</p>
          {state.reasons.length ? (
            <ul className="grid list-disc gap-1 pl-4 text-[11px] text-destructive">
              {state.reasons.map((r) => <li key={r}>{r}</li>)}
            </ul>
          ) : null}
          {bar}
          {unfinished && !busy ? (
            <Button variant="primary" onClick={() => send()}>
              <CloudUpload className="size-3.5" /> Send the upload again
            </Button>
          ) : null}
          {unfinished || state.offerState === 'review' ? (
            <Button variant="outline" onClick={() => cancel()}>
              {busy ? 'Cancel the upload' : 'Withdraw the offer'}
            </Button>
          ) : null}
        </div>
      ) : (
        <>
          <ul className="grid gap-1.5 text-xs">
            <li className="flex items-start gap-2">
              <CheckIcon className={played && cleanExit ? 'mt-0.5 size-3.5 text-success' : 'mt-0.5 size-3.5 text-muted-foreground'} aria-hidden />
              <span>
                Play it for {minutes(state.needSeconds)}: {minutes(state.playedSeconds)} so far
                {!cleanExit ? <span className="text-destructive">, and it last ended with an error (code {state.exitCode})</span> : null}.
              </span>
            </li>
            <li className="flex items-start gap-2">
              <Camera className={hasShot ? 'mt-0.5 size-3.5 text-success' : 'mt-0.5 size-3.5 text-muted-foreground'} aria-hidden />
              <span className="grid gap-1">
                {hasShot
                  ? `${state.shots.length} screenshot${state.shots.length === 1 ? '' : 's'} of it running.`
                  : state.autoShots
                    ? 'A screenshot of it running: Kryoto takes one while you play.'
                    : 'A screenshot of it running, taken while you play.'}
                {!state.autoShots || (!hasShot && played) ? (
                  <Button size="sm" variant="outline" className="justify-self-start" onClick={() => addShot()}>
                    <ImagePlus className="size-3.5" /> Add a screenshot
                  </Button>
                ) : null}
              </span>
            </li>
          </ul>
          <div className="grid gap-1.5">
            <Check checked={works} onChange={setWorks} label="The game works on this build" />
            <Check checked={version} onChange={setVersion} label={`It really is ${state.toVersion || 'the newer build'}`} />
          </div>
          {state.reasons.length ? (
            <ul className="grid list-disc gap-1 pl-4 text-[11px] text-destructive">
              {state.reasons.map((r) => <li key={r}>{r}</li>)}
            </ul>
          ) : null}
          {bar}
          <Button variant="primary" disabled={busy || running || !played || !cleanExit || !hasShot || !works || !version} onClick={() => send()}>
            <CloudUpload className="size-3.5" /> {busy ? 'Offering…' : 'Offer it to Kryoto'}
          </Button>
          {busy ? (
            <Button variant="outline" onClick={() => cancel()}>
              Cancel the upload
            </Button>
          ) : null}
        </>
      )}
      {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
    </Card>
  )
}
