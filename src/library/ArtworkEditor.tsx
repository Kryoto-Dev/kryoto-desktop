import { useEffect, useRef, useState } from 'react'
import { ImagePlus, Search } from 'lucide-react'
import { Button, Section, inputCls } from '@/ui'
import { errorText, isTauri } from '@/lib/bridge'
import { pickPath } from '@/lib/pick'
import { artSrc } from '@/lib/art'
import { coverFor, fetchArtwork, heroFor, importArtwork, searchArtwork, titleKey, type Artwork, type ArtMatch } from '@/lib/game-artwork'
import { Art } from '@/library/Art'

export function ArtworkEditor({ title, value, adult = false, onChange, onBusy }: {
  title: string
  value: Artwork
  adult?: boolean
  onChange: (art: Partial<Artwork>) => void
  onBusy?: (busy: boolean) => void
}) {
  const [query, setQuery] = useState(title)
  const [matches, setMatches] = useState<ArtMatch[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const active = useRef<AbortController | null>(null)
  useEffect(() => { setQuery(title); setMatches(null); setNote(null) }, [title])
  useEffect(() => { onBusy?.(busy) }, [busy, onBusy])
  useEffect(() => () => { active.current?.abort(); onBusy?.(false) }, [onBusy])

  async function useSteam(match: ArtMatch, signal?: AbortSignal) {
    const art = await fetchArtwork(match.appid, signal)
    signal?.throwIfAborted()
    onChange({ ...art, customCover: null, customHero: null })
    setMatches(null)
    setNote(`Using Steam artwork for ${match.title}.`)
  }

  async function run(work: (signal: AbortSignal) => Promise<void>) {
    active.current?.abort()
    const controller = new AbortController()
    active.current = controller
    setBusy(true); setError(null); setNote(null)
    try { await work(controller.signal) }
    catch (e) { if (!controller.signal.aborted) setError(errorText(e)) }
    finally { if (!controller.signal.aborted) setBusy(false) }
  }

  async function find(signal: AbortSignal) {
    const results = await searchArtwork(query, signal)
    const exact = results.filter((m) => titleKey(m.title) === titleKey(query))
    const match = exact.length === 1 ? exact[0] : undefined
    if (match) await useSteam(match, signal)
    else {
      setMatches(results)
      if (!results.length) setNote('No Steam match. Try another name or choose your own pictures.')
    }
  }

  async function choose(slot: 'customCover' | 'customHero') {
    await run(async (signal) => {
      const path = await pickPath({ title: slot === 'customCover' ? 'Choose a cover picture' : 'Choose a banner picture', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'avif'] })
      if (!path || signal.aborted) return
      const uri = await importArtwork(path)
      if (signal.aborted) return
      onChange({ [slot]: uri })
      setNote('Picture copied to Kryoto. You can move or delete the original.')
    })
  }

  return (
    <Section title="Artwork" hint="Choose Steam artwork or your own pictures. Custom pictures are copied into Kryoto's app storage and kept when the artwork cache is cleared.">
      <div className="grid grid-cols-[7rem_1fr] items-start gap-4">
        {([['customCover', 'Cover', coverFor(value)], ['customHero', 'Banner', heroFor(value)]] as const).map(([slot, label, src]) => (
          <div key={slot} className="grid min-w-0 gap-2">
            <Art src={src} fallback={slot === 'customCover' ? [value.cover, value.header] : [value.hero, value.header, coverFor(value)]} title={label} adult={adult} className={slot === 'customCover' ? 'aspect-[2/3] w-full object-cover' : 'aspect-[96/31] w-full object-cover'} />
            <span className="text-xs font-bold text-foreground">{label}</span>
            <Button size="sm" disabled={busy || !isTauri()} aria-label={`Choose ${label.toLowerCase()} picture`} onClick={() => void choose(slot)}>
              <ImagePlus className="size-3.5" /> Choose picture
            </Button>
            {value[slot] ? <Button size="sm" variant="ghost" disabled={busy} onClick={() => onChange({ [slot]: null })}>Use default {label.toLowerCase()}</Button> : null}
          </div>
        ))}
      </div>
      <div className="flex gap-2">
        <input aria-label="Game name for Steam artwork" className={inputCls} value={query} maxLength={120} placeholder="Game name on Steam" disabled={busy} onChange={(e) => setQuery(e.target.value)} />
        <Button disabled={busy || query.trim().length < 2} onClick={() => void run(find)}><Search className="size-3.5" /> Find art</Button>
      </div>
      {matches?.length ? <ul className="grid gap-1">
        {matches.map((match) => <li key={match.appid}><button type="button" disabled={busy} onClick={() => void run((signal) => useSteam(match, signal))} className="kryo-radius flex w-full items-center gap-3 p-2 text-left text-xs text-foreground hover:bg-secondary disabled:opacity-40">
          {match.cover ? <img src={artSrc(match.cover) ?? undefined} alt="" className="h-10 w-20 object-cover blur-sm" /> : null}
          <span>{match.title}</span>
        </button></li>)}
      </ul> : null}
      {busy ? <p role="status" className="text-xs text-muted-foreground">Loading artwork...</p> : null}
      {note ? <p role="status" className="text-xs text-muted-foreground">{note}</p> : null}
      {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
      {!isTauri() ? <p className="text-xs text-muted-foreground">Choose local pictures in the desktop app.</p> : null}
    </Section>
  )
}
