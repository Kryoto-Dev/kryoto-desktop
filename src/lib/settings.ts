import { useEffect, useState } from 'react'
import { call } from '@/lib/bridge'
import { clearCatalogEndpointCache } from '@/lib/endpoint'

export type Palette = 'monochrome' | 'oled' | 'amber' | 'emerald' | 'nord' | 'sepia' | 'blossom'
export type Radius = 'sharp' | 'soft' | 'rounded' | 'round' | 'pill'
export type Theme = 'account' | 'system' | 'light' | 'dark'

/** One of the two buttons Discord allows. */
export type PresenceButton = { kind: 'desktop' | 'game' | 'donate' | 'profile' | 'custom' | 'none' | 'rotate'; label: string; url: string }

/** What Discord shows while a game runs (src-tauri discord_rpc/). */
export type DiscordPresence = {
  on: boolean
  /** Show Kryoto itself while no game runs. */
  browsing: boolean
  line1: string
  line2: string
  picture: 'cover' | 'kryoto' | 'none'
  elapsed: boolean
  playtime: boolean
  /** Adult games show their name and art like any other. */
  adultNames: boolean
  button1: PresenceButton
  button2: PresenceButton
}

/** Mirrors `src-tauri/src/settings.rs`. */
export type Settings = {
  recoveryError?: string | null
  libraryDir: string
  deleteArchives: boolean
  /** Where archives download to. Empty: inside the library folder. */
  downloadDir: string
  startPage: 'store' | 'library'
  defaultCompatTool: string | null
  minimizeOnPlay: boolean
  phoneLan: boolean
  notifyDownloads: boolean
  palette: Palette
  theme?: Theme
  radius: Radius
  font: 'teletext' | 'mono'
  showAdult: boolean
  /** Take palette, corners, typeface and the adult blur from the kryo.to account. */
  followAccount: boolean
  /** Library folders besides `libraryDir` (where new games go). */
  libraryFolders: string[]
  /** With two or more library folders, a download asks which one. */
  askInstallFolder: boolean
  sendReports: boolean
  closeToTray: boolean
  startWithSystem: boolean
  /** Buttons press in under the pointer. */
  pressEffect: boolean
  /** Add play time to the kryo.to account. */
  sharePlaytime: boolean
  /** Parallel connections per download. */
  connections: number
  /** MB/s cap; 0 is none. */
  speedLimitMb: number
  /** Blank uses production; set a local site origin while developing. */
  catalogEndpoint: string
  /** Linux: MangoHud's overlay on Windows games. */
  linuxMangohud: boolean
  /** Linux: run Windows games under GameMode. */
  linuxGamemode: boolean
  /** Linux: Proton-GE's FSR upscaling at lower fullscreen resolutions. */
  linuxFsr: boolean
  /** The name in games: your K// username, `playerName`, or each build's own. */
  playerNameMode: PlayerNameMode
  playerName: string
  /** The language games start in, as Steam names it; empty leaves each build's own. */
  gameLanguage: string
  /** Cloud saves (saves/ in src-tauri), where kryo.to has them on. */
  cloudSaves: boolean
  /** Back a game's saves up when it closes. */
  cloudSavesOnExit: boolean
  /** Newer saves from another PC before Play: put them here, or ask first. */
  cloudSavesRestore: 'auto' | 'ask'
  /** Watch where games write, to find their saves. */
  cloudSavesDetect: boolean
  /** How: `precise` asks for administrator once per game and sees exactly
   *  which files it writes; `smart` needs no administrator. */
  cloudSavesFind: 'precise' | 'smart'
  /** Tell kryo.to the save folders found, so other players' games find theirs. */
  cloudSavesShare: boolean
  /** Local backups of replaced saves kept per game. */
  cloudSavesKeepLocal: number
  /** The desktop pet (src/pet): out on the screen or put away. */
  pet: boolean
  petMascot: PetMascot
  /** Where it stands, in physical pixels; pet.rs keeps it. */
  petPos?: [number, number] | null
  discord: DiscordPresence
  /** Patchbay: a cloud saves server of your own instead of kryo.to. Blank is kryo.to. */
  savesEndpoint: string
  /**
   * Where save versions are kept (src-tauri saves/): the kryo.to account (a
   * supporter perk), a folder on this PC (`savesLocalDir`), or the Patchbay
   * server. Blank, as before there was a choice: the own server when one is set.
   */
  savesMode?: '' | KeptIn
  /** On this PC: the folder for save versions and achievements, apart from caches. */
  savesLocalDir?: string
  /** On this PC: versions kept per game. */
  savesLocalKeep?: number
  /** The card about kryo.to's cloud saves was closed for good. */
  savesPromoClosed?: boolean
  /** Where achievements are kept, the same three places. */
  achievementsMode?: '' | KeptIn
  /** Patchbay: an achievements server of your own instead of kryo.to. */
  achievementsEndpoint: string
  /** Patchbay: the debrid service downloads try first. Its token is in the OS secure store. */
  debrid: '' | 'real-debrid' | 'torbox'
  /** Unpack archives in place with HatchZip: room for the game, not the game and its archive. Used anyway when a drive is short. */
  extractInPlace: boolean
  /** New-feature notices (ui/FeatureNotice.tsx) already dismissed. */
  seenNotices: string[]
  /** The first-start setup (setup/FirstStart.tsx) is done or skipped; false only on a fresh install. */
  welcomed: boolean
  /** VR: the OpenXR runtime VR games get, by id; blank leaves the system's own. */
  vrRuntime: string
  /** VR: start SteamVR before a VR game when it is installed and not running. */
  vrStartSteamvr: boolean
  /** The Store's address bar refuses ad servers on other sites (adblock.rs). kryo.to is never filtered. */
  blockAds: boolean
  /** Get newer Kryoto builds of installed games without asking (game_update.rs). Off by default. */
  autoUpdateGames: boolean
  /** Where an update copies the files it replaces first, so it can be undone. Empty: no copies. */
  updateBackupDir: string
}

export type PetMascot = 'telly' | 'kubi' | 'yuki' | 'blip'
export const PET_MASCOTS: { id: PetMascot; label: string }[] = [
  { id: 'telly', label: 'Telly' },
  { id: 'kubi', label: 'Kubi' },
  { id: 'yuki', label: 'Yuki' },
  { id: 'blip', label: 'Blip' },
]

/** Every language Steam has, by the name the emulators hand a game (src-tauri player_name.rs). */
export const GAME_LANGUAGES: { value: string; label: string }[] = [
  { value: 'arabic', label: 'Arabic' },
  { value: 'bulgarian', label: 'Bulgarian' },
  { value: 'schinese', label: 'Chinese (Simplified)' },
  { value: 'tchinese', label: 'Chinese (Traditional)' },
  { value: 'czech', label: 'Czech' },
  { value: 'danish', label: 'Danish' },
  { value: 'dutch', label: 'Dutch' },
  { value: 'english', label: 'English' },
  { value: 'finnish', label: 'Finnish' },
  { value: 'french', label: 'French' },
  { value: 'german', label: 'German' },
  { value: 'greek', label: 'Greek' },
  { value: 'hungarian', label: 'Hungarian' },
  { value: 'indonesian', label: 'Indonesian' },
  { value: 'italian', label: 'Italian' },
  { value: 'japanese', label: 'Japanese' },
  { value: 'koreana', label: 'Korean' },
  { value: 'norwegian', label: 'Norwegian' },
  { value: 'polish', label: 'Polish' },
  { value: 'portuguese', label: 'Portuguese' },
  { value: 'brazilian', label: 'Portuguese (Brazil)' },
  { value: 'romanian', label: 'Romanian' },
  { value: 'russian', label: 'Russian' },
  { value: 'spanish', label: 'Spanish' },
  { value: 'latam', label: 'Spanish (Latin America)' },
  { value: 'swedish', label: 'Swedish' },
  { value: 'thai', label: 'Thai' },
  { value: 'turkish', label: 'Turkish' },
  { value: 'ukrainian', label: 'Ukrainian' },
  { value: 'vietnamese', label: 'Vietnamese' },
]

/** How a game's in-game name is picked (src-tauri player_name.rs). */
export type PlayerNameMode = 'account' | 'custom' | 'build'

/** Steam's limit for a name, which the emulators share. */
export const PLAYER_NAME_MAX = 32

/** kryo.to's palettes, named as the site names them. */
export const PALETTES: { id: Palette; label: string }[] = [
  { id: 'monochrome', label: 'Monochrome' },
  { id: 'oled', label: 'Black' },
  { id: 'amber', label: 'Amber' },
  { id: 'emerald', label: 'Green' },
  { id: 'nord', label: 'Slate' },
  { id: 'sepia', label: 'Sepia' },
  { id: 'blossom', label: 'Pink' },
]

export const RADII: { value: Radius; label: string }[] = [
  { value: 'sharp', label: 'Sharp' },
  { value: 'soft', label: 'Soft' },
  { value: 'rounded', label: 'Rounded' },
  { value: 'round', label: 'Round' },
  { value: 'pill', label: 'Pill' },
]

export type Look = Pick<Settings, 'palette' | 'radius' | 'font' | 'showAdult'> & {
  theme: 'system' | 'light' | 'dark'
  season?: string | null
  reducedMotion?: boolean
  /** Classic line icons instead of the pixel ones, as chosen on kryo.to. */
  classicIcons?: boolean
  /** kryo.to's "Hide" for adult games: left out of every list the client draws from kryo.to. */
  hideAdult?: boolean
}

/** What a kryo.to account says about its look (`/api/auth/me`). */
export type AccountAppearance = {
  theme?: string | null
  palette: string | null
  radius: string | null
  typeface: string | null
  nsfwBlur: boolean
  /** kryo.to's "Hide adult games" (Settings > Appearance on the site). */
  nsfwHide?: boolean
  /** kryo.to's "reduce motion": no press, no animations. */
  motion?: string | null
}

const PALETTE_IDS = new Set(PALETTES.map((p) => p.id as string))
const RADIUS_IDS = new Set(RADII.map((r) => r.value as string))

/** Resolved appearance of the website, including signed-out preferences. */
export type CatalogAppearance = {
  theme: 'light' | 'dark'
  palette?: string | null
  radius?: string | null
  font?: string | null
  motion?: string | null
  season?: string | null
  /** kryo.to's Icons choice (Settings > Appearance): Kryoto's pixel icons or the classic line ones. */
  icons?: string | null
}
const SEASON_IDS = new Set(['halloween', 'christmas', 'new-year', 'valentine', 'easter', 'winter'])

/**
 * Follow the site's resolved theme, including its theme toggle.
 * A local theme choice also works offline and for guests.
 */
export function effectiveLook(s: Settings, account: { appearance?: AccountAppearance | null } | null | undefined, catalog?: CatalogAppearance | null): Look {
  const a = account?.appearance
  const choice = s.theme ?? 'account'
  const accountTheme = a?.theme === 'light' || a?.theme === 'system' ? a.theme : 'dark'
  const theme = choice === 'account' ? catalog?.theme ?? accountTheme : choice
  const season = s.followAccount && catalog?.season && SEASON_IDS.has(catalog.season) ? catalog.season : null
  if (!s.followAccount) return { theme, palette: s.palette, radius: s.radius, font: s.font, showAdult: s.showAdult, classicIcons: catalog?.icons === 'classic' }
  const palette = catalog?.palette ?? a?.palette ?? s.palette
  const radius = catalog?.radius ?? a?.radius ?? s.radius
  const font = catalog?.font ?? a?.typeface ?? s.font
  return {
    theme,
    season,
    palette: (PALETTE_IDS.has(palette) ? palette : 'monochrome') as Palette,
    radius: (RADIUS_IDS.has(radius) ? radius : 'pill') as Radius,
    font: font === 'mono' ? 'mono' : 'teletext',
    showAdult: a ? !a.nsfwBlur && !a.nsfwHide : s.showAdult,
    hideAdult: a?.nsfwHide === true,
    reducedMotion: (catalog?.motion ?? a?.motion) === 'reduced',
    classicIcons: catalog?.icons === 'classic',
  }
}

/** Stamp the look on <html>, the way kryo.to does before first paint. */
export function applyLook(s: Pick<Settings, 'palette' | 'radius' | 'font'> & { theme?: 'system' | 'light' | 'dark'; season?: string | null; reducedMotion?: boolean; classicIcons?: boolean; systemDark?: boolean | null }) {
  const html = document.documentElement
  const dark = s.theme === 'system' ? s.systemDark ?? matchMedia('(prefers-color-scheme: dark)').matches : s.theme !== 'light'
  html.classList.toggle('dark', dark)
  // Never dark on Linux: WebKitGTK 2.54 draws a see-through window's
  // backgrounds at partial alpha under a dark scheme (styles.css, KB-0006).
  html.style.colorScheme = dark && html.dataset.os !== 'linux' ? 'dark' : 'light'
  if (s.reducedMotion) html.dataset.motion = 'reduced'
  else delete html.dataset.motion
  if (s.classicIcons) html.dataset.icons = 'classic'
  else delete html.dataset.icons
  html.dataset.palette = s.palette || 'monochrome'
  html.dataset.radius = s.radius || 'pill'
  if (s.season && SEASON_IDS.has(s.season)) html.dataset.season = s.season
  else delete html.dataset.season
  if (s.font === 'mono') html.dataset.font = 'mono'
  else delete html.dataset.font
}

/* One copy of the settings for the whole window, so a save in Settings
   reaches everything that reads them (the look, the Store's start page). */
let cached: Settings | null = null
const subscribers = new Set<(s: Settings) => void>()
function publish(s: Settings) {
  cached = s
  subscribers.forEach((fn) => fn(s))
}

/** How the Linux build draws its window (src-tauri/src/display_env.rs). */
export type DisplayMode = 'auto' | 'compatible' | 'full'
export type DisplayState = { mode: DisplayMode; reason: string; healed: boolean }

export const displayApi = {
  /** Null outside the Linux build. */
  state: () => call<DisplayState | null>('display_state'),
  /** Takes effect on the next start. */
  setMode: (mode: DisplayMode) => call<void>('display_set_mode', { mode }),
}

/** Where saves or achievements are kept: kryo.to, this PC, or your own server. */
export type KeptIn = 'account' | 'local' | 'patchbay'

let changing: Promise<unknown> = Promise.resolve()

export const settingsApi = {
  recover: async () => {
    const settings = await call<Settings>('settings_recover')
    publish(settings)
    return settings
  },
  /** The K// username games get, remembered from the last sign-in. Null for a guest. */
  playerAccountName: () => call<string | null>('player_account_name'),
  get: async () => {
    const s = await call<Settings>('settings_get')
    clearCatalogEndpointCache()
    publish(s)
    return s
  },
  save: async (settings: Settings) => {
    const s = await call<Settings>('settings_save', { settings })
    clearCatalogEndpointCache()
    publish(s)
    return s
  },
  /**
   * Change something on top of what is saved now. One at a time: two running
   * together both started from the same copy, and the later one undid the
   * earlier, so a notice marked seen came back at the next start.
   */
  change: (patch: (now: Settings) => Settings) => {
    const done = changing.then(async () => settingsApi.save(patch(await settingsApi.get())))
    changing = done.catch(() => undefined)
    return done
  },
  /** Re-read after the native side changed them (Storage writes folders). */
  reload: () => settingsApi.get(),
}

export function useSettings(): Settings | null {
  const [s, setS] = useState<Settings | null>(cached)
  useEffect(() => {
    subscribers.add(setS)
    if (!cached) void settingsApi.get().catch(() => {})
    else setS(cached)
    return () => {
      subscribers.delete(setS)
    }
  }, [])
  return s
}
