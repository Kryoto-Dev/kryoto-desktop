import { ProfileLink, TeamName } from '@/ui/TeamName'
import { useEffect, useRef, useState } from 'react'
import { artSrc } from '@/lib/art'
import { Wand2, Bell, Cloud, Download, FolderInput, HardDrive, Heart, LogOut, MessageCircle, Palette, Package, ScrollText, Shield, SlidersHorizontal, Smartphone, User, ChevronDown } from '@/ui/pixel-icons'
import { AsciiBar, Button, Caption, Check, Dropdown, Section, Segmented, inputCls } from '@/ui'
import { errorText, on } from '@/lib/bridge'
import { displayApi, GAME_LANGUAGES, PLAYER_NAME_MAX, settingsApi, useSettings, type DisplayMode, type DisplayState, PET_MASCOTS, PET_SIZES, type PetMascot, type Settings } from '@/lib/settings'
import { isWindowsHost } from '@/lib/library'
import { browserNavigate, isTauri, mountStore, openBigPicture, openSetup, openStorePage, placeMainStore, STORE_HOME } from '@/lib/window'
import type { Account } from '@/hooks/useAccount'
import type { BrowserPageState } from '@/hooks/useBrowserPage'
import { cn } from '@/lib/utils'
import { SoundsPane } from '@/settings/SoundsPane'
import { RuntimesPane } from '@/settings/RuntimesPane'
import { DownloadFolder, LeftoverPrefixes, StoragePane } from '@/settings/StoragePane'
import { CompatPane } from '@/settings/CompatPane'
import { LogsPane } from '@/settings/LogsPane'
import { PhonePane } from '@/settings/PhonePane'
import { ControllerPane } from '@/settings/ControllerPane'
import { CloudPane } from '@/settings/CloudPane'
import { DebridSection, PatchbayPane } from '@/settings/PatchbayPane'
import { ControllerWelcome } from '@/controller/ControllerWelcome'
import { ChatSettingsPanel } from '@/friends/ChatSettings'
import { SceneMotion } from '@/ui/ascii/SceneMotion'
import { BACKUP, TRAY, type CellScene } from '@/ui/ascii/motion'
import { HatchZipExplainer, NEW_IN_SETTINGS, NewDot, NewHere, markSeen, useNewSettings } from '@/ui/FeatureNotice'
import { FindGamesButton } from '@/library/FoundGames'
import { VrPane } from '@/settings/VrPane'
import { Bridge } from '@/library/Bridge'
import { DiscordPane } from '@/settings/DiscordPane'
import { GameUpdatesSection } from '@/settings/GameUpdatesSection'
import { DefenderSection } from '@/settings/DefenderSection'
import { anchorId, revealSetting } from '@/lib/settings-links'
import { useSettled } from '@/hooks/useSettled'
import { TEXT_SIZES, useTextSize, ZOOM_MODIFIER } from '@/lib/text-size'

/**
 * Settings: one page for everything you can set.
 *
 * The top of the rail is your kryo.to account - the site's own settings
 * (profile, appearance, notifications, security, support), shown in the
 * right pane exactly as kryo.to has them, so there is one place to change
 * each thing and it is the same place as on the web. Under it, what only the
 * app has. Changes save as you make them.
 */

/** A small moving picture beside a section's title: what the section is about, at a glance. */
const SECTION_SCENES: Partial<Record<SettingsSection, CellScene>> = {
  downloads: TRAY,
  cloud: BACKUP,
}

export type SettingsSection =
  | 'profile'
  | 'appearance'
  | 'notifications'
  | 'security'
  | 'support'
  | 'general'
  /** Phone and Controller. */
  | 'devices'
  | 'storage'
  | 'downloads'
  | 'sounds'
  /** Chat and calls on this PC: keys, message settings, microphone and speaker. */
  | 'chat'
  | 'runtimes'
  | 'phone'
  | 'controller'
  | 'compat'
  | 'developer'
  | 'logs'
  /** Achievements and saves on the account, and how this PC syncs them. */
  | 'cloud'
  /** The old name of Cloud, still asked for by older links. */
  | 'achievements'
  /** Your own servers for saves and achievements, and debrid. */
  | 'patchbay'
  /** Headset runtimes, and what a VR game starts first. */
  | 'vr'

/** kryo.to's settings, by the fragment that opens each category there. */
const WEB: Partial<Record<SettingsSection, string>> = {
  profile: 'profile',
  appearance: 'appearance',
  notifications: 'notifications',
  security: 'password',
  support: 'donations',
}

export const isWebSection = (s: SettingsSection) => s in WEB

type RailItem = { id: SettingsSection; label: string; icon: React.ReactNode }

export function SettingsPage({
  section,
  reveal = null,
  onSection,
  account,
  page,
  onSignOut,
  onSignIn,
}: {
  section: SettingsSection
  /** One setting to scroll to and light (lib/settings-links.ts); `at` changes when it is asked for again. */
  reveal?: { anchor: string; at: number } | null
  onSection: (s: SettingsSection) => void
  account: Account
  page: BrowserPageState
  onSignOut: () => void
  onSignIn: () => void
}) {
  const guest = !!account.guest
  const railSection = section
  const fresh = useNewSettings(guest)
  // Opening a section with something new in it is seeing it: the dots go out.
  useEffect(() => {
    const id = NEW_IN_SETTINGS[railSection]?.id
    if (id && fresh.includes(railSection)) markSeen(id)
  }, [railSection, fresh])
  const kryo: RailItem[] = [
    { id: 'profile', label: 'Profile', icon: <User /> },
    { id: 'appearance', label: 'Appearance', icon: <Palette /> },
    { id: 'notifications', label: 'Notifications', icon: <Bell /> },
    { id: 'security', label: 'Security', icon: <Shield /> },
    { id: 'support', label: 'Support', icon: <Heart /> },
  ]
  const desktop: RailItem[] = [
    { id: 'general', label: 'General', icon: <SlidersHorizontal /> },
    { id: 'storage', label: 'Storage', icon: <HardDrive /> },
    { id: 'downloads', label: 'Downloads', icon: <Download /> },
    // Saves and achievements can be kept on this PC, so Cloud is for guests too.
    { id: 'cloud' as const, label: 'Cloud', icon: <Cloud /> },
    // Behind kryo.to's `chat` feature flag.
    ...(account.chat && !guest ? [{ id: 'chat' as const, label: 'Chat & calls', icon: <MessageCircle /> }] : []),
    { id: isWindowsHost() ? ('runtimes' as const) : ('compat' as const), label: 'Running games', icon: <Package /> },
    { id: 'devices', label: 'Devices', icon: <Smartphone /> },
    { id: 'logs', label: 'Troubleshooting', icon: <ScrollText /> },
  ]
  const name = account.displayName || account.username

  return (
    <div className="grid min-h-0 grow grid-cols-[232px_1fr]">
      <nav aria-label="Settings" className="flex min-h-0 flex-col gap-5 overflow-auto border-r border-border bg-card/40 p-3">
        <ProfileLink username={guest ? null : account.username} className="flex items-center gap-2.5 px-2 pt-2 focus-visible:outline-2 focus-visible:outline-primary">
          {account.avatarUrl ? (
            <img decoding="async" src={artSrc(account.avatarUrl) ?? undefined} alt="" className="kryo-pill size-9 object-cover" />
          ) : (
            <span className="kryo-pill grid size-9 place-items-center bg-secondary text-xs font-bold">{name.slice(0, 1).toUpperCase()}</span>
          )}
          <span className="grid min-w-0">
            <b className="truncate text-xs text-foreground"><TeamName username={account.guest ? null : account.username}>{name}</TeamName></b>
            <span className="truncate text-[10px] text-muted-foreground">{guest ? 'No account' : <TeamName username={account.username}>@{account.username}</TeamName>}</span>
          </span>
        </ProfileLink>
        {guest ? null : <Rail title="kryo.to account" items={kryo} current={railSection} onSection={onSection} fresh={fresh} />}
        <Rail title="Kryoto Desktop" items={desktop} current={railSection} onSection={onSection} fresh={fresh} />
        <div className="mt-auto px-1">
          {guest ? (
            <Button variant="primary" size="sm" className="w-full" onClick={onSignIn}>
              Sign in to kryo.to
            </Button>
          ) : (
            <Button variant="ghost" size="sm" className="w-full justify-start" onClick={onSignOut}>
              <LogOut className="size-3" />
              Sign out
            </Button>
          )}
        </div>
      </nav>
      {WEB[section] && !guest ? (
        // kryo.to's own sections are addressed by its fragments (its lib/settings-links.ts).
        <WebPane fragment={reveal?.anchor ?? WEB[section]!} page={page} />
      ) : (
        <DesktopPane section={railSection} account={account} reveal={reveal} />
      )}
    </div>
  )
}

function Rail({ title, items, current, onSection, fresh = [] }: { title: string; items: RailItem[]; current: SettingsSection; onSection: (s: SettingsSection) => void; fresh?: SettingsSection[] }) {
  return (
    <div className="grid gap-1">
      <Caption className="px-2 pb-1">{title}</Caption>
      {items.map((it) => (
        <button
          key={it.id}
          type="button"
          aria-current={current === it.id ? 'page' : undefined}
          onClick={() => onSection(it.id)}
          className={cn(
            'kryo-pill flex h-9 items-center gap-2.5 px-3 text-left text-[11px] uppercase tracking-wider [&>svg]:size-3.5',
            current === it.id ? 'bg-primary font-bold text-primary-foreground' : 'text-muted-foreground hover:bg-secondary hover:text-foreground',
          )}
        >
          {it.icon}
          {it.label}
          {fresh.includes(it.id) && current !== it.id ? <NewDot className="ml-auto" /> : null}
        </button>
      ))}
    </div>
  )
}

/**
 * kryo.to's settings in the pane: the Store's web view, moved here and sent
 * to the category. It goes back to its own place when you leave; the Store
 * returns to its page when it is next shown (Shell follows its history).
 */
function WebPane({ fragment, page }: { fragment: string; page: BrowserPageState }) {
  const slot = useRef<HTMLDivElement | null>(null)
  const [ready, setReady] = useState(false)

  useEffect(() => {
    const el = slot.current
    if (!el || !isTauri()) return
    const place = () => {
      const r = el.getBoundingClientRect()
      if (r.width > 2 && r.height > 2) void mountStore(STORE_HOME, { x: r.left, y: r.top, width: r.width, height: r.height })
    }
    place()
    const ro = new ResizeObserver(place)
    ro.observe(el)
    return () => {
      ro.disconnect()
      placeMainStore()
    }
  }, [])

  useEffect(() => {
    if (!isTauri()) return
    setReady(false)
    void browserNavigate(`${STORE_HOME}settings#${fragment}`)
      .then(() => setReady(true))
      .catch(() => setReady(true))
  }, [fragment])

  return (
    <div ref={slot} className="relative grid min-h-0 place-content-center justify-items-center gap-3 bg-background">
      {!isTauri() ? (
        <p className="max-w-sm text-center text-xs text-muted-foreground">kryo.to&apos;s settings open here in the app.</p>
      ) : page.error ? (
        <p className="max-w-sm text-center text-xs text-destructive">{page.error.message}</p>
      ) : !ready || page.loading ? (
        <AsciiBar fraction={null} cells={16} showPct={false} className="text-muted-foreground" />
      ) : null}
    </div>
  )
}

/** The app's own settings. Each change is saved straight away. */
function DesktopPane({ section, account, reveal }: { section: SettingsSection; account: Account; reveal: { anchor: string; at: number } | null }) {
  const stored = useSettings()
  const ready = !!stored
  useEffect(() => {
    if (reveal && ready) return revealSetting(reveal.anchor)
  }, [reveal, section, ready])
  const [s, setS] = useState<Settings | null>(stored)
  // A section's parts each ask for their own data: shown together, so none pushes the next.
  const settled = useSettled(section, !s)
  const [state, setState] = useState<'idle' | 'saving' | 'saved' | string>('idle')
  const [endpointDraft, setEndpointDraft] = useState(stored?.catalogEndpoint ?? '')
  const timer = useRef<number | undefined>(undefined)
  useEffect(() => {
    if (stored && (!s || (s.recoveryError && !stored.recoveryError))) setS(stored)
  }, [stored, s])
  useEffect(() => {
    if (stored) setEndpointDraft(stored.catalogEndpoint)
  }, [stored?.catalogEndpoint])
  // The pet's own right-click menu changes these too (pet.rs).
  useEffect(() => {
    const off = on<{ pet: boolean; petMascot: PetMascot }>('pet-settings', (p) => setS((cur) => (cur ? { ...cur, ...p } : cur)))
    return () => void off.then((f) => f())
  }, [])

  const set = <K extends keyof Settings>(k: K, v: Settings[K]) => patch({ [k]: v } as Partial<Settings>)
  const patch = (p: Partial<Settings>) => {
    if (!s) return
    const next = { ...s, ...p }
    setS(next)
    setState('saving')
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => {
      // Storage changes its folders straight to disk; keep those.
      void settingsApi
        .get()
        .then((latest) =>
          settingsApi.save({
            ...next,
            libraryDir: latest.libraryDir,
            libraryFolders: latest.libraryFolders,
            ...('pet' in p || 'petMascot' in p ? {} : { pet: latest.pet, petMascot: latest.petMascot }),
          }),
        )
        .then(() => setState('saved'))
        .catch((e: unknown) => setState(errorText(e)))
    }, 250)
  }

  if (!s) return <div className="grid place-items-center"><AsciiBar fraction={null} cells={16} showPct={false} /></div>
  const title: Record<string, string> = {
    general: 'General',
    storage: 'Storage',
    downloads: 'Downloads',
    cloud: 'Cloud',
    chat: 'Chat & calls',
    runtimes: 'Running games',
    devices: 'Devices',
    compat: 'Running games',
    logs: 'Troubleshooting',
  }

  const applyEndpoint = () => {
    const endpoint = endpointDraft.trim()
    setState('saving')
    void settingsApi
      .get()
      .then((latest) => settingsApi.save({ ...latest, catalogEndpoint: endpoint }))
      .then((saved) => {
        setS(saved)
        setEndpointDraft(saved.catalogEndpoint)
        setState('saved')
      })
      .catch((e: unknown) => setState(errorText(e)))
  }

  return (
    <div className="min-h-0 overflow-auto">
      {/* The controller page is wider: the drawing and its labels need the room. */}
      <div className={cn('mx-auto grid gap-7 px-8 py-8', section === 'devices' && account.controller ? 'max-w-5xl' : 'max-w-3xl')}>
        <header className="flex items-center justify-between gap-4">
          <h1 className="text-xl font-bold text-foreground">{title[section]}</h1>
          <span className="flex items-center gap-5">
            <span className={cn('text-[10px] uppercase tracking-wider', state === 'saved' || state === 'saving' || state === 'idle' ? 'text-muted-foreground' : 'text-destructive')}>
              {state === 'saving' ? 'Saving' : state === 'saved' ? 'Saved' : state === 'idle' ? '' : state}
            </span>
            {SECTION_SCENES[section] ? <SceneMotion scene={SECTION_SCENES[section]!} className="h-9 text-muted-foreground" /> : null}
          </span>
        </header>

        <div className={cn('grid gap-7 transition-opacity duration-150', !settled && 'opacity-0')}>
        {section === 'storage' ? (
          <>
            <StoragePane onChanged={() => void settingsApi.get().then((v) => setS((cur) => (cur ? { ...cur, libraryDir: v.libraryDir, libraryFolders: v.libraryFolders } : v)))} />
            <Section title="Bring games in" anchor="find-games" hint="Find games Kryoto installed that your library lost, or bring them over from Steam and Hydra with Bridge.">
              <span className="flex flex-wrap items-center gap-2">
                <FindGamesButton />
                <span id={anchorId('bridge')}>
                  <BridgeButton />
                </span>
                <NewHere id="find-games" />
              </span>
            </Section>
            {!isWindowsHost() ? <LeftoverPrefixes /> : null}
            {isWindowsHost() ? (
              <Section title="Windows Security" anchor="defender">
                <DefenderSection />
              </Section>
            ) : null}
          </>
        ) : null}
        {section === 'downloads' ? (
          <>
            {s.libraryFolders.length ? (
              <Section title="Installing" anchor="ask-folder" hint="Updates always go where the game already is.">
                <Check
                  checked={s.askInstallFolder}
                  onChange={(v) => set('askInstallFolder', v)}
                  label={<span className="flex items-center gap-2">Ask where to install <NewHere id="ask-folder" /></span>}
                />
              </Section>
            ) : null}
            <Section title="Download folder" anchor="download-folder" hint="Archives download here, and games still install to your library folders. Keep downloads on one drive and games on another.">
              <DownloadFolder value={s.downloadDir} onChange={(v) => set('downloadDir', v)} />
            </Section>
            <Section title="Unpacking" hint="Used by itself when a drive has room for the game but not for the game and its archive side by side. The archive is deleted as it is unpacked.">
              <Check checked={s.extractInPlace} onChange={(v) => set('extractInPlace', v)} label="Extract in place (HatchZip) for every install" />
              <details className="group">
                <summary className="w-fit cursor-pointer list-none text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:text-foreground">How it works</summary>
                <HatchZipExplainer compact className="pt-3" />
              </details>
            </Section>
            <Section title="Connections" hint="More may improve speed if the filehost allows it.">
              <Segmented
                label="Download connections" value={String(s.connections)}
                options={['1', '4', '8', '16', '32'].map((v) => ({ value: v, label: v }))}
                onChange={(v) => set('connections', Number(v))}
              />
            </Section>
            <Section title="Speed limit">
              <Segmented
                label="Download speed limit" value={String(s.speedLimitMb)}
                options={[
                  { value: '0', label: 'None' },
                  { value: '5', label: '5 MB/s' },
                  { value: '10', label: '10 MB/s' },
                  { value: '25', label: '25 MB/s' },
                  { value: '50', label: '50 MB/s' },
                ]}
                onChange={(v) => set('speedLimitMb', Number(v))}
              />
            </Section>
            <Check checked={s.deleteArchives} onChange={(v) => set('deleteArchives', v)} label="Delete the archive once a game is installed" />
            <Check checked={s.notifyDownloads} onChange={(v) => set('notifyDownloads', v)} label="Tell me when a game is ready to play" />
            <GameUpdatesSection settings={s} set={set} />
            <DebridSection s={s} onSaved={setS} />
          </>
        ) : null}
        {section === 'general' ? (
          <>
            <Section title="Setup" hint={`Where games go, ${isWindowsHost() ? 'Windows Security' : 'Proton for Windows games'}, the theme and the extras, one step at a time.`}>
              <Button className="w-fit" onClick={openSetup}>
                <Wand2 className="size-3.5" />
                Open setup
              </Button>
            </Section>
            <Section title="Desktop theme" hint="Applies to the library, downloads, settings and menus.">
              <Segmented label="Desktop theme" value={s.theme ?? 'account'} options={[{ value: 'account', label: 'Follow kryo.to' }, { value: 'system', label: 'System' }, { value: 'light', label: 'Light' }, { value: 'dark', label: 'Dark' }]} onChange={(v) => set('theme', v)} />
            </Section>
            <TextSizeSection />
            <Section title="Open on">
              <Segmented label="Open on" value={s.startPage} options={[{ value: 'library', label: 'Library' }, { value: 'store', label: 'Store' }]} onChange={(v) => set('startPage', v)} />
            </Section>
            <Section title="Built-in browser" hint="Sites you open from the Store's address bar load without their ad servers. kryo.to itself is never filtered, and its ads open in your own browser.">
              <Check checked={s.blockAds} onChange={(v) => set('blockAds', v)} label="Block ads on other sites" />
            </Section>
            <Check checked={s.closeToTray} onChange={(v) => set('closeToTray', v)} label="Closing the window keeps Kryoto running in the tray" />
            <Check checked={s.startWithSystem} onChange={(v) => set('startWithSystem', v)} label={`Start Kryoto when I sign in to ${isWindowsHost() ? 'Windows' : 'my computer'}`} />
            <Check checked={s.minimizeOnPlay} onChange={(v) => set('minimizeOnPlay', v)} label="Minimize Kryoto while a game runs" />
            <Section title="Desktop pet" hint="Drag it anywhere; right-click it to switch or hide it. It steps aside while a game runs.">
              <Check checked={s.pet} onChange={(v) => set('pet', v)} label="Show the pet on my desktop" />
              {s.pet ? <Segmented label="Desktop pet" value={s.petMascot} options={PET_MASCOTS.map((m) => ({ value: m.id, label: m.label }))} onChange={(v) => set('petMascot', v)} /> : null}
              {s.pet ? <Segmented label="Size" value={s.petSize ?? 'normal'} options={PET_SIZES.map((z) => ({ value: z.id, label: z.label }))} onChange={(v) => set('petSize', v)} /> : null}
            </Section>
            <PlayerNameSection s={s} patch={patch} />
            <Section title="Game language" hint="The language games start in, when they have it, in games built with gbe_fork or RUNE (most of the catalog). Set it per game under Properties. Game's own leaves each game's files as they are now.">
              <Dropdown
                label="Game language"
                value={s.gameLanguage}
                options={[{ value: '', label: "Game's own" }, ...GAME_LANGUAGES]}
                onChange={(gameLanguage) => patch({ gameLanguage })}
              />
            </Section>
            <Section title="Play time and activity" hint="Share anonymous totals of Desktop players online, playing or idle. Signed-in play time counts toward community statistics; the play-time board only shows public profiles. Games kryo.to does not list are never shared: their play time stays on this PC.">
              <Check checked={s.sharePlaytime} onChange={(v) => set('sharePlaytime', v)} label="Share my play time and Desktop activity with kryo.to" />
            </Section>
            <DiscordPane s={s} patch={patch} />
            <GraphicsSection />
            <SoundsPane />
          </>
        ) : null}
        {section === 'cloud' ? (
          <>
            <CloudPane s={s} set={set} onManage={() => openStorePage('/perks#cloud')} />
            <details id={anchorId('patchbay')} className="group grid">
              <summary className="flex w-fit cursor-pointer list-none items-center gap-1.5 text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:text-foreground">
                <ChevronDown className="size-3 transition-transform group-open:rotate-180" aria-hidden />
                Patchbay: your own servers
              </summary>
              <div className="grid gap-4 pt-5">
                <PatchbayPane s={s} onSaved={setS} />
              </div>
            </details>
          </>
        ) : null}
        {section === 'devices' ? (
          <>
            <div id={anchorId('phone')} className="grid gap-7">
              <h2 className="text-sm font-bold text-foreground">Phone</h2>
              <PhonePane />
            </div>
            {account.controller ? (
              <div id={anchorId('controller')} className="grid gap-7 border-t border-border pt-7">
                <h2 className="text-sm font-bold text-foreground">Controller</h2>
                <ControllerWelcome onBigPicture={openBigPicture} />
                <ControllerPane />
              </div>
            ) : null}
          </>
        ) : null}
        {section === 'compat' ? <CompatPane s={s} set={set} /> : null}
        {section === 'runtimes' ? <RuntimesPane /> : null}
        {section === 'compat' || section === 'runtimes' ? (
          <div id={anchorId('vr')} className="grid gap-7 border-t border-border pt-7">
            <h2 className="text-sm font-bold text-foreground">VR</h2>
            <VrPane s={s} set={set} />
          </div>
        ) : null}
        {section === 'logs' ? (
          <>
            <LogsPane sendReports={s.sendReports} onSendReports={(v) => set('sendReports', v)} />
          <Section title="Kryo.to endpoint" hint="Blank uses production. For local testing, use http://localhost:3000.">
            <div className="grid gap-3">
              <label className="grid gap-1.5 text-[10px] uppercase tracking-wider text-muted-foreground">
                Site origin
                <input
                  className={inputCls}
                  type="url"
                  value={endpointDraft}
                  onChange={(e) => setEndpointDraft(e.target.value)}
                  placeholder="https://kryo.to"
                  spellCheck={false}
                />
              </label>
              <p className="text-[10px] leading-relaxed text-muted-foreground">
                This changes the Store, catalog requests and download metadata. Sign in separately on the selected site.
                HTTP is allowed on localhost only.
              </p>
              <Button
                variant="primary"
                size="sm"
                onClick={applyEndpoint}
                disabled={state === 'saving' || endpointDraft.trim() === s.catalogEndpoint}
              >
                Apply endpoint
              </Button>
            </div>
          </Section>
          </>
        ) : null}
        {section === 'chat' ? <ChatSettingsPanel available={!!account.chat} calls={!!account.voice} /> : null}
        </div>
      </div>
    </div>
  )
}

/**
 * The name you have in games: your K// username, one typed here, or whatever
 * each build came with. The same three choices, in the same control, as a
 * game's Properties (which adds "Same as Settings"). Written into the game's
 * emulator files as it starts (src-tauri player_name.rs).
 */
function PlayerNameSection({ s, patch }: { s: Settings; patch: (p: Partial<Settings>) => void }) {
  const [account, setAccount] = useState<string | null>(null)
  /** Custom picked just now: the name field takes the cursor (not when Settings opens on it). */
  const [picked, setPicked] = useState(false)
  useEffect(() => {
    if (isTauri()) void settingsApi.playerAccountName().then(setAccount).catch(() => {})
  }, [])
  return (
    <Section
      title="In-game name"
      hint="What games and other players call you, in games built with gbe_fork or RUNE (most of the catalog). Set it per game under Properties. Kryoto Online games use your Steam name."
    >
      <div className="grid gap-2">
        <Segmented
          value={s.playerNameMode}
          options={[
            { value: 'account', label: 'K// username' },
            { value: 'custom', label: 'Custom' },
            { value: 'build', label: "Game's own" },
          ]}
          onChange={(v) => {
            setPicked(v === 'custom')
            patch({ playerNameMode: v })
          }}
        />
        {s.playerNameMode === 'custom' ? (
          <input
            className={inputCls}
            autoFocus={picked}
            value={s.playerName}
            maxLength={PLAYER_NAME_MAX}
            placeholder="Your name in games"
            aria-label="Your name in games"
            spellCheck={false}
            onChange={(e) => patch({ playerName: e.target.value })}
          />
        ) : s.playerNameMode === 'account' ? (
          <Caption>{account ? `Games call you ${account}.` : 'Sign in to kryo.to to use your username.'}</Caption>
        ) : null}
      </div>
    </Section>
  )
}

/**
 * How large Kryoto draws everything, the Store included (lib/text-size.ts).
 * Applies as it is picked; saved by its own command, not with the page.
 */
function TextSizeSection() {
  const [size, pick] = useTextSize()
  if (size === null) return null
  return (
    <Section title="Text size" anchor="text-size" hint={`Everything in Kryoto, the Store included. ${ZOOM_MODIFIER} + and ${ZOOM_MODIFIER} - change it anywhere, and ${ZOOM_MODIFIER} 0 goes back to 100%.`}>
      <Segmented label="Text size" value={String(size)} options={TEXT_SIZES.map((v) => ({ value: String(v), label: `${v}%` }))} onChange={(v) => pick(Number(v))} />
    </Section>
  )
}

const DISPLAY_HINT: Record<DisplayMode, string> = {
  auto: 'Uses shared-memory buffers to avoid DMA-BUF driver problems while keeping accelerated rendering available.',
  compatible: 'Draws on anything, a little slower. Use it when pages are blank, flicker or are drawn in the wrong place.',
  full: 'Enables DMA-BUF buffer sharing. Some graphics drivers may stutter, draw incorrectly or crash with it enabled.',
}

/**
 * How the Linux build draws its window (src-tauri/src/display_env.rs). Shown
 * only there; takes effect on the next start.
 */
function GraphicsSection() {
  const [state, setState] = useState<DisplayState | null>(null)
  const [picked, setPicked] = useState<DisplayMode | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    void displayApi
      .state()
      .then(setState)
      .catch(() => setState(null))
  }, [])
  if (!state) return null
  const mode = picked ?? state.mode
  const choose = (m: DisplayMode) => {
    setError(null)
    void displayApi
      .setMode(m)
      .then(() => setPicked(m))
      .catch((e: unknown) => setError(errorText(e)))
  }
  return (
    <Section title="Graphics" hint={DISPLAY_HINT[mode]}>
      <Segmented
        value={mode}
        options={[
          { value: 'auto', label: 'Automatic' },
          { value: 'compatible', label: 'Compatible' },
          { value: 'full', label: 'Full' },
        ]}
        onChange={(v) => choose(v as DisplayMode)}
      />
      <p className="text-[11px] text-muted-foreground">
        {state.healed ? 'The last start did not draw its window, so Kryoto switched to Compatible. ' : ''}
        Now: {state.reason}.{picked && picked !== state.mode ? ' Restart Kryoto to use the new setting.' : ''}
      </p>
      {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
    </Section>
  )
}

/**
 * The guided screen (library/Bridge.tsx), also under Library > Bridge and in
 * Add a game. Linked to as `storage#bridge`.
 */
function BridgeButton() {
  const [open, setOpen] = useState(false)
  return (
    <>
      <Button size="sm" onClick={() => setOpen(true)}>
        <FolderInput className="size-3" />
        Open Bridge
      </Button>
      {open ? <Bridge onClose={() => setOpen(false)} /> : null}
    </>
  )
}
