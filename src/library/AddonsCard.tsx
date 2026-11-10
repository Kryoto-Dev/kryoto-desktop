import { useEffect, useState } from 'react'
import { CheckCircle2, Download, ExternalLink, Globe, RotateCw, Undo2 } from '@/ui/pixel-icons'
import { AsciiBar, Button, Caption, Card, Label, Modal } from '@/ui'
import { errorText } from '@/lib/bridge'
import { fetchAddons, library, type KryoAddon, type LibraryGame } from '@/lib/library'
import { downloads, formatBytes, isActive, phaseOf, progressOf, useDownloads, type Download as Dl } from '@/lib/downloads'
import { GameRepair } from './GameRepair'

const isOnline = (a: { label: string | null; source: string | null }) => /online/i.test(`${a.label ?? ''} ${a.source ?? ''}`)
/** A DLC packed on its own, filed on kryo.to as `DLC: <name>` (kryo.to lib/dlc-addons.ts). */
const isDlc = (a: { label: string | null }) => /^dlc:\s*/i.test(a.label ?? '')
const dlcName = (label: string | null) => (label ?? '').replace(/^dlc:\s*/i, '').trim() || 'DLC'
/** More DLC than this and the list folds away behind a button. */
const DLC_SHOWN = 5
/** downloads.rs `ADDON_CHALLENGE`: kryo.to wants its own check for this account. */
const needsWindow = (error: string | null) => !!error && /download window/i.test(error)

/**
 * A game's Add-ons & DLC: what kryo.to has for it (its DLC, language packs,
 * the Online add-on), what is installed, and Kryoto Online set up on this PC.
 *
 * Add downloads it for this game and applies it to the game's folder, with
 * the progress in its row, and the row ends on Installed (addons.rs
 * `addons_get`: the link is minted for this PC, nothing opens and nothing asks
 * which game). Only when the app cannot fetch it itself (signed out, or a host
 * it cannot download from) does Add open the game's download window instead.
 * Remove deletes what the add-on wrote and puts back the game's files it
 * replaced (addons.rs), so a DLC taken out leaves the game as it was.
 */
export function AddonsCard({
  game,
  onGet,
  onChanged,
}: {
  game: LibraryGame
  /** The game's download window in the Store: the way when the app cannot fetch an add-on itself. */
  onGet: () => void
  onChanged: (g: LibraryGame) => void
}) {
  const [available, setAvailable] = useState<KryoAddon[] | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  // Add presses still being answered, by add-on id ('all' for Add all DLC): one never holds up another.
  const [adding, setAdding] = useState<ReadonlySet<string>>(new Set())
  const [error, setError] = useState<string | null>(null)
  const [repairOpen, setRepairOpen] = useState(false)
  const [allDlc, setAllDlc] = useState(false)
  const [confirm, setConfirm] = useState<{ kind: 'addon'; file: string; label: string; count: number; replaced?: number } | { kind: 'online' } | null>(null)
  const { list: rows } = useDownloads()

  useEffect(() => {
    setAvailable(null)
    if (!game.slug) return setAvailable([])
    let cancelled = false
    void fetchAddons(game.slug)
      .then((a) => !cancelled && setAvailable(a))
      .catch(() => !cancelled && setAvailable([]))
    return () => {
      cancelled = true
    }
  }, [game.slug])

  // Kryoto Online is offered where Steam says the game plays online and the
  // release has no way online of its own (online.rs, online_unneeded).
  // Unknown - offline, or kryo.to did not answer - counts as not offered.
  const [onlineOffered, setOnlineOffered] = useState(false)
  useEffect(() => {
    setOnlineOffered(false)
    if (!game.slug || game.online) return
    let cancelled = false
    void library
      .onlineCheck(game.id)
      .then((why) => !cancelled && setOnlineOffered(why === null))
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [game.id, game.slug, game.online])

  const applied = game.addons ?? []
  const appliedFiles = new Set(applied.map((a) => a.file.toLowerCase()))
  // A DLC also by its name: its file's name has changed since some were applied
  // (kryo.to names it `<Game> - DLC <Name> - kryoto`), its label has not.
  const appliedDlc = new Set(applied.filter(isDlc).map((a) => dlcName(a.label).toLowerCase()))
  const isApplied = (a: KryoAddon) =>
    a.links.some((l) => l.name && appliedFiles.has(l.name.toLowerCase())) || (isDlc(a) && appliedDlc.has(dlcName(a.label).toLowerCase()))
  // The Online add-on only where Kryoto Online is offered at all: Steam lists
  // online play and the release has no way online of its own. kryo.to says
  // so too (`offered`); either one saying no hides it.
  const wanted = (a: KryoAddon) => !isOnline(a) || (onlineOffered && a.offered !== false)
  const offers = (available ?? []).filter((a) => !isApplied(a) && wanted(a))
  const dlcOffers = offers.filter((a) => isDlc(a))
  const otherOffers = offers.filter((a) => !isDlc(a))
  const dlcApplied = applied.filter(isDlc)
  const otherApplied = applied.filter((a) => !isDlc(a))
  const undo = (a: (typeof applied)[number]) =>
    setConfirm({ kind: 'addon', file: a.file, label: isDlc(a) ? dlcName(a.label) : a.label, count: a.files.length, replaced: a.replaced?.length })
  const kryoOnline = offers.find(isOnline)
  const showOnline = !!game.online || onlineOffered

  // Each add-on's download, wherever it was started: from this card (by its
  // id on kryo.to) or from the Store's download window (by its label).
  const mine = rows.filter((d) => d.addon && d.slug === game.slug && (!d.gameId || d.gameId === game.id) && d.status !== 'canceled')
  const rowFor = (a: KryoAddon): Dl | undefined => {
    const label = (a.label ?? '').toLowerCase()
    return mine
      .filter((d) => d.addonRelease === String(a.id) || (!!label && d.addon?.toLowerCase() === label))
      .sort((x, y) => y.addedAt - x.addedAt)[0]
  }
  const going = (a: KryoAddon) => {
    const d = rowFor(a)
    return !!d && (isActive(d) || d.status === 'paused')
  }

  const run = async (key: string, job: () => Promise<LibraryGame>) => {
    setBusy(key)
    setError(null)
    try {
      onChanged(await job())
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(null)
    }
  }

  /** Add these add-ons to this game, here. */
  const add = async (key: string, list: KryoAddon[]) => {
    const ids = list.filter((a) => !going(a)).map((a) => String(a.id))
    if (!ids.length) return
    setAdding((s) => new Set(s).add(key))
    setError(null)
    try {
      const got = await library.addonsGet(game.id, ids)
      // Signed out, or a host only a browser reaches: the download window
      // still adds it to this game when it lands.
      if (got.store) onGet()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setAdding((s) => {
        const next = new Set(s)
        next.delete(key)
        return next
      })
    }
  }

  if (available === null) return null
  const nothing = offers.length === 0 && applied.length === 0 && !showOnline
  if (nothing) return null

  /** The right side of a row not installed yet: Add, its progress, or why it stopped. */
  const offerRow = (a: KryoAddon, title: string, sub: string | undefined, verb: string) => {
    const key = String(a.id)
    const d = rowFor(a)
    if (d && (isActive(d) || d.status === 'paused')) {
      const queued = d.status === 'queued'
      const text = d.status === 'paused' ? 'Paused in Downloads' : queued ? 'Waiting its turn in Downloads' : `${phaseOf(d)}${d.total && d.status === 'downloading' ? ` · ${formatBytes(d.received)} of ${formatBytes(d.total)}` : ''}`
      return (
        <Row
          key={key}
          title={title}
          sub={text}
          live
          action={
            d.status === 'paused' ? (
              <Button size="sm" onClick={() => void downloads.resume(d.id).catch((e) => setError(errorText(e)))}>
                <Download className="size-3" />
                Resume
              </Button>
            ) : (
              <AsciiBar fraction={queued || d.status === 'resolving' ? null : progressOf(d)} cells={12} className="text-muted-foreground" />
            )
          }
        />
      )
    }
    if (d?.status === 'installed') {
      // Landed; the library's own record follows a moment later.
      return <Row key={key} title={title} sub="Installed" action={<CheckCircle2 className="size-4 text-primary" aria-label="Installed" />} />
    }
    if (d?.status === 'failed') {
      return (
        <Row
          key={key}
          title={title}
          sub={d.error ?? 'It stopped.'}
          tone="error"
          action={
            needsWindow(d.error) ? (
              <Button size="sm" onClick={onGet}>
                <ExternalLink className="size-3" />
                Download window
              </Button>
            ) : (
              <Button size="sm" onClick={() => void downloads.resume(d.id).catch((e) => setError(errorText(e)))}>
                <RotateCw className="size-3" />
                Retry
              </Button>
            )
          }
        />
      )
    }
    return (
      <Row
        key={key}
        title={title}
        sub={sub}
        action={
          <Button size="sm" busy={adding.has(key) || adding.has('all')} onClick={() => void add(key, [a])}>
            <Download className="size-3" />
            {verb}
          </Button>
        }
      />
    )
  }

  const dlcIdle = dlcOffers.filter((a) => !going(a) && rowFor(a)?.status !== 'installed')

  return (
    <Card className="grid gap-4">
      <Label>{dlcOffers.length + dlcApplied.length > 0 ? 'Add-ons & DLC' : 'Add-ons'}</Label>

      {/* The game's DLC, each optional. Add (or Add all DLC) downloads each
          for this game and applies it as it lands; Remove takes it out again. */}
      {dlcOffers.length + dlcApplied.length > 0 ? (
        <div className="grid gap-2">
          <div className="flex items-center gap-3">
            <Caption className="grow">
              DLC · {[dlcApplied.length ? `${dlcApplied.length} installed` : '', dlcOffers.length ? `${dlcOffers.length} not installed` : ''].filter(Boolean).join(' · ')}
            </Caption>
            {dlcIdle.length > 1 ? (
              <Button size="sm" variant="primary" busy={adding.has('all')} onClick={() => void add('all', dlcIdle)}>
                <Download className="size-3" />
                Add all {dlcIdle.length} DLC
              </Button>
            ) : null}
          </div>
          {dlcApplied.map((a) => (
            <Row
              key={a.file}
              title={dlcName(a.label)}
              sub="Installed"
              action={
                <Button size="sm" variant="ghost" disabled={!!busy} onClick={() => undo(a)}>
                  <Undo2 className="size-3" />
                  Remove
                </Button>
              }
            />
          ))}
          {(allDlc ? dlcOffers : dlcOffers.slice(0, DLC_SHOWN)).map((a) => offerRow(a, dlcName(a.label), a.download_size ?? undefined, 'Add'))}
          {dlcOffers.length > DLC_SHOWN ? (
            <Button size="sm" variant="ghost" className="justify-self-start" onClick={() => setAllDlc((v) => !v)}>
              {allDlc ? 'Show fewer' : `Show ${dlcOffers.length - DLC_SHOWN} more DLC`}
            </Button>
          ) : null}
        </div>
      ) : null}

      {otherOffers.map((a) => offerRow(a, a.label || 'Add-on', [a.note, a.download_size].filter(Boolean).join(' · ') || undefined, 'Add'))}

      {otherApplied.map((a) => (
        <Row
          key={a.file}
          title={a.label}
          sub={`Installed · ${a.files.length} file${a.files.length === 1 ? '' : 's'}`}
          action={
            <Button size="sm" variant="ghost" disabled={!!busy} onClick={() => undo(a)}>
              <Undo2 className="size-3" />
              Undo
            </Button>
          }
        />
      ))}

      {showOnline ? (
        <Row
          title="Kryoto Online on this PC"
          sub={
            game.online
              ? `Set up · version ${game.online.version}`
              : kryoOnline
                ? 'Or use the Online add-on above - either works.'
                : 'Plays online through Steam, set up here instead of downloaded.'
          }
          action={
            game.online ? (
              <Button size="sm" variant="ghost" disabled={!!busy} onClick={() => setConfirm({ kind: 'online' })}>
                <Undo2 className="size-3" />
                Undo
              </Button>
            ) : (
              <Button size="sm" disabled={!!busy} onClick={() => setRepairOpen(true)}>
                <Globe className="size-3" />
                {busy === 'online' ? 'Setting up' : 'Set up'}
              </Button>
            )
          }
        />
      ) : null}

      {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
      {repairOpen ? <Modal title="Repair game" onClose={() => setRepairOpen(false)}><GameRepair gameId={game.id} initialOnline /></Modal> : null}

      {confirm ? (
        <Modal
          title={confirm.kind === 'online' ? 'Remove Kryoto Online' : `Remove ${confirm.label}`}
          onClose={() => setConfirm(null)}
          footer={
            <>
              <Button variant="ghost" onClick={() => setConfirm(null)}>
                Cancel
              </Button>
              <Button
                variant="danger"
                onClick={() => {
                  const c = confirm
                  setConfirm(null)
                  if (c.kind === 'online') void run('online', () => library.onlineUndo(game.id))
                  else void run(c.file, () => library.addonUndo(game.id, c.file))
                }}
              >
                Remove
              </Button>
            </>
          }
        >
          <p className="text-xs leading-relaxed text-muted-foreground">
            {confirm.kind === 'online'
              ? "Deletes Kryoto Online's files and puts back the game's own."
              : confirm.replaced === undefined
                ? // Applied before Kryoto kept the originals (0.3.0): nothing to put back.
                  `Deletes the ${confirm.count} file${confirm.count === 1 ? '' : 's'} it added. Files it replaced are not brought back; download the game again if it needs them.`
                : `Deletes the ${confirm.count} file${confirm.count === 1 ? '' : 's'} it added${confirm.replaced ? ` and puts back the ${confirm.replaced} game file${confirm.replaced === 1 ? '' : 's'} it replaced` : ''}. The game is left as it was before.`}
          </p>
        </Modal>
      ) : null}
    </Card>
  )
}

function Row({ title, sub, action, live = false, tone }: { title: string; sub?: string; action: React.ReactNode; live?: boolean; tone?: 'error' }) {
  return (
    <div data-addon-row={title} className="kryo-radius flex items-center gap-3 border border-border p-3">
      <span className="grid min-w-0 grow gap-0.5">
        <span className="truncate text-xs font-bold text-foreground">{title}</span>
        {sub ? (
          <Caption className={`normal-case tracking-normal ${tone === 'error' ? 'text-destructive' : ''}`}>
            <span aria-live={live ? 'polite' : undefined}>{sub}</span>
          </Caption>
        ) : null}
      </span>
      <span className="shrink-0">{action}</span>
    </div>
  )
}
