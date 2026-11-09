import { useEffect, useRef, useState } from 'react'
import { gameContext } from './context'
import { ChevronDown, Download, FolderOpen, Glasses, Globe, History, NotebookPen, Play, Settings2, Square, X } from '@/ui/pixel-icons'
import { AsciiBar, Button, Caption, Card, CommandLine, Fact, IconButton, iconButtonCls, Floating, Label, MenuButton, MenuList, Stat, useDismiss, type MenuEntry } from '@/ui'
import { fetchCatalogGame, hasChoice, knowsBuild, latestFor, library, NEW_UPLOAD, normalEntry, playTarget, startFileTypes, vrKind, waysToPlay, withExecutable, type CatalogGame, type LibraryGame, type UpdateBackup } from '@/lib/library'
import { VrTag } from '@/library/VrTag'
import { isActive, phaseOf, progressOf, type Download as DownloadItem } from '@/lib/downloads'
import { pickPath } from '@/lib/pick'
import { useOnline } from '@/lib/online'
import { useSettled } from '@/hooks/useSettled'
import { useLivePlaytime } from '@/hooks/useLivePlaytime'
import { GameBanner } from '@/library/Art'
import { formatLastPlayed, formatPlaytime } from '@/lib/format'
import { errorText, isTauri } from '@/lib/bridge'
import { cn } from '@/lib/utils'
import { AddonsCard } from '@/library/AddonsCard'
import { AntivirusCard } from '@/library/AntivirusCard'
import { AchievementsCard } from '@/library/AchievementsCard'
import { CloudSavesCard } from '@/library/CloudSavesCard'
import { CommunityOfferCard } from '@/library/CommunityOfferCard'
import { CompatibilityFacts } from '@/library/Compatibility'
import { LibraryWriteups } from '@/library/Writeups'
import { STATUSES, STATUS_LABEL, type SavedStatus } from '@/hooks/useSaved'
import { automaticArtwork } from '@/lib/game-artwork'
import { BranchTag } from '@/library/BranchTag'
import { launcherName } from '@/lib/launchers'
import { DiscordChip } from '@/library/DiscordChip'
import { SourceSplitButton, useReleases } from '@/library/Versions'

/** The library's own words for a game whose folder has gone: Locate and Reinstall are the answer, not Repair. */
export function isMissingFolderError(error: string | null): boolean {
  return !!error && (/folder cannot be found/i.test(error) || /is not there any more/i.test(error))
}

/** A kryo.to game's place on the account's list, on the bar of its page. */
export function ListStatusMenu({ status, onSet }: { status: SavedStatus | null | undefined; onSet: (status: SavedStatus | null) => void }) {
  return (
    <MenuButton
      className="kryo-square grid gap-1 text-left"
      trigger={
        <>
          <Caption>Track</Caption>
          <span className="flex items-center gap-1 text-sm text-foreground">
            {status ? STATUS_LABEL[status] : 'Not tracked'}
            <ChevronDown className="size-3 text-muted-foreground" />
          </span>
        </>
      }
      items={[
        ...STATUSES.map((st) => ({ label: STATUS_LABEL[st], checked: status === st, hint: status === st ? 'now' : undefined, onSelect: () => onSet(st) })),
        ...(status ? [{ separator: true } as const, { label: 'Take off my list', onSelect: () => onSet(null) }] : []),
      ]}
    />
  )
}

/**
 * One game. Its art across the top fading into the page, the logo on it, a
 * white PLAY pill (a blinking RUNNING and STOP while it runs) with the ways to
 * play one press away, time played - and under it what kryo.to says, every way
 * it starts, and the exact command Play will run, which Steam never shows.
 *
 * Checks kryo.to for a newer build on open, and says so the way Steam does.
 */
export function GamePage({
  game,
  running,
  error,
  onDismissError,
  onRepair,
  onPlay,
  onPlayEntry,
  onPlayVr,
  onStop,
  gearItems,
  onStorePage,
  onWeb,
  onGetUpdate,
  download = null,
  onDownloads,
  onLatest,
  savedStatus,
  onSetStatus,
  onGameChanged,
  onContext,
}: {
  game: LibraryGame
  running: boolean
  error: string | null
  onDismissError: () => void
  onRepair: () => void
  onPlay: () => void
  /** `entry` null is the game's own exe. */
  onPlayEntry: (entry: number | null) => void
  /** "Play in VR" for a game that also plays flat. */
  onPlayVr?: () => void
  onStop: () => void
  gearItems: MenuEntry[]
  onStorePage: (() => void) | null
  /** Opens a kryo.to page (a path) in the Store view. */
  onWeb?: (path: string) => void
  onGetUpdate: () => void
  /** This game's download (an update on its way), when one is going. */
  download?: DownloadItem | null
  onDownloads?: () => void
  /** kryo.to answered what its current build is (null: it has none). */
  onLatest?: (slug: string, latest: string | null) => void
  /** Its status in the account's kryo.to library, when it is a kryo.to game. */
  savedStatus?: SavedStatus | null
  onSetStatus?: (status: SavedStatus | null) => void
  onGameChanged?: (g: LibraryGame) => void
  onContext?: (x: number, y: number) => void
}) {
  const [latest, setLatest] = useState<string | null>(null)
  const [exeBusy, setExeBusy] = useState(false)
  const [exeError, setExeError] = useState<string | null>(null)

  // Said on the page, before Play has to find out.
  const [missing, setMissing] = useState(false)
  useEffect(() => {
    let off = false
    setMissing(false)
    if (!game.installDir || game.launcher) return
    library.folderExists(game.installDir).then((there) => !off && setMissing(!there)).catch(() => {})
    return () => {
      off = true
    }
  }, [game.installDir, game.launcher])

  /** `locate`: the game moved, and only where it is changes (its play time stays). */
  async function changeExe(locate = false) {
    setExeError(null)
    const picked = await pickPath({ title: `Choose what starts ${game.title}`, defaultPath: game.installDir, extensions: startFileTypes() }).catch((e: unknown) => {
      setExeError(`The file picker did not open: ${errorText(e)}`)
      return null
    })
    if (typeof picked !== 'string') return
    setExeBusy(true)
    try {
      onGameChanged?.(await (locate ? library.relocate(game.id, picked) : library.save(withExecutable(game, picked))))
      if (locate) onDismissError()
    } catch (e) {
      setExeError(errorText(e))
    } finally {
      setExeBusy(false)
    }
  }

  /** "Keep my files", or going back on it. */
  async function keepFiles(keep: boolean) {
    setExeError(null)
    try {
      onGameChanged?.(await library.save({ ...game, keepFiles: keep }))
      onDismissError()
    } catch (e) {
      setExeError(errorText(e))
    }
  }
  const [command, setCommand] = useState<string>('')
  const [modesOpen, setModesOpen] = useState(false)
  const modesRef = useRef<HTMLDivElement | null>(null)
  useDismiss(modesOpen, modesRef, () => setModesOpen(false))

  // What kryo.to says about it now: a newer build, and its current art (the
  // real banner and logo). New art is saved with the game, so it shows
  // straight away next time, and offline.
  const [catalog, setCatalog] = useState<CatalogGame | null>(null)
  const [asked, setAsked] = useState(false)
  const online = useOnline()
  const latestRef = useRef(onLatest)
  latestRef.current = onLatest
  useEffect(() => {
    setCatalog(null)
    setLatest(null)
    setAsked(false)
    if (!online) return setAsked(true)
    let cancelled = false
    if (!game.slug) {
      if (game.cover && game.hero) return setAsked(true)
      void automaticArtwork(game.title).then((art) => {
        if (!cancelled && art) void library.refreshArtwork(game.id, { ...art, cover: game.cover ?? art.cover, hero: game.hero ?? art.hero }).then((g) => onGameChanged?.(g)).catch(() => {})
      }).finally(() => !cancelled && setAsked(true))
      return () => { cancelled = true }
    }
    fetchCatalogGame(game.slug)
      .then((c) => {
        if (cancelled) return
        setCatalog(c)
        setLatest(c.version)
        if (game.slug) latestRef.current?.(game.slug, c.version)
        // The same number (or none): the file decides, and asking costs a request.
        if (game.buildSha && !(c.version && c.version !== (game.version ?? ''))) {
          void latestFor(game, c.version).then((l) => {
            if (cancelled || l === c.version) return
            setLatest(l)
            if (game.slug) latestRef.current?.(game.slug, l)
          })
        }
        // The VR flag kept with the game, so a VR-only one starts its headset
        // even when kryo.to cannot be asked (an older install has none).
        if ((c.vr ?? null) !== (game.vr ?? null)) {
          void library.save({ ...game, vr: c.vr }).then((g) => onGameChanged?.(g)).catch(() => {})
        }
        const art = { hero: c.hero, logo: c.logo, header: c.header, cover: game.cover ?? c.cover }
        if (art.hero !== game.hero || art.logo !== (game.logo ?? null) || art.header !== (game.header ?? null) || art.cover !== game.cover) {
          void library.refreshArtwork(game.id, art).then((g) => onGameChanged?.(g)).catch(() => {})
        }
      })
      .catch(() => {})
      .finally(() => !cancelled && setAsked(true))
    return () => {
      cancelled = true
    }
    // Once per game and connection: saving the art must not fetch again.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [game.id, game.slug, online])

  const playtime = useLivePlaytime(game, running)
  // Under the bar the page is banners and cards that each ask for their own
  // data: shown together, so none of them lands on top of the others.
  const settled = useSettled(game.id, !asked)

  const target = playTarget(game)
  const previewEntry = target === 'ask' ? normalEntry(game) : target
  useEffect(() => {
    library
      .preview(game, previewEntry)
      .then(setCommand)
      .catch((e: unknown) => setCommand(errorText(e)))
  }, [game, previewEntry])

  const ways = waysToPlay(game)
  // VR, as kryo.to marks it (Steam's categories). A game that also plays
  // flat gets "Play in VR" beside Play when Steam lists no VR entry of its
  // own; a VR-only game's Play is the VR start.
  const vr = vrKind(catalog?.vr ?? game.vr)
  // Play can take a moment (a look at the account, saves to fetch) before the
  // game counts as running: it shows it heard, for that long and no longer.
  const [starting, setStarting] = useState(false)
  useEffect(() => {
    if (!starting) return
    if (running) return setStarting(false)
    const t = window.setTimeout(() => setStarting(false), 8000)
    return () => window.clearTimeout(t)
  }, [starting, running])
  const offerVr = vr === 'vr_supported' && !ways.some((w) => w.vr) && !!onPlayVr
  const choice = hasChoice(game) || offerVr
  // A build kept under Properties > Builds is not offered updates.
  const kept = !!game.pinnedVersion && game.pinnedVersion === game.version
  const updateAvailable = !!latest && knowsBuild(game) && latest !== (game.version ?? '') && !kept && !game.keepFiles
  const reuploaded = !!latest?.endsWith(NEW_UPLOAD)
  // The update's sources, for the same picker as Install (only asked for once there is one).
  const { releases } = useReleases(updateAvailable ? game.slug ?? null : null)
  const [updateError, setUpdateError] = useState<string | null>(null)
  // The update on its way: the banner gives way to its progress, as the
  // page of a game not yet on this PC does (CatalogGamePage).
  const going = download && (isActive(download) || download.status === 'paused') ? download : null
  const folderGone = missing || isMissingFolderError(error)
  const problem = error ?? (missing ? `Kryoto cannot find ${game.title} in its folder any more.` : null)
  // Left over from a repair: the player may simply want the files as they are.
  const repairStuck = !!error && /a repair was interrupted/i.test(error)

  return (
    <section aria-label={game.title} className="min-h-0 grow overflow-auto" {...gameContext(onContext)}>
      <GameBanner
        title={game.title}
        adult={game.nsfw}
        caption={
          (game.steamBranch && game.steamBranch !== 'public') || vr || game.launcher ? (
            <span className="flex gap-1.5 justify-self-start">
              <BranchTag branch={game.steamBranch} launcher={game.launcher} />
              <VrTag vr={vr} />
            </span>
          ) : undefined
        }
        // Art saved before kryo.to resolved it (no header on record) waits for
        // the answer, so the banner does not show a guess and then swap.
        pending={!game.customHero && !asked && game.header === undefined}
        banners={[game.customHero, catalog?.hero ?? game.hero, ...(catalog?.screenshots.slice(0, 1) ?? []), catalog?.header ?? game.header, game.customCover, game.cover]}
        logo={game.customHero ? null : catalog ? catalog.logo : game.logo}
      />

      <div className="kryo-radius relative z-10 mx-6 -mt-2 flex flex-wrap items-center gap-6 border border-border bg-card/90 p-4 backdrop-blur">
        {running ? (
          <div className="flex items-center gap-3">
            <Button variant="outline" size="lg" onClick={onStop} className="border-foreground text-foreground">
              <Square className="size-3.5 fill-current" />
              Stop
            </Button>
            <span className="flex items-center gap-2 text-[10px] font-bold uppercase tracking-[0.25em] text-success">
              <span className="kryo-blink size-2 rounded-full bg-success" />
              Running
            </span>
          </div>
        ) : (
          <div ref={modesRef} className="relative flex">
            <Button
              variant="primary"
              size="lg"
              busy={starting}
              onClick={() => {
                setStarting(true)
                onPlay()
              }}
              className={cn(choice && 'pr-6')}
              style={choice ? { borderTopRightRadius: 0, borderBottomRightRadius: 0 } : undefined}
            >
              {vr === 'vr_only' ? <Glasses className="size-4" /> : <Play className="size-4 fill-current" />}
              {vr === 'vr_only' ? 'Play in VR' : 'Play'}
            </Button>
            {choice ? (
              <>
                <button
                  type="button"
                  aria-label="Ways to play"
                  aria-expanded={modesOpen}
                  onClick={() => setModesOpen((o) => !o)}
                  className="kryo-square grid w-11 place-items-center border-l border-primary-foreground/20 bg-primary text-primary-foreground transition-opacity hover:opacity-90"
                  style={{ borderTopRightRadius: 'var(--kryo-radius-pill)', borderBottomRightRadius: 'var(--kryo-radius-pill)' }}
                >
                  <ChevronDown className="size-4" />
                </button>
                {modesOpen ? (
                  <Floating role="menu" anchor={() => modesRef.current?.getBoundingClientRect()} gap={8} className="kryo-pop kryo-radius min-w-72 overflow-y-auto overflow-x-hidden border border-border bg-popover py-1 shadow-2xl shadow-black/60">
                    <MenuList
                      onDone={() => setModesOpen(false)}
                      items={[
                        { heading: 'Play as' },
                        ...ways.map((w) => ({
                          label: w.label,
                          icon: w.vr ? <Glasses /> : <Play />,
                          hint: target === w.entry ? 'default' : w.args || undefined,
                          onSelect: () => onPlayEntry(w.entry),
                        })),
                        ...(offerVr ? [{ label: 'Play in VR', icon: <Glasses />, hint: 'starts SteamVR first', onSelect: () => onPlayVr?.() }] : []),
                      ]}
                    />
                  </Floating>
                ) : null}
              </>
            ) : null}
          </div>
        )}

        <dl className="flex gap-8">
          <Stat label="Last played" value={running ? 'Now' : formatLastPlayed(game.lastPlayed)} />
          <Stat label="Play time" value={formatPlaytime(playtime)} />
          {choice ? (
            <Stat
              label="Starts as"
              value={
                target === 'ask'
                  ? 'Asks each time'
                  : ways.find((w) => w.entry === target)?.label ?? 'Play'
              }
            />
          ) : null}
        </dl>

        <DiscordChip id={game.id} />

        {onSetStatus && game.slug ? <ListStatusMenu status={savedStatus} onSet={onSetStatus} /> : null}

        <div className="ml-auto flex gap-2">
          {/* What you thought of it, on its kryo.to page: the Store opens with the editor. */}
          {onWeb && game.slug ? (
            <IconButton label="Write it up" onClick={() => onWeb(`/game/${encodeURIComponent(game.slug!)}#write-up`)}>
              <NotebookPen className="size-4" />
            </IconButton>
          ) : null}
          {onStorePage ? (
            <IconButton label="Store page" onClick={onStorePage}>
              <Globe className="size-4" />
            </IconButton>
          ) : null}
          <IconButton label="Open folder" onClick={() => void library.openFolder(game.installDir)}>
            <FolderOpen className="size-4" />
          </IconButton>
          <MenuButton label="Manage" align="right" className={iconButtonCls} trigger={<Settings2 className="size-4" />} items={gearItems} />
        </div>
      </div>

      <div className={cn('transition-opacity duration-150', !settled && 'opacity-0')}>
      {going ? (
        <Banner>
          <Download className="size-4 shrink-0" />
          <button type="button" onClick={onDownloads} className="kryo-square grid grow gap-1.5 text-left" title="Open Downloads">
            <Caption>
              {going.status === 'paused' ? 'Paused' : phaseOf(going)}
              {going.meta.version ? ` build ${going.meta.version}` : ''}
            </Caption>
            <AsciiBar fraction={progressOf(going)} cells={24} />
          </button>
        </Banner>
      ) : updateAvailable ? (
        <Banner>
          <Download className="size-4 shrink-0" />
          <span className="grow text-xs">
            {reuploaded ? (
              <>kryo.to has a new upload of {game.version ? <b className="text-foreground">{game.version}</b> : 'this game'}: the file you installed was replaced.</>
            ) : (
              <>A newer build is on kryo.to <b className="text-foreground">{latest}</b>. You have {game.version}.</>
            )}
            {updateError ? <span className="block text-destructive">{updateError}</span> : null}
          </span>
          {game.slug ? (
            <SourceSplitButton
              slug={game.slug}
              title={game.title}
              release={releases?.find((r) => r.primary) ?? null}
              onOurs={() => onGetUpdate()}
              onError={setUpdateError}
              size="sm"
              align="right"
            >
              Update
            </SourceSplitButton>
          ) : (
            <Button variant="primary" size="sm" onClick={onGetUpdate}>
              Update
            </Button>
          )}
        </Banner>
      ) : null}
      {problem ? (
        <Banner tone="bad">
          <span className="grow text-xs">
            {problem}
            {folderGone ? <span className="block text-muted-foreground">Moved it? Point Kryoto at the game&apos;s .exe where it is now: your play time and settings stay. Or download it again.</span> : null}
            {repairStuck ? <span className="block text-muted-foreground">Changed the game yourself? Keep your files and Play works again. You can undo that on this page.</span> : null}
          </span>
          {/* A folder that has gone (moved, deleted, a drive unplugged) is not
              something Repair can fix: the game is found again, or fetched again. */}
          {folderGone ? (
            <>
              <Button size="sm" variant="outline" disabled={running || exeBusy} onClick={() => changeExe(true)}>
                <FolderOpen className="size-3" />
                Locate
              </Button>
              {game.slug ? (
                <Button size="sm" variant="primary" disabled={running} onClick={onGetUpdate}>
                  <Download className="size-3" />
                  Reinstall
                </Button>
              ) : null}
            </>
          ) : game.keepFiles ? null : (
            <>
              {repairStuck ? <Button size="sm" variant="primary" disabled={running} onClick={() => keepFiles(true)}>Keep my files</Button> : null}
              <Button size="sm" variant="outline" disabled={running} onClick={onRepair}>Repair game</Button>
            </>
          )}
          {error ? (
            <IconButton label="Dismiss" onClick={onDismissError} className="size-7">
              <X className="size-3.5" />
            </IconButton>
          ) : null}
        </Banner>
      ) : null}

      {!going && game.slug ? <UndoUpdate slug={game.slug} version={game.version ?? null} running={running} onDone={() => onGameChanged?.(game)} /> : null}

      <div className="grid grid-cols-[minmax(0,1fr)_320px] items-start gap-4 p-6">
        <div className="grid gap-4">
          {game.short ? (
            <Card>
              <Label className="mb-3">About</Label>
              <p className="text-sm leading-relaxed text-muted-foreground">{game.short}</p>
              {game.slug && onWeb ? <LibraryWriteups slug={game.slug} onWeb={onWeb} /> : null}
            </Card>
          ) : game.slug && onWeb ? (
            <LibraryWriteups slug={game.slug} onWeb={onWeb} own />
          ) : null}
          <AntivirusCard game={game} running={running} error={error} onGetFiles={onGetUpdate} />
          <AddonsCard game={game} onGet={onGetUpdate} onChanged={(g) => onGameChanged?.(g)} />
          <AchievementsCard game={game} running={running} />
          <CloudSavesCard game={game} running={running} />
          {game.keepFiles ? null : <CommunityOfferCard game={game} running={running} />}
        </div>
        <Card className="grid gap-3">
          <Label>Info</Label>
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-xs">
            {game.developer ? <Fact k="Developer" v={game.developer} /> : null}
            {game.source ? <Fact k="Release" v={game.source} /> : null}
            {game.version ? (
              <Fact k="Build" v={kept ? `${game.version}, kept${latest && latest !== game.version ? ` (current is ${latest})` : ''}` : game.version} />
            ) : null}
            <Fact k="Folder" v={game.installDir} mono />
            {/* The file Play starts, changeable right here: a release can name
                the wrong one, and the game then only starts from its folder. */}
            {game.launcher ? (
              <Fact k="Starts in" v={launcherName(game.launcher) ?? ''} />
            ) : (
              <>
                <dt className="text-[10px] uppercase tracking-wider text-muted-foreground">Starts</dt>
                <dd className="m-0 flex min-w-0 items-center gap-2">
                  <span className="kryo-ascii-art min-w-0 select-text break-all text-[11px] text-foreground/90">{game.executable || 'None set'}</span>
                  {isTauri() ? (
                    <Button size="sm" variant="ghost" disabled={running || exeBusy} onClick={() => changeExe()}>
                      Change
                    </Button>
                  ) : null}
                </dd>
              </>
            )}
            {game.launchOptions ? <Fact k="Options" v={game.launchOptions} mono /> : null}
            {game.keepFiles ? (
              <>
                <dt className="text-[10px] uppercase tracking-wider text-muted-foreground">Files</dt>
                <dd className="m-0 flex min-w-0 items-center gap-2">
                  <span className="min-w-0 text-foreground/90">Your own, no updates</span>
                  <Button size="sm" variant="ghost" disabled={running} onClick={() => keepFiles(false)}>
                    Undo
                  </Button>
                </dd>
              </>
            ) : null}
          </dl>
          {exeError ? <p className="text-[11px] text-destructive">{exeError}</p> : null}
          {game.slug && onWeb ? <CompatibilityFacts slug={game.slug} onWeb={onWeb} /> : null}
          {/* What Play starts, for when a game will not: there when it is
              wanted, folded away when it is not. The Store page and the ways
              to play are in the bar above (the globe, and Play's own menu). */}
          <details className="group border-t border-border pt-3">
            <summary className="flex cursor-pointer list-none items-center justify-between text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:text-foreground">
              Launch command
              <ChevronDown className="size-3 transition-transform group-open:rotate-180" />
            </summary>
            <div className="mt-2 grid gap-2">
              <CommandLine text={command || '...'} />
              <p className="text-[11px] text-muted-foreground">Change it under Manage, Properties.</p>
            </div>
          </details>
        </Card>
      </div>
      </div>
    </section>
  )
}

/**
 * After an update with a backup (Settings > Downloads > Game updates): the way
 * back to the build before it, for as long as the copies are kept. Only while
 * the installed build is the one that update brought.
 */
function UndoUpdate({ slug, version, running, onDone }: { slug: string; version: string | null; running: boolean; onDone: () => void }) {
  const [backup, setBackup] = useState<UpdateBackup | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let cancelled = false
    library.updateBackup(slug).then((b) => !cancelled && setBackup(b)).catch(() => {})
    return () => {
      cancelled = true
    }
  }, [slug, version])
  if (!backup || backup.state !== 'applied' || backup.to_version !== version) return null
  async function undo() {
    setBusy(true)
    setError(null)
    try {
      await library.undoUpdate(slug)
      setBackup(null)
      onDone()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Banner>
      <History className="size-4 shrink-0" />
      <span className="grow text-xs">
        Updated to <b className="text-foreground">{backup.to_version}</b>. If it does not run right, the files it replaced are backed up.
        {error ? <span className="block text-destructive">{error}</span> : null}
      </span>
      <Button size="sm" variant="outline" disabled={busy || running} onClick={() => undo()}>
        {busy ? 'Putting it back' : `Undo update${backup.from_version ? ` (back to ${backup.from_version})` : ''}`}
      </Button>
    </Banner>
  )
}

function Banner({ children, tone = 'info' }: { children: React.ReactNode; tone?: 'info' | 'bad' }) {
  return (
    <div
      role={tone === 'bad' ? 'alert' : undefined}
      className={cn(
        'kryo-radius mx-6 mt-4 flex items-center gap-3 border p-3 pl-4',
        tone === 'bad' ? 'border-destructive/50 text-destructive' : 'border-foreground/30 text-muted-foreground',
      )}
    >
      {children}
    </div>
  )
}
