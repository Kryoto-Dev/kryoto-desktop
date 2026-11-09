import { useEffect, useState } from 'react'
import { CheckCircle2, Download, ExternalLink, RotateCw } from '@/ui/pixel-icons'
import { AsciiBar, Button, Section } from '@/ui'
import { errorText } from '@/lib/bridge'
import { fetchAddons, library, type KryoAddon } from '@/lib/library'
import { downloads, isActive, phaseOf, progressOf, useDownloads } from '@/lib/downloads'

/**
 * Properties > Repair: a fix kryo.to keeps for this game (a release of kind
 * `repair`, Online-Fix's generic fix packed by Kryoto Forge). Get it downloads
 * it for this game right here, like an add-on from the game's Add-ons & DLC
 * card (addons.rs `addons_get`), and the download runner applies it as a
 * repair (`install_fix`): checked against this game and copied with backups,
 * so Undo below puts back what it replaced. Only when the app cannot fetch it
 * itself does it open the game's download window.
 */
export function KryoFix({ gameId, slug, onGet }: { gameId: string; slug: string; onGet: () => void }) {
  const [fixes, setFixes] = useState<KryoAddon[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const { list: rows } = useDownloads()
  useEffect(() => {
    let cancelled = false
    void fetchAddons(slug)
      .then((all) => !cancelled && setFixes(all.filter((a) => a.kind === 'repair' && a.offered !== false)))
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [slug])
  if (fixes.length === 0) return null

  const get = async (f: KryoAddon) => {
    setBusy(true)
    setError(null)
    try {
      const got = await library.addonsGet(gameId, [String(f.id)])
      if (got.store) onGet()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Section
      title="From kryo.to"
      hint="A fix kryo.to keeps for this game. It replaces only the files it ships, keeps a backup of each, and Undo puts them back. Online-Fix needs Steam running and signed in."
    >
      {fixes.map((f) => {
        const d = rows
          .filter((r) => r.slug === slug && r.status !== 'canceled' && (r.addonRelease === String(f.id) || (!!f.label && r.addon === f.label)))
          .sort((x, y) => y.addedAt - x.addedAt)[0]
        const moving = !!d && isActive(d)
        return (
          <div key={String(f.id)} className="flex flex-wrap items-center gap-3">
            <span className="grow text-xs">
              <b className="text-foreground">{f.label ?? 'Fix'}</b>
              {f.download_size ? <span className="text-muted-foreground"> · {f.download_size}</span> : null}
              {moving ? <span className="block text-muted-foreground" aria-live="polite">{d.status === 'queued' ? 'Waiting its turn in Downloads' : phaseOf(d)}</span> : null}
              {d?.status === 'installed' ? <span className="block text-muted-foreground">Applied. Undo below puts the game's own files back.</span> : null}
              {d?.status === 'failed' && d.error ? <span className="block text-destructive">{d.error}</span> : null}
              {!moving && f.note && d?.status !== 'failed' ? <span className="block text-muted-foreground">{f.note}</span> : null}
            </span>
            {moving ? (
              <AsciiBar fraction={d.status === 'queued' || d.status === 'resolving' ? null : progressOf(d)} cells={12} className="text-muted-foreground" />
            ) : d?.status === 'installed' ? (
              <CheckCircle2 className="size-4 text-primary" aria-label="Applied" />
            ) : d?.status === 'failed' ? (
              /download window/i.test(d.error ?? '') ? (
                <Button size="sm" variant="outline" onClick={onGet}>
                  <ExternalLink className="size-3.5" />
                  Download window
                </Button>
              ) : (
                <Button size="sm" variant="outline" onClick={() => void downloads.resume(d.id).catch((e) => setError(errorText(e)))}>
                  <RotateCw className="size-3.5" />
                  Retry
                </Button>
              )
            ) : (
              <Button size="sm" variant="outline" busy={busy} onClick={() => void get(f)}>
                <Download className="size-3.5" />
                Get it
              </Button>
            )}
          </div>
        )
      })}
      {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
    </Section>
  )
}
