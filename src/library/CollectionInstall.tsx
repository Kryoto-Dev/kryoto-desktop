import { useEffect, useMemo, useState } from 'react'
import { AsciiBar, Button, Check, Modal } from '@/ui'
import { errorText, on } from '@/lib/bridge'
import { artSrc } from '@/lib/art'
import { adultBlur, useHideAdult, useShowAdult } from '@/lib/adult'
import { formatBytes } from '@/lib/downloads'
import { library, type LibraryGame } from '@/lib/library'
import { collections, installNeeds, onAskInstall, useCollections, type Collection, type CollectionGame, type CollectionUpdate } from '@/lib/collections'
import { useSettings } from '@/lib/settings'
import { usePromptTurn } from '@/lib/prompts'
import { FolderChoice, defaultFolder, useLibraryFolders } from '@/downloads/FolderChoice'
import { cn } from '@/lib/utils'

/**
 * Installing a kryo.to collection, and hearing when one gains a game.
 *
 * One dialog for both. Asked for (the sidebar, a collection's page in the
 * Store) it lists what this PC lacks; a collection that was installed and
 * gained games asks by itself, once, in its turn among the start-up prompts.
 * It shows the download and the room it takes first, and the games then join
 * the normal queue (collections.rs, downloads.rs `enqueue_collection`).
 */
export function CollectionInstall() {
  const state = useCollections()
  const [asked, setAsked] = useState<string | null>(null)
  useEffect(() => onAskInstall(setAsked), [])
  useEffect(() => {
    const off = on<string>('collection-install', setAsked)
    return () => void off.then((f) => f())
  }, [])
  const collection = asked ? (state.collections.find((c) => c.id === asked) ?? null) : null
  const update = state.updates[0] ?? null
  const turn = usePromptTurn('collection', !!update && !collection)

  if (collection) return <Dialog key={collection.id} collection={collection} installed={state.installed.includes(collection.id)} update={null} onClose={() => setAsked(null)} />
  if (!turn || !update) return null
  const owner = state.collections.find((c) => c.id === update.id)
  return owner ? <Dialog key={`${update.id}-update`} collection={owner} installed update={update} onClose={() => {}} /> : null
}

function Dialog({ collection, installed, update, onClose }: { collection: Collection; installed: boolean; update: CollectionUpdate | null; onClose: () => void }) {
  const settings = useSettings()
  const showAdult = useShowAdult()
  const hideAdult = useHideAdult()
  const [games, setGames] = useState<LibraryGame[] | null>(null)
  useEffect(() => {
    void library.list().then(setGames).catch(() => setGames([]))
  }, [])
  const onPc = useMemo(() => new Set((games ?? []).map((g) => g.slug).filter(Boolean)), [games])
  const offered = useMemo(() => (update ? update.added : collection.games.filter((g) => !onPc.has(g.slug))).filter((g) => !(hideAdult && g.nsfw)), [update, collection, onPc, hideAdult])
  const [unticked, setUnticked] = useState<Set<string>>(new Set())
  const picked = offered.filter((g) => !unticked.has(g.slug))
  const { folders, error: foldersError } = useLibraryFolders(true, collection.id)
  const [folder, setFolder] = useState<string | null>(null)
  const chosen = folder ?? defaultFolder(folders)?.path ?? null
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const needs = installNeeds(picked, !!settings?.extractInPlace)
  const room = folders?.find((f) => f.path === chosen) ?? null
  const short = !!room && room.total > 0 && room.free < needs.free
  const asks = !!settings?.askInstallFolder && (folders?.length ?? 0) > 1

  const run = (job: Promise<unknown>) => {
    setBusy(true)
    setError(null)
    job
      .then(onClose)
      .catch((e: unknown) => setError(errorText(e)))
      .finally(() => setBusy(false))
  }
  const install = () => run(collections.install(collection.id, picked.map((g) => g.slug), asks ? chosen : null))
  // Asked by itself: Later is the answer that stops it asking.
  const later = () => (update ? run(collections.later(collection.id)) : onClose())

  const n = offered.length
  const title = update ? `"${collection.name}" was updated` : `Install "${collection.name}"`
  const everything = !update && n === 0 && games !== null
  return (
    <Modal
      title={title}
      onClose={later}
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={later}>
            {update ? 'Later' : 'Cancel'}
          </Button>
          {everything ? (
            installed ? null : (
              <Button variant="primary" disabled={busy} onClick={install}>
                Watch for new games
              </Button>
            )
          ) : (
            <Button variant="primary" disabled={busy || picked.length === 0 || games === null} onClick={install}>
              {update ? 'Install now' : `Install ${picked.length} game${picked.length === 1 ? '' : 's'}`}
            </Button>
          )}
        </>
      }
    >
      {games === null ? <AsciiBar fraction={null} cells={24} showPct={false} className="text-muted-foreground" /> : null}
      {update ? (
        <p className="text-xs leading-relaxed text-muted-foreground">
          It has {n === 1 ? '1 extra game' : `${n} extra games`} since you installed it{needs.download ? `: ${formatBytes(needs.download)} to download` : ''}.
        </p>
      ) : everything ? (
        <p className="text-xs leading-relaxed text-muted-foreground">
          {collection.games.length ? 'Every game in it is on this PC.' : 'It has no games yet.'} {installed ? 'Kryoto will ask when it gains one.' : 'Kryoto can ask when it gains one.'}
        </p>
      ) : games !== null ? (
        <p className="text-xs leading-relaxed text-muted-foreground">
          {collection.games.length - n > 0 ? `${collection.games.length - n} of its ${collection.games.length} games are on this PC already. ` : ''}
          {collection.games.length - n > 0 ? 'The rest join' : 'They join'} your downloads, one at a time, and Kryoto will ask when the collection gains a game.
          {collection.role === 'visitor' ? ' Installing it also follows it.' : ''}
        </p>
      ) : null}
      {n > 0 ? (
        <ul className="grid max-h-64 min-h-[3.75rem] gap-1 overflow-auto">
          {offered.map((g) => (
            <Row
              key={g.slug}
              game={g}
              blur={!!adultBlur(g.nsfw, showAdult)}
              ticked={!unticked.has(g.slug)}
              onToggle={() =>
                setUnticked((s) => {
                  const next = new Set(s)
                  if (next.has(g.slug)) next.delete(g.slug)
                  else next.add(g.slug)
                  return next
                })
              }
            />
          ))}
        </ul>
      ) : null}
      {picked.length ? (
        <dl className="flex flex-wrap gap-x-6 gap-y-1 text-[10px] uppercase tracking-wider text-muted-foreground">
          <Figure label="Download" value={needs.download ? formatBytes(needs.download) : 'Not listed'} />
          <Figure label="Free space needed" value={needs.free ? `About ${formatBytes(needs.free)}` : 'Not listed'} warn={short} />
          {room?.total ? <Figure label={`Free on ${room.drive}`} value={formatBytes(room.free)} warn={short} /> : null}
        </dl>
      ) : null}
      {needs.unknown && picked.length ? <p className="text-[11px] text-muted-foreground">Size not listed for {needs.unknown === picked.length ? 'these games' : `${needs.unknown} of them`}, so they need more than this.</p> : null}
      {short ? <p className="text-[11px] text-warning">Not enough room for all of them. Each game checks when its turn comes, and the ones that do not fit stop with a message.</p> : null}
      {asks && folders && picked.length ? <FolderChoice folders={folders} picked={chosen ?? ''} onPick={setFolder} needs={needs.free} /> : null}
      {error || foldersError ? <p className="text-xs text-destructive">{error ?? foldersError}</p> : null}
    </Modal>
  )
}

function Figure({ label, value, warn = false }: { label: string; value: string; warn?: boolean }) {
  return (
    <div className="flex gap-1.5">
      <dt>{label}</dt>
      <dd className={cn('font-bold tabular-nums', warn ? 'text-warning' : 'text-foreground')}>{value}</dd>
    </div>
  )
}

function Row({ game, blur, ticked, onToggle }: { game: CollectionGame; blur: boolean; ticked: boolean; onToggle: () => void }) {
  const corners = { borderRadius: 'min(var(--kryo-radius), 4px)' }
  return (
    <li className={cn('kryo-radius flex items-center gap-3 border border-border p-2', !ticked && 'opacity-50')}>
      <Check checked={ticked} onChange={onToggle} label={<span className="sr-only">Install {game.title}</span>} />
      {game.cover ? (
        <img decoding="async" src={artSrc(game.cover) ?? undefined} alt="" className={cn('h-10 w-7 shrink-0 bg-secondary object-cover', blur && 'blur-[3px]')} style={corners} onError={(e) => (e.currentTarget.style.visibility = 'hidden')} />
      ) : (
        <span className="h-10 w-7 shrink-0 bg-secondary" style={corners} />
      )}
      <span className="min-w-0 grow truncate text-sm text-foreground">{game.title}</span>
      <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">{game.sizeBytes ? formatBytes(game.sizeBytes) : ''}</span>
    </li>
  )
}
