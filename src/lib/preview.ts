/**
 * Browser preview: the native side, played by an in-memory stand-in.
 *
 * Only used when the UI runs in a plain browser (`pnpm dev`) - the desktop
 * app never loads any of this. It exists so every screen of the client can
 * be opened, clicked and checked without building Rust: a small library with
 * Captain Hardcore's real launch entries, a download that moves, a game that
 * runs for a few seconds and adds to its playtime.
 */

import { PREVIEW_FRIENDS, type FriendsState } from '@/hooks/useFriends'

type Handler = (p: unknown) => void
const listeners = new Map<string, Set<Handler>>()

const remoteConsent = { signedIn: true, enabled: false, decided: false }

export const previewBus = {
  on(event: string, fn: Handler) {
    if (!listeners.has(event)) listeners.set(event, new Set())
    listeners.get(event)!.add(fn)
    return () => listeners.get(event)?.delete(fn)
  },
  emit(event: string, payload: unknown) {
    listeners.get(event)?.forEach((fn) => fn(payload))
  },
}

const phone: { lan: boolean; code: string | null; pairUri: string | null } = { lan: false, code: null, pairUri: null }
const now = () => Math.floor(Date.now() / 1000)
const steam = (appid: number, file: string) => `https://cdn.cloudflare.steamstatic.com/steam/apps/${appid}/${file}`

function game(appid: number, title: string, extra: Record<string, unknown> = {}) {
  return {
    id: title.toLowerCase().replace(/[^a-z0-9]+/g, '-'),
    title,
    slug: title.toLowerCase().replace(/[^a-z0-9]+/g, '-'),
    cover: steam(appid, 'library_600x900.jpg'),
    hero: steam(appid, 'library_hero.jpg'),
    installDir: `C:\\Users\\you\\Kryoto Games\\${title}`,
    executable: `${title}.exe`,
    defaultArgs: '',
    entries: [],
    source: 'Steam + gbe_fork',
    preferredEntry: null,
    askEachTime: false,
    launchOptions: '',
    compatTool: null,
    applyOverrides: true,
    playtimeSeconds: 0,
    lastPlayed: null,
    addedAt: now() - 86400 * 20,
    version: 'b1',
    short: null,
    developer: null,
    nsfw: false,
    ...extra,
  }
}

const games = [
  game(1190600, 'Captain Hardcore', {
    executable: 'Captain Hardcore.exe',
    entries: [
      { executable: 'Captain Hardcore.exe', arguments: '', workingdir: '', description: '', oslist: 'windows', type: 'vr' },
      {
        executable: 'Captain Hardcore.exe',
        arguments: '-nohmd',
        workingdir: '',
        description: 'Captain Hardcore Desktop Mode',
        oslist: 'windows',
        type: 'default',
      },
    ],
    source: 'Steam (DRM-free)',
    playtimeSeconds: 5400,
    lastPlayed: now() - 86400 * 4,
    short: 'A VR action game with a desktop mode behind -nohmd.',
    developer: 'AntiZero Games',
    nsfw: true,
    version: 'b20488383',
  }),
  game(367520, 'Hollow Knight', {
    playtimeSeconds: 162000,
    lastPlayed: now() - 3600 * 3,
    source: 'Steam (DRM-free)',
    // One of its DLC already in (the Add-ons & DLC card, previewAddons below).
    addons: [{ label: 'DLC: Hidden Dreams', file: 'Hollow Knight DLC - Hidden Dreams - Kryoto.7z', files: ['DLC/hidden_dreams.pak'], replaced: [], installedAt: now() - 86400 * 5 }],
  }),
  game(504230, 'Celeste', { playtimeSeconds: 30000, lastPlayed: now() - 86400 * 9 }),
  game(1145360, 'Hades', { playtimeSeconds: 72000, lastPlayed: now() - 86400 * 30, source: 'Steam + Kryoto Online', launcher: 'steam' }),
  game(413150, 'Stardew Valley', { playtimeSeconds: 0 }),
  // One Steam game, two listings: the public build and the gb27 beta.
  game(2027330, 'GoreBox', { steamBranch: 'public', playtimeSeconds: 7200, lastPlayed: now() - 86400 * 2 }),
  game(2027330, 'GoreBox27 (early access)', { steamBranch: 'gb27', playtimeSeconds: 3600, lastPlayed: now() - 86400 }),
  // A beta listed under the very same name: only its tag tells them apart.
  game(2027330, 'GoreBox', {
    id: 'gorebox-public-beta',
    slug: 'gorebox-public-beta',
    steamBranch: 'public-beta',
    installDir: 'C:\\Users\\you\\Kryoto Games\\GoreBox (public beta)',
    playtimeSeconds: 1080,
    lastPlayed: now() - 86400 * 3,
  }),
  game(105600, 'Terraria', {
    launcher: 'steam',
    playtimeSeconds: 900,
    lastPlayed: now() - 86400 * 60,
    source: 'Steam + online-fix',
  }),
] as Record<string, unknown>[]

const running = new Set<string>()
/** Settings > Patchbay's own server for `key`, by host, as the native side reports it. */
function ownHost(key: 'savesEndpoint' | 'achievementsEndpoint'): string | null {
  try {
    return settings[key] ? new URL(String(settings[key])).host : null
  } catch {
    return null
  }
}

const settings: Record<string, unknown> = {
  libraryDir: 'C:\\Users\\you\\Kryoto Games',
  deleteArchives: true,
  downloadDir: '',
  startPage: 'library',
  defaultCompatTool: null,
  minimizeOnPlay: false,
  phoneLan: false,
  notifyDownloads: true,
  palette: 'monochrome',
  radius: 'pill',
  font: 'teletext',
  showAdult: false,
  followAccount: true,
  libraryFolders: ['D:\\Games\\Kryoto'],
  askInstallFolder: true,
  sendReports: true,
  closeToTray: true,
  startWithSystem: false,
  pressEffect: true,
  sharePlaytime: true,
  connections: 16,
  speedLimitMb: 0,
  catalogEndpoint: '',
  savesEndpoint: '',
  achievementsEndpoint: '',
  debrid: '',
  extractInPlace: false,
  // An everyday player: this version's What's new (setup/FirstStart.tsx) and
  // found-games check are behind them. `?whatsnew` in the preview shows them.
  seenNotices:
    typeof location !== 'undefined' && location.search.includes('whatsnew')
      ? []
      : [
          ...['setup', 'intro', 'found-games'].map((n) => `${n}-${__APP_VERSION__.replace(/\./g, '-')}`),
          // What's new is the 0.3.0 one until a version brings its own (FirstStart's SETUP_ID).
          'setup-0-3-0',
        ],
  // `?firststart` in the preview shows the first-start setup (setup/FirstStart.tsx).
  welcomed: typeof location === 'undefined' || !location.search.includes('firststart'),
  vrRuntime: '',
  vrStartSteamvr: true,
  blockAds: true,
  linuxMangohud: false,
  linuxGamemode: false,
  linuxFsr: false,
  gameLanguage: '',
  cloudSaves: true,
  cloudSavesOnExit: true,
  cloudSavesRestore: 'auto',
  cloudSavesDetect: true,
  cloudSavesFind: 'precise',
  cloudSavesShare: true,
  cloudSavesKeepLocal: 5,
  pet: true,
  petMascot: 'telly',
  discord: {
    on: true,
    browsing: false,
    line1: '{game}',
    line2: '{playtime} played',
    picture: 'cover',
    elapsed: true,
    playtime: true,
    adultNames: false,
    button1: { kind: 'desktop', label: '', url: '' },
    button2: { kind: 'rotate', label: '', url: '' },
  },
}

const GB = 1_073_741_824
const stored = (id: string, bytes: number) => {
  const g = games.find((x) => x.id === id) ?? {}
  return { id, title: String(g.title ?? id), folder: `C:\\Users\\you\\Kryoto Games\\${String(g.title ?? id)}`, bytes, lastPlayed: g.lastPlayed ?? null, cover: g.cover ?? null, nsfw: !!g.nsfw }
}

const LOG = [
  '2026-09-25 14:03:07Z INFO  app: Kryoto Desktop 0.2.2 started on windows',
  '2026-09-25 14:03:09Z INFO  storage: added library folder D:\\Games\\Kryoto',
  '2026-09-25 14:05:41Z ERROR download: dl-1: The connection kept dropping (timed out). Resume to try again.',
  '2026-09-25 14:07:12Z INFO  library: uninstalling Terraria from C:\\Users\\you\\Kryoto Games\\Terraria',
]

const dl = (over: Record<string, unknown>) => ({
  id: 'dl-1',
  slug: 'hades-ii',
  url: 'https://dl.kryo.to/d/preview',
  fileName: 'Hades II - Kryoto.7z',
  archivePath: '',
  total: 11_200_000_000,
  received: 3_900_000_000,
  speed: 38_000_000,
  extracted: 0,
  extractTotal: null,
  status: 'downloading',
  error: null,
  installDir: null,
  gameId: null,
  addedAt: now() - 600,
  finishedAt: null,
  meta: {
    title: 'Hades II',
    cover: steam(1145350, 'library_600x900.jpg'),
    hero: steam(1145350, 'library_hero.jpg'),
    executable: 'Hades2.exe',
    entries: [],
    source: 'Steam + gbe_fork',
    version: 'b1',
    sizeBytes: 11_200_000_000,
  },
  ...over,
})

/** Two lists of the account's and a followed one that gained a game (collections.rs). */
const collectionGame = (slug: string, title: string, gb: number | null, nsfw = false) => ({ slug, title, cover: null, nsfw, sizeBytes: gb === null ? null : gb * GB })
const collectionsPreview: { collections: { id: string; name: string; href: string; role: string; games: ReturnType<typeof collectionGame>[] }[]; installed: string[]; updates: { id: string; name: string; href: string; added: ReturnType<typeof collectionGame>[] }[] } = {
  collections: [
    { id: '101', name: 'Cozy nights', href: '/user/mira/c/cozy-nights', role: 'owner', games: [collectionGame('stardew-valley', 'Stardew Valley', 0.6), collectionGame('celeste', 'Celeste', 1.2), collectionGame('a-short-hike', 'A Short Hike', 0.4), collectionGame('unpacking', 'Unpacking', 1.1)] },
    { id: '102', name: 'Finished in a weekend', href: '/user/mira/c/weekend', role: 'owner', games: [collectionGame('celeste', 'Celeste', 1.2)] },
    { id: '201', name: 'Best of the decade', href: '/user/ren/c/decade', role: 'following', games: [collectionGame('hades-ii', 'Hades II', 9.8), collectionGame('tunic', 'TUNIC', 1.9), collectionGame('outer-wilds', 'Outer Wilds', null), collectionGame('after-dark', 'After Dark', 2.2, true)] },
  ],
  installed: ['201'],
  // The followed list's new game asks on start (library/CollectionInstall.tsx); `?collectionupdate` shows it.
  updates:
    typeof location !== 'undefined' && location.search.includes('collectionupdate')
      ? [{ id: '201', name: 'Best of the decade', href: '/user/ren/c/decade', added: [collectionGame('outer-wilds', 'Outer Wilds', 6.3)] }]
      : [],
}

const downloadList: Record<string, unknown>[] = [
  dl({}),
  dl({
    id: 'dl-0',
    slug: 'celeste',
    status: 'installed',
    received: 1_200_000_000,
    total: 1_200_000_000,
    speed: 0,
    finishedAt: now() - 86400,
    gameId: 'celeste',
    meta: {
      ...dl({}).meta,
      title: 'Celeste',
      cover: steam(504230, 'library_600x900.jpg'),
      hero: steam(504230, 'library_hero.jpg'),
    },
  }),
  // Its checksum failed; the archive is kept for Install anyway.
  dl({
    id: 'dl-2',
    slug: 'schedule-i',
    status: 'failed',
    received: 2_075_315_145,
    total: 2_075_315_145,
    speed: 0,
    hashMismatch: true,
    error: "The archive doesn't match kryo.to's checksum. Install it anyway, or Retry to download it again.",
    meta: {
      ...dl({}).meta,
      title: 'Schedule I',
      cover: steam(3164500, 'library_600x900.jpg'),
      hero: steam(3164500, 'library_hero.jpg'),
    },
  }),
]

// Initialized only by the browser-preview backend, never by native/web chat.
let simulation: ReturnType<typeof setInterval> | undefined
export function stopPreview() {
  clearInterval(simulation)
  simulation = undefined
}
export function startPreview() {
  if (simulation !== undefined) return
  simulation = setInterval(() => {
/**
 * kryo.to's add-ons for the sample games (`addons` in /api/games/<slug>/downloads):
 * Hollow Knight's DLC, more than the card shows unfolded and one with a long
 * name, and Terraria's language pack and Online-Fix repair. `?addonchallenge`
 * makes kryo.to want its own check for every add-on (a new account).
 */
const addonFixture = (id: number, label: string, size: string, extra: Record<string, unknown> = {}) => ({
  id,
  label,
  note: null,
  source: 'DLC',
  version: 'b1',
  download_size: size,
  kind: 'addon',
  offered: true,
  links: [{ name: `${label.replace(/^DLC: /, 'Hollow Knight DLC - ')} - Kryoto.7z` }],
  pages: [],
  hashes: [],
  ...extra,
})
const previewAddons: Record<string, Record<string, unknown>[]> = {
  'hollow-knight': [
    addonFixture(9001, 'DLC: Hidden Dreams', '120 MB'),
    addonFixture(9002, 'DLC: The Grimm Troupe', '340 MB'),
    addonFixture(9003, 'DLC: Lifeblood', '210 MB'),
    addonFixture(9004, 'DLC: Godmaster', '480 MB'),
    addonFixture(9005, 'DLC: Official Soundtrack and Digital Art Book with Concept Sketches', '1.1 GB'),
    addonFixture(9006, 'DLC: Silksong Preview Pack', '95 MB'),
    addonFixture(9007, 'DLC: Gods and Glory Costume Set', '60 MB'),
    addonFixture(9008, 'DLC: Voidheart Edition Extras', '150 MB'),
  ],
  terraria: [
    addonFixture(9101, 'Japanese language pack', '45 MB', { source: 'Language', note: 'Menus and dialogue in Japanese.' }),
    addonFixture(9102, 'Online-Fix', '12 MB', { source: 'OnlineFix', kind: 'repair', note: 'Online play through Steam.' }),
  ],
}
const SIZE_UNIT: Record<string, number> = { KB: 1024, MB: 1024 ** 2, GB: 1024 ** 3 }
const sizeOf = (text: unknown) => {
  const [n, unit] = String(text ?? '').split(' ')
  return Math.round(Number(n) * (SIZE_UNIT[unit ?? ""] ?? 0)) || 200_000_000
}

  let moved = false
  for (const d of downloadList) {
    if (d.status === 'downloading') {
      d.received = Math.min(d.total as number, (d.received as number) + (d.speed as number) / 2)
      d.speed = 30_000_000 + Math.round(Math.random() * 16_000_000)
      if (d.received === d.total) {
        d.status = 'extracting'
        d.extractTotal = Math.round((d.total as number) * 1.6)
        d.extracted = 0
      }
      moved = true
    } else if (d.status === 'extracting' && d.extractTotal) {
      // Unpacks in a few seconds, then it is ready to play.
      d.extracted = Math.min(d.extractTotal as number, (d.extracted as number) + (d.extractTotal as number) / 12)
      if (d.extracted === d.extractTotal) {
        d.status = 'installed'
        d.finishedAt = now()
        const owner = d.addon ? games.find((g) => g.id === d.gameId) : undefined
        if (owner) {
          // An add-on goes into its game (addons.rs `install`).
          const list = ((owner.addons as Record<string, unknown>[] | undefined) ?? []).filter((a) => a.file !== d.fileName)
          if (!d.repair) owner.addons = [...list, { label: d.addon, file: d.fileName, files: [`DLC/${String(d.fileName)}.pak`], replaced: [], installedAt: now() }]
          previewBus.emit('library-changed', null)
          previewBus.emit('notify', { title: `${String(d.addon).replace(/^DLC: /, '')} applied`, body: `Added to ${String(owner.title)}.`, gameId: owner.id })
        } else {
          previewBus.emit('notify', { title: `${(d.meta as { title: string }).title} is ready to play`, body: 'Installed. Press Play.', gameId: d.gameId ?? null })
        }
      }
      moved = true
    }
  }
  if (moved) previewBus.emit('downloads', downloadList.map((x) => ({ ...x })))
}, 500)
}
if (import.meta.hot) import.meta.hot.dispose(stopPreview)

const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T

let previewChat: { state: string } = { state: 'off' }

// A small pretend chat for `pnpm dev`: messages live in memory, the other
// side "reads" and answers after a moment. Ids match lib/chat's conversation id.
type PreviewMsg = {
  msgId: string; conversationId: string; senderUser: string; outgoing: boolean; sentAt: number; receivedAt: number
  kind: 'text' | 'gif' | 'invite' | 'file'; body: string; replyTo: string | null; editedAt: number | null; deleted: boolean
  status: string; reactions: { emoji: string; userIds: string[] }[]
}
const previewMsgs: PreviewMsg[] = []
const previewRoom: { id: string; body: string; replyTo?: { id: string; gone: boolean; name: string | null; username: string | null; excerpt: string } | null; createdAt: string; user: { id: string; username: string; displayName: string | null; avatarUrl: string | null; supporter: boolean } }[] = [
  { id: '1', body: 'anyone up for Lethal Company tonight?', createdAt: new Date().toISOString(), user: { id: '2', username: 'bo', displayName: 'Bo', avatarUrl: null, supporter: true } },
]
const previewVerified = new Set<string>()
let previewGroups: { id: string; name: string; members: { userId: string; role: string }[]; myRole: string }[] = [
  { id: '900', name: 'Lethal crew', members: [{ userId: '1', role: 'owner' }, { userId: '2', role: 'member' }, { userId: '8', role: 'member' }], myRole: 'owner' },
]
let previewBackup = { exists: false, updatedAtMs: 0, keptHere: false }
let previewSettings = { readReceipts: true, typing: true, notificationContent: 'name', gifsAuto: true }
const convOf = (peer: string) => {
  if (peer.startsWith('g:')) return `6772703a${BigInt(peer.slice(2)).toString(16).padStart(16, '0')}`
  const [lo, hi] = BigInt('1') <= BigInt(peer) ? [1n, BigInt(peer)] : [BigInt(peer), 1n]
  return `646d3a${lo.toString(16).padStart(16, '0')}${hi.toString(16).padStart(16, '0')}`
}
const newId = () => Array.from({ length: 16 }, () => Math.floor(Math.random() * 256).toString(16).padStart(2, '0')).join('')
const update = (m: PreviewMsg) => previewBus.emit('chat-updated', { ...m, reactions: m.reactions.map((r) => ({ ...r })) })
function previewSeed() {
  if (previewMsgs.length) return
  const now = Date.now()
  previewMsgs.push(
    { msgId: newId(), conversationId: convOf('2'), senderUser: '2', outgoing: false, sentAt: now - 3_600_000, receivedAt: now - 3_600_000, kind: 'text', body: 'Lethal Company tonight?', replyTo: null, editedAt: null, deleted: false, status: 'received', reactions: [] },
    { msgId: newId(), conversationId: convOf('2'), senderUser: '1', outgoing: true, sentAt: now - 3_500_000, receivedAt: now - 3_500_000, kind: 'text', body: 'yes! after dinner', replyTo: null, editedAt: null, deleted: false, status: 'read', reactions: [{ emoji: '🔥', userIds: ['2'] }] },
    { msgId: newId(), conversationId: convOf('6'), senderUser: '6', outgoing: false, sentAt: now - 600_000, receivedAt: now - 600_000, kind: 'text', body: 'hey, saw your review of OFME, can I ask something?', replyTo: null, editedAt: null, deleted: false, status: 'received', reactions: [] },
  )
}

// Game runtimes (runtimes.rs): a PC missing a few, and an install played out
// on timers, so the start-screen prompt and Settings can be clicked through.
const RUNTIMES = [
  { id: 'vc2015-x64', name: 'Visual C++ 2015-2022 (64-bit)', group: 'Visual C++', detail: 'Needed by most games made since 2015.', mb: 25, recommended: true, installed: true, cached: false },
  { id: 'vc2015-x86', name: 'Visual C++ 2015-2022 (32-bit)', group: 'Visual C++', detail: 'The same, for 32-bit games.', mb: 14, recommended: true, installed: true, cached: false },
  { id: 'vc2013-x86', name: 'Visual C++ 2013 (32-bit)', group: 'Visual C++', detail: 'Games from around 2014, 32-bit.', mb: 7, recommended: true, installed: false, cached: false },
  { id: 'vc2010-x86', name: 'Visual C++ 2010 (32-bit)', group: 'Visual C++', detail: 'Older games, 32-bit.', mb: 9, recommended: true, installed: false, cached: false },
  { id: 'directx', name: 'DirectX End-User Runtime (June 2010)', group: 'DirectX', detail: 'The extra DirectX 9, 10 and 11 files (d3dx9, XInput 1.3, XAudio 2.7) older games load. Windows does not include them.', mb: 1, recommended: true, installed: false, cached: false },
  { id: 'xna40', name: 'XNA Framework 4.0', group: 'XNA', detail: 'Games built with XNA, like Terraria and many indies from 2010 to 2016.', mb: 7, recommended: true, installed: false, cached: false },
  { id: 'dotnet8-desktop', name: '.NET 8 Desktop Runtime (64-bit)', group: '.NET', detail: 'Newer tools and games built on .NET, and many mod managers.', mb: 56, recommended: false, installed: false, cached: false },
  { id: 'netfx35', name: '.NET Framework 3.5', group: '.NET', detail: 'Games and launchers from the 2000s. A Windows feature: Windows Update supplies it, which can take a few minutes.', mb: 0, recommended: false, installed: true, cached: true },
]
let runtimesJob: Record<string, unknown> | null = null
let runtimesCancel = false
function previewRuntimesInstall(ids: string[]) {
  const chosen = RUNTIMES.filter((r) => ids.includes(r.id))
  runtimesCancel = false
  const results: { id: string; name: string; ok: boolean; detail: string }[] = []
  const emit = (p: Record<string, unknown>) => {
    runtimesJob = { current: null, step: 0, steps: chosen.length, fraction: 0, message: null, results: [...results], reboot: false, ...p }
    previewBus.emit('runtimes-progress', runtimesJob)
  }
  let t = 0
  const at = (ms: number, f: () => void) => setTimeout(() => !runtimesCancel && f(), (t += ms))
  chosen.forEach((r, i) =>
    [0.2, 0.55, 0.9, 1].forEach((f) => at(260, () => emit({ phase: 'downloading', current: r.name, step: i + 1, fraction: (0.4 * (i + f)) / chosen.length, message: `Downloading ${i + 1} of ${chosen.length}` }))),
  )
  at(900, () => emit({ phase: 'permission', fraction: 0.4, message: 'Windows is asking for permission' }))
  chosen.forEach((r, i) => {
    at(500, () => emit({ phase: 'installing', current: r.name, step: i + 1, fraction: 0.4 + (0.6 * i) / chosen.length, message: `Installing ${i + 1} of ${chosen.length}` }))
    at(900, () => {
      results.push({ id: r.id, name: r.name, ok: true, detail: 'Installed' })
      r.installed = true
      emit({ phase: 'installing', current: r.name, step: i + 1, fraction: 0.4 + (0.6 * (i + 1)) / chosen.length, message: `Installing ${i + 1} of ${chosen.length}` })
    })
  })
  at(300, () => emit({ phase: 'done', fraction: 1, message: 'All installed.' }))
}

/**
 * The friends list's actions against a copy of the sample friends, answered
 * the way kryo.to answers (`nobody` is an account that does not exist).
 */
let previewFriendsState: FriendsState | null = null
function previewFriendFetch(a: Record<string, unknown>): Record<string, unknown> {
  const s: FriendsState = (previewFriendsState ??= structuredClone(PREVIEW_FRIENDS))
  const method = String(a.method)
  const parts = String(a.path).split('/').slice(2)
  const body = (a.body ?? {}) as Record<string, unknown>
  const face = (f: { id: string; username: string; displayName: string | null; avatarUrl: string | null; supporter: boolean }) => ({ id: f.id, username: f.username, displayName: f.displayName, avatarUrl: f.avatarUrl, supporter: f.supporter })
  const befriend = (p: ReturnType<typeof face>) => s.friends.push({ ...p, favourite: false, nickname: null, muted: false, online: false })
  let answer: Record<string, unknown> = { ok: true }
  if (parts[0] === 'friends' && parts[1] === 'requests' && !parts[2]) {
    const username = String(body.username ?? '').toLowerCase()
    if (!username || username === 'nobody') throw new Error('Nobody is called that. Check the spelling.')
    if (s.friends.some((f) => f.username === username)) throw new Error('You are already friends.')
    const theirs = s.incoming.find((r) => r.username === username)
    if (theirs) {
      s.incoming = s.incoming.filter((r) => r !== theirs)
      befriend(face(theirs))
      answer = { status: 'friends' }
    } else {
      s.sent = [{ id: String(100 + s.sent.length), username, displayName: null, avatarUrl: null, supporter: false, requestId: String(200 + s.sent.length) }, ...s.sent]
      answer = { status: 'requested' }
    }
  } else if (parts[0] === 'friends' && parts[1] === 'requests') {
    const incoming = s.incoming.find((r) => r.requestId === parts[2])
    if (incoming && body.action === 'accept') befriend(face(incoming))
    s.incoming = s.incoming.filter((r) => r.requestId !== parts[2])
    s.sent = s.sent.filter((r) => r.requestId !== parts[2])
  } else if (parts[0] === 'friends' && parts[1] === 'invites') {
    answer = { url: 'https://kryo.to/friends/invite/preview-invite-link', expiresAt: new Date(Date.now() + 7 * 86_400_000).toISOString() }
  } else if (parts[0] === 'friends' && method === 'DELETE') {
    s.friends = s.friends.filter((f) => f.id !== parts[1])
  } else if (parts[0] === 'friends' && method === 'PATCH') {
    s.friends = s.friends.map((f) => (f.id === parts[1] ? { ...f, favourite: !!body.favourite } : f))
  } else if (parts[0] === 'blocks' && method === 'POST') {
    const gone = s.friends.find((f) => f.id === parts[1])
    s.friends = s.friends.filter((f) => f.id !== parts[1])
    if (gone) s.hidden = [...s.hidden, gone.username]
  } else if (parts[0] === 'mutes') {
    s.friends = s.friends.map((f) => (f.id === parts[1] ? { ...f, muted: method === 'POST' } : f))
  }
  s.outgoing = s.sent.length
  previewBus.emit('friends-state', structuredClone(s))
  return answer
}

const HANDLERS: Record<string, (a: Record<string, unknown>) => unknown> = {
  friends_fetch: (a) => previewFriendFetch(a),
  store_refresh_account: () => null,
  // Community updates (src-tauri community_offer.rs): the first game has a newer build, half played.
  community_offer_state: (a) => (a.id === games[0]?.id
    ? { shown: true, toVersion: '1.4.2', kryotoVersion: '1.4.0', playedSeconds: 420, needSeconds: 600, exitCode: 0, shots: [], autoShots: true, offerId: null, offerState: null, reasons: [] }
    : { shown: false }),
  // The second game: Steam has a newer build to try.
  community_update_check: (a) => (a.id === games[1]?.id ? { available: true, steamBuildId: '24935044', kryotoBuildId: '19870421', reason: null } : { available: false }),
  community_update_try: () => ({ shown: true, toVersion: 'Steam build 24935044', kryotoVersion: '1.0', playedSeconds: 0, needSeconds: 600, exitCode: null, shots: [], autoShots: true, offerId: null, offerState: null, reasons: [] }),
  community_offer_add_shot: () => ({ shown: true, toVersion: '1.4.2', kryotoVersion: '1.4.0', playedSeconds: 720, needSeconds: 600, exitCode: 0, shots: ['shot.png'], autoShots: true, offerId: null, offerState: null, reasons: [] }),
  community_offer_send: () => ({ shown: true, toVersion: '1.4.2', kryotoVersion: '1.4.0', playedSeconds: 720, needSeconds: 600, exitCode: 0, shots: ['shot.png'], autoShots: true, offerId: '1', offerState: 'review', reasons: [] }),
  // Cloud saves (src-tauri saves/): one game with every kind of location, a conflict on another.
  cloud_saves_status: () => ({ feature: true, on: true, perk: true, allowance: { totalBytes: GB, versionsPerGame: 5, maxBackupBytes: 100 * 1_048_576 }, usage: { bytes: 214_000_000, games: 7, versions: 23 }, lastSync: now() - 300, conflicts: [], endpoint: ownHost('savesEndpoint'), mode: settings.savesMode === 'local' ? 'local' : ownHost('savesEndpoint') ? 'patchbay' : 'account', localDir: (settings.savesLocalDir as string) || null }),
  cloud_saves_refresh: () => HANDLERS.cloud_saves_status!({}),
  cloud_saves_library: () => games.slice(0, 4).map((g, i) => ({ id: g.id, title: g.title, lastBackupAt: i === 1 ? null : now() - i * 3600, conflict: i === 1, error: null, tooBig: i === 2 })),
  cloud_saves_game: (a) => {
    const index = games.findIndex((g) => g.id === a.id)
    const conflict = index === 1
    // The third game is one whose folder holds downloaded maps: too big.
    const big = index === 2
    const loc = (source: string, sourceLabel: string, label: string, files: number, bytes: number, extra: Record<string, unknown> = {}) => ({ key: `${source}:${label}`, kind: 'files', root: 'Documents', label, source, sourceLabel, path: `C:\\Users\\you\\${label}`, exists: files > 0, files, bytes, values: 0, score: null, reasons: [], leftOut: [], ...extra })
    return {
      identity: { key: 'steam:367520', aliases: ['steam:367520'], appid: '367520' },
      feature: true, on: true, perk: true,
      // The first game was found precisely; the second asks next time; the rest offer it.
      finding: { mode: index === 1 ? 'precise' : 'smart', canPrecise: true, preciseNext: index === 1, preciseFound: index === 0, preciseDeclined: false, preciseSessions: index === 0 ? 1 : 0 },
      locations: [
        ...(index === 0 ? [loc('detected', 'Precise', 'AppData\\Roaming\\Team Cherry\\Hollow Knight\\profiles', 2, 64_000, { score: 85, reasons: ['the game wrote here', 'named after the game'] })] : []),
        loc('emulator', 'Emulator', 'AppData\\Roaming\\GSE Saves\\367520', 3, 48_000),
        loc('known', 'Known', 'AppData\\LocalLow\\Team Cherry\\Hollow Knight', big ? 480 : 6, big ? 438_000_000 : 1_900_000, big ? {} : { leftOut: ['mapcache'] }),
        loc('engine', 'Engine', 'HKEY_CURRENT_USER\\Software\\Team Cherry\\Hollow Knight', 0, 0, { kind: 'registry', path: null, values: 35, exists: true }),
        loc('emulator', 'Emulator', 'AppData\\Roaming\\Goldberg SteamEmu Saves\\367520', 0, 0),
        loc('emulator', 'Emulator', 'Public Documents\\Steam\\RUNE\\367520', 0, 0),
      ],
      suggestions: [loc('detected', 'Detected', 'Documents\\My Games\\Hollow Knight', 2, 4_096, { score: 35, reasons: ["named in the game's files"] })],
      ignored: [loc('known', 'Known', 'AppData\\Local\\Hollow Knight', 0, 0)],
      heavy: big
        ? [
            { location: 'known:AppData\\LocalLow\\Team Cherry\\Hollow Knight', folder: 'maps', label: 'AppData\\LocalLow\\Team Cherry\\Hollow Knight\\maps', files: 18, bytes: 390_000_000 },
            { location: 'known:AppData\\LocalLow\\Team Cherry\\Hollow Knight', folder: 'Saves', label: 'AppData\\LocalLow\\Team Cherry\\Hollow Knight\\Saves', files: 6, bytes: 1_200_000 },
          ]
        : [],
      files: big ? 483 : 9, bytes: big ? 438_048_000 : 1_948_000, registryValues: 35, truncated: false, maxBackupBytes: 100 * 1_048_576,
      state: { baseVersion: 'v3', baseHash: 'x', lastBackupAt: now() - 3600, lastRestoreAt: null, cloudHead: conflict ? { id: 'v4', size: 1_200_000, sha256: '', md5: '', fileCount: 9, machine: 'LIVING-ROOM-PC', createdAt: now() - 7200, parentId: 'v3' } : null, conflict, error: big ? "This game's saves are 257 MB packed, over the 100 MB a backup can be." : null, tooBig: big },
      localBackups: [{ file: `${now() - 86400}.ksave`, createdAt: now() - 86400, size: 1_700_000 }],
    }
  },
  cloud_saves_location: () => null,
  cloud_saves_leave_out: () => null,
  cloud_saves_find_precisely: () => null,
  cloud_saves_add_folder: () => null,
  // Plays a backup's phases on the card, as the native side reports them.
  cloud_saves_backup: async (a) => {
    const step = (phase: string, done: number, total: number) => previewBus.emit('cloud-saves-progress', { gameId: a.id, phase, done, total })
    const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))
    step('scanning', 0, 0)
    await wait(800)
    for (let i = 0; i <= 10; i++) { step('packing', i * 4_200_000, 42_000_000); await wait(150) }
    for (let i = 0; i <= 10; i++) { step('uploading', i * 2_600_000, 26_000_000); await wait(250) }
    step('checking', 0, 0)
    await wait(600)
    step('done', 0, 0)
    return 'Backed up.'
  },
  cloud_saves_versions: () => [3, 2, 1].map((n) => ({ id: `v${n}`, size: 1_900_000 - n * 1000, sha256: '', md5: '', fileCount: 9, machine: n === 2 ? 'LIVING-ROOM-PC' : 'DESKTOP-MIRA', createdAt: now() - n * 86400, parentId: n > 1 ? `v${n - 1}` : null })),
  cloud_saves_restore: () => 'Saves restored. What was here before is kept under Versions, On this PC.',
  cloud_saves_restore_local: () => 'Saves put back from this PC\'s backup.',
  cloud_saves_answer: () => null,
  cloud_achievements_status: () => ({ perk: true, games: 3, lastSync: Math.floor(Date.now() / 1000) - 120, endpoint: ownHost('achievementsEndpoint'), mode: settings.achievementsMode === 'local' ? 'local' : ownHost('achievementsEndpoint') ? 'patchbay' : 'account', localDir: (settings.savesLocalDir as string) || null }),
  patchbay_status: () => ({ saves: { endpoint: (settings.savesEndpoint as string) || null, token: false }, achievements: { endpoint: (settings.achievementsEndpoint as string) || null, token: false }, debrid: (settings.debrid as string) || null, debridToken: false, torboxToken: false }),
  patchbay_set_token: (a) => ({
    saves: { endpoint: null, token: a.slot === 'saves' && !!a.token },
    achievements: { endpoint: null, token: a.slot === 'achievements' && !!a.token },
    debrid: (settings.debrid as string) || null,
    debridToken: a.slot === 'real-debrid' && !!a.token,
    torboxToken: a.slot === 'torbox' && !!a.token,
  }),
  patchbay_test: (a) => (a.slot === 'real-debrid' ? 'Signed in as preview: premium, 120 days left.' : a.slot === 'torbox' ? 'Signed in as preview@example.com: paid plan until 2027-01-31.' : 'Connected. 3 games stored there.'),
  runtimes_list: () => RUNTIMES.map((r) => ({ ...r })),
  runtimes_install: (a) => previewRuntimesInstall(a.ids as string[]),
  runtimes_progress: () => runtimesJob,
  runtimes_cancel: () => {
    runtimesCancel = true
    runtimesJob = { ...runtimesJob, phase: 'canceled', current: null, message: 'Canceled. Nothing was installed.' }
    previewBus.emit('runtimes-progress', runtimesJob)
  },
  runtimes_clear_cache: () => 64_000_000,
  chat_status: () => previewChat,
  chat_set_context: () => null,
  chat_conversations: () => {
    previewSeed()
    const peers = [...new Set(previewMsgs.map((m) => m.conversationId))]
    return peers.map((c) => {
      const list = previewMsgs.filter((m) => m.conversationId === c)
      const last = list[list.length - 1] as PreviewMsg
      return { id: c, peerUserId: last.outgoing ? '2' : last.senderUser, lastAt: last.sentAt, unread: list.filter((m) => !m.outgoing && m.status === 'received').length, last }
    })
  },
  chat_messages: (a) => {
    previewSeed()
    return previewMsgs.filter((m) => m.conversationId === convOf(String(a.peer)))
  },
  chat_mark_read: (a) => {
    for (const m of previewMsgs) if (m.conversationId === convOf(String(a.peer)) && !m.outgoing) m.status = 'read'
    return null
  },
  chat_typing: () => null,
  chat_settings_get: () => previewSettings,
  chat_settings_set: (a) => {
    previewSettings = a.settings as typeof previewSettings
    return null
  },
  chat_gif_search: () => ({ gifs: [], attribution: 'Powered by GIPHY' }),
  chat_message_request: (a) => {
    const username = String(a?.username ?? '').replace(/^@/, '')
    if (username === 'nobody') throw new Error(`Nobody is called ${username}.`)
    return { status: 'requested', userId: '77', username, name: username }
  },
  chat_message_request_respond: () => null,
  chat_verify_info: (a) =>
    String(a.peer) === '3'
      ? { safetyNumber: [], verified: false, pendingChange: true }
      : { safetyNumber: ['05213', '88410', '29371', '64102', '77215', '30981', '45120', '99832', '11046', '58723', '20419', '67310'], verified: previewVerified.has(String(a.peer)), pendingChange: false },
  chat_verify_mark: (a) => {
    if (a.verified) previewVerified.add(String(a.peer))
    else previewVerified.delete(String(a.peer))
    return null
  },
  chat_identity_ack: () => null,
  chat_backup_status: () => previewBackup,
  chat_backup_create: () => {
    previewBackup = { exists: true, updatedAtMs: Date.now(), keptHere: true }
    return '7K2M-9QXR-4HWD-P8TC-3NVB-6YJF-1GSE-5ZAQ-0RKD-8MXT-2HPW-4CNV-B000'
  },
  chat_backup_delete: () => {
    previewBackup = { exists: false, updatedAtMs: 0, keptHere: false }
    return null
  },
  chat_devices: () => [
    { deviceId: '2', kind: 'desktop', name: 'GAMING-PC', certified: true, createdAtMs: Date.now() - 40 * 86_400_000, lastSeenDayMs: Date.now(), current: true },
    { deviceId: '9', kind: 'desktop', name: 'LAPTOP', certified: true, createdAtMs: Date.now() - 9 * 86_400_000, lastSeenDayMs: Date.now() - 3 * 86_400_000, current: false },
  ],
  chat_device_revoke: () => null,
  chat_unlock: () => previewChat,
  chat_restore: () => null,
  chat_reset_identity: () => null,
  chat_send_invite: (a) => {
    const inv = a.invite as Record<string, unknown>
    const m: PreviewMsg = { msgId: newId(), conversationId: convOf(String(a.peer)), senderUser: '1', outgoing: true, sentAt: Date.now(), receivedAt: Date.now(), kind: 'invite', body: JSON.stringify({ ...inv, expiresAt: Date.now() + 900_000 }), replyTo: null, editedAt: null, deleted: false, status: 'sent', reactions: [] }
    previewMsgs.push(m)
    previewBus.emit('chat-message', m)
    return m
  },
  online_lobby: () => ({ lobby: '109775241075364289', hostSteamId: '76561198000000000' }),
  steam_join_lobby: () => null,
  chat_report: () => null,
  chat_send_file: (a) => {
    const m: PreviewMsg = { msgId: newId(), conversationId: convOf(String(a.peer)), senderUser: '1', outgoing: true, sentAt: Date.now(), receivedAt: Date.now(), kind: 'file', body: JSON.stringify({ name: 'tall-screenshot.svg', mime: 'image/svg+xml', size: 2048, width: 400, height: 800 }), replyTo: null, editedAt: null, deleted: false, status: 'sent', reactions: [] }
    previewMsgs.push(m)
    return m
  },
  room_list: (a) => ({ canModerate: false, messages: a.after ? previewRoom.filter((m) => Number(m.id) > Number(a.after)) : previewRoom }),
  room_post: (a) => {
    const to = previewRoom.find((m) => m.id === a.replyTo)
    previewRoom.push({
      id: String(previewRoom.length + 1),
      body: String(a.text),
      replyTo: to ? { id: to.id, gone: false, name: to.user.displayName, username: to.user.username, excerpt: to.body } : null,
      createdAt: new Date().toISOString(),
      user: { id: '1', username: 'mira', displayName: 'Mira', avatarUrl: null, supporter: false },
    })
    return {}
  },
  room_delete: () => null,
  chat_call_signal: () => null,
  chat_ice_servers: () => ({ iceServers: [{ urls: 'stun:stun.cloudflare.com:3478' }] }),
  chat_file_save: () => null,
  // Every second file fails, for the message that says so.
  chat_file_preview: (a) => {
    const nth = previewMsgs.filter((m) => m.kind === 'file').findIndex((m) => m.msgId === a.msg)
    if (nth % 2 === 1) throw new Error('This picture could not be fetched.')
    return `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="400" height="800"><rect width="400" height="800" fill="#7fd3ff"/><circle cx="200" cy="400" r="150" fill="#b26bff"/></svg>')}`
  },
  chat_groups: () => previewGroups,
  chat_group_create: (a) => {
    const g = { id: String(1000 + previewGroups.length), name: String(a.name), members: [{ userId: '1', role: 'owner' }, ...(a.members as string[]).map((userId) => ({ userId, role: 'member' }))], myRole: 'owner' }
    previewGroups = [g, ...previewGroups]
    return g
  },
  chat_group_rename: () => null,
  chat_group_add: (a) => previewGroups.find((g) => g.id === a.group),
  chat_group_remove: (a) => {
    previewGroups = previewGroups.filter((g) => g.id !== a.group)
    return null
  },
  chat_people: () => ({ people: [{ id: '8', username: 'rook', displayName: 'Rook', avatarUrl: null, supporter: false }] }),
  // Mentions (src/rich): the room's `@` list, names for ids the chat does not
  // know (anyone else reads as anonymous here), and the notify call.
  chat_mention_search: (a) => ({
    people: [
      { id: '2', username: 'bo', displayName: 'Bo', avatarUrl: null },
      { id: '8', username: 'rook', displayName: 'Rook', avatarUrl: null },
    ].filter((p) => p.username.includes(String(a.query ?? '').toLowerCase())),
  }),
  chat_mention_lookup: (a) => ({ people: Object.fromEntries((a.ids as string[]).map((id) => [id, { state: 'anonymous' }])) }),
  chat_mention_notify: () => null,
  chat_export_history: () => 'C:\Users\you\Documents\kryoto-chat-history.json',
  chat_send: (a) => {
    const peer = String(a.peer)
    const m: PreviewMsg = { msgId: newId(), conversationId: convOf(peer), senderUser: '1', outgoing: true, sentAt: Date.now(), receivedAt: Date.now(), kind: 'text', body: String(a.text), replyTo: (a.replyTo as string) ?? null, editedAt: null, deleted: false, status: 'sent', reactions: [] }
    previewMsgs.push(m)
    setTimeout(() => { m.status = 'delivered'; update(m) }, 600)
    setTimeout(() => previewBus.emit('chat-typing', { conversationId: m.conversationId, userId: peer, active: true }), 900)
    setTimeout(() => {
      m.status = 'read'
      update(m)
      const reply: PreviewMsg = { ...m, msgId: newId(), senderUser: peer, outgoing: false, sentAt: Date.now(), receivedAt: Date.now(), body: 'sounds good', replyTo: null, status: 'received', reactions: [] }
      previewMsgs.push(reply)
      previewBus.emit('chat-message', reply)
    }, 2200)
    return m
  },
  chat_edit: (a) => {
    const m = previewMsgs.find((x) => x.msgId === a.msg)
    if (!m) throw new Error('No such message.')
    m.body = String(a.text)
    m.editedAt = Date.now()
    return m
  },
  chat_delete: (a) => {
    const m = previewMsgs.find((x) => x.msgId === a.msg)
    if (m) { m.deleted = true; m.body = ''; m.reactions = []; update(m) }
    return null
  },
  chat_react: (a) => {
    const m = previewMsgs.find((x) => x.msgId === a.msg)
    if (!m) return null
    const emoji = String(a.emoji)
    let r = m.reactions.find((x) => x.emoji === emoji)
    if (a.remove) { if (r) r.userIds = r.userIds.filter((u) => u !== '1'); m.reactions = m.reactions.filter((x) => x.userIds.length) }
    else { if (!r) { r = { emoji, userIds: [] }; m.reactions.push(r) } if (!r.userIds.includes('1')) r.userIds.push('1') }
    update(m)
    return null
  },
  chat_enable: () => {
    previewChat = { state: 'connecting' }
    setTimeout(() => {
      previewChat = { state: 'online', userId: '1', deviceId: '2' } as { state: string }
      previewBus.emit('chat-status', previewChat)
    }, 800)
    return previewChat
  },
  chat_remove_device: () => {
    previewChat = { state: 'off' }
    previewBus.emit('chat-status', previewChat)
  },
  compat_status: () => ({
    tools: [
      { name: 'GE-Proton10-17', path: '/home/you/.local/share/to.kryo.desktop/compat/GE-Proton10-17/proton', kind: 'proton', managed: true },
      { name: 'Proton 9.0 (Beta)', path: '/home/you/.steam/steam/steamapps/common/Proton 9.0 (Beta)/proton', kind: 'proton', managed: false },
      { name: 'umu-run', path: '/home/you/.local/share/to.kryo.desktop/compat/umu/umu-run', kind: 'umu', managed: true },
      { name: 'wine', path: '/usr/bin/wine', kind: 'wine', managed: false },
    ],
    umu: '/home/you/.local/share/to.kryo.desktop/compat/umu/umu-run',
    mangohud: true,
    gamemode: false,
  }),
  library_list: () => clone(games),
  library_set_hidden: (a) => {
    for (const g of games) if ((a.ids as string[]).includes(String(g.id))) g.hidden = a.hidden
    previewBus.emit('library-changed', null)
  },
  library_artwork: (a) => {
    const game = games.find((g) => g.id === a.id)
    if (!game) throw new Error('Game not found.')
    for (const key of ['cover', 'hero', 'logo', 'header']) if (a[key]) game[key] = a[key]
    return clone(game)
  },
  game_running: () => [...running],
  library_save: (a) => {
    const next = a.game as Record<string, unknown>
    const i = games.findIndex((g) => g.id === next.id)
    const old = games[i]
    if (!old) return clone(next)
    games[i] = { ...next, playtimeSeconds: old.playtimeSeconds, lastPlayed: old.lastPlayed }
    return clone(games[i])
  },
  library_add: (a) => {
    const g = { ...(a.game as Record<string, unknown>) }
    const path = String(a.exePath)
    g.id = String(g.slug || g.title || 'game').toLowerCase().replace(/[^a-z0-9]+/g, '-')
    g.installDir = path.replace(/[\\/][^\\/]+$/, '')
    g.executable = path.split(/[\\/]/).pop()
    g.addedAt = now()
    games.push(g)
    return clone(g)
  },
  library_remove: (a) => {
    const i = games.findIndex((g) => g.id === a.id)
    if (i >= 0) games.splice(i, 1)
    return false
  },
  game_launch: (a) => {
    const id = String(a.id)
    running.add(id)
    const g = games.find((x) => x.id === id)
    if (g) g.lastPlayed = now()
    previewBus.emit('game-state', { id, running: true, seconds: null, code: null })
    setTimeout(() => {
      running.delete(id)
      if (g) g.playtimeSeconds = (g.playtimeSeconds as number) + 600
      previewBus.emit('game-state', { id, running: false, seconds: 600, code: 0 })
    }, 6000)
    return null
  },
  game_stop: (a) => {
    running.delete(String(a.id))
    previewBus.emit('game-state', { id: a.id, running: false, seconds: 60, code: 1 })
    return null
  },
  game_launch_preview: (a) => {
    const g = a.game as { installDir: string; executable: string; entries: { executable: string; arguments: string }[]; launchOptions: string; defaultArgs: string }
    const e = a.entry != null ? g.entries[a.entry as number] : null
    const exe = e ? e.executable : g.executable
    const args = [e ? e.arguments : g.defaultArgs, g.launchOptions.replace(/^.*%command%/, '').trim()].filter(Boolean).join(' ')
    return `"${g.installDir}\\${exe}"${args ? ` ${args}` : ''}`
  },
  game_disk_size: () => 14_300_000_000,
  open_folder: () => null,
  game_languages: () => ['english', 'french', 'german', 'spanish'],
  game_shortcut: () => 'desktop',
  settings_get: () => ({ ...settings }),
  // Every sample game can use Kryoto Online, so the add-ons card shows it.
  online_check: () => null,
  settings_save: (a) => ({ ...Object.assign(settings, a.settings) }),
  downloads_list: () => clone(downloadList),
  storage_overview: () => ({
    folders: [
      {
        path: String(settings.libraryDir),
        drive: 'C:',
        isDefault: true,
        exists: true,
  addons_list: (a) => clone(previewAddons[String(a.slug)] ?? []),
  // Add on a game's Add-ons & DLC card: a row per add-on, for that game, minted at its turn.
  addons_get: (a) => {
    const g = games.find((x) => x.id === a.gameId)
    if (!g) throw new Error('That game is no longer in the library.')
    if (running.has(String(g.id))) throw new Error(`Close ${String(g.title)} first: its files are in use.`)
    const challenge = typeof location !== 'undefined' && location.search.includes('addonchallenge')
    const queued: string[] = []
    for (const want of a.ids as string[]) {
      const addon = (previewAddons[String(g.slug)] ?? []).find((x) => String(x.id) === want)
      if (!addon) continue
      const same = downloadList.find((d) => d.addonRelease === want && d.slug === g.slug && d.status !== 'installed' && d.status !== 'canceled')
      if (same) {
        queued.push(String(same.id))
        continue
      }
      const id = `dl-${Date.now()}-addon-${want}`
      const total = sizeOf(addon.download_size)
      const item = dl({
        id,
        slug: g.slug,
        gameId: g.id,
        url: '',
        fileName: (addon.links as { name: string }[])[0]?.name ?? `${String(addon.label)} - Kryoto.7z`,
        status: 'queued',
        received: 0,
        total,
        speed: 0,
        addedAt: now() + queued.length,
        addon: addon.label,
        addonRelease: want,
        repair: addon.kind === 'repair',
        meta: { ...dl({}).meta, title: /^DLC: /.test(String(addon.label)) ? `${String(g.title)} DLC - ${String(addon.label).slice(5)}` : `${String(g.title)} - ${String(addon.label)}`, cover: g.cover, hero: g.hero, version: g.version, sizeBytes: null },
      })
      downloadList.unshift(item)
      queued.push(id)
      // One at a time, as the native queue runs them: each starts when the one before is done.
      const begin = () => {
        if (downloadList.some((d) => d !== item && ['resolving', 'downloading', 'extracting'].includes(String(d.status)) && d.addon)) return setTimeout(begin, 700)
        item.status = 'resolving'
        previewBus.emit('downloads', clone(downloadList))
        setTimeout(() => {
          if (challenge) {
            Object.assign(item, { status: 'failed', error: 'kryo.to wants a check only its download window can pass for this account. Open the download window to add it there.' })
          } else {
            Object.assign(item, { status: 'downloading', speed: 30_000_000 })
          }
          previewBus.emit('downloads', clone(downloadList))
        }, 700)
      }
      setTimeout(begin, 400)
    }
    previewBus.emit('downloads', clone(downloadList))
    return { queued, store: false }
  },
  addon_undo: (a) => {
    const g = games.find((x) => x.id === a.gameId)
    if (!g) throw new Error('That game is no longer in the library.')
    if (running.has(String(g.id))) throw new Error('Close the game first.')
    g.addons = ((g.addons as Record<string, unknown>[] | undefined) ?? []).filter((x) => x.file !== a.file)
    previewBus.emit('library-changed', null)
    return clone(g)
  },
        total: 953 * GB,
        free: 212 * GB,
        games: [stored('captain-hardcore', 22.1 * GB), stored('stardew-valley', 0.6 * GB), stored('celeste', 1.2 * GB)],
      },
      { path: 'D:\\Games\\Kryoto', drive: 'D:', isDefault: false, exists: true, total: 1863 * GB, free: 1204 * GB, games: [] },
    ],
    elsewhere: [stored('terraria', 0.4 * GB), ...Array.from({ length: 24 }, (_, i) => ({ ...stored('hades', 0), id: `steam-${i}`, title: `Steam game ${i + 1}`, blocked: 'Steam manages this game.', launcher: 'steam' }))],
  }),
  storage_add_folder: (a) => {
    ;(settings.libraryFolders as string[]).push(String(a.path))
    return null
  },
  storage_remove_folder: () => null,
  storage_set_default: () => null,
  storage_folders: () => [
    { path: String(settings.libraryDir), drive: 'C:', isDefault: true, exists: true, total: 953 * GB, free: 212 * GB, games: [] },
    ...(settings.libraryFolders as string[]).map((path, i) => ({ path, drive: i ? 'E:' : 'D:', isDefault: false, exists: true, total: 1863 * GB, free: 1204 * GB, games: [] })),
  ],
  // Two folders' worth of lost installs (library.rs `library_find_kryoto_games`).
  library_find_kryoto_games: () => [
    { title: 'Hollow Knight', installDir: 'D:\\Games\\Kryoto\\Hollow Knight', cover: null, nsfw: false, playtimeSeconds: 41 * 3600 },
    { title: 'Hades II', installDir: 'D:\\Games\\Kryoto\\Hades II', cover: null, nsfw: false, playtimeSeconds: 0 },
  ],
  library_restore_found: (a) => (a.dirs as string[]).length,
  collections_state: () => clone(collectionsPreview),
  collection_install: (a) => {
    const c = collectionsPreview.collections.find((x) => x.id === a.id)
    if (!collectionsPreview.installed.includes(String(a.id))) collectionsPreview.installed.push(String(a.id))
    collectionsPreview.updates = collectionsPreview.updates.filter((u) => u.id !== a.id)
    for (const slug of a.slugs as string[]) {
      const g = c?.games.find((x) => x.slug === slug)
      if (g) downloadList.unshift(dl({ id: `dl-${slug}`, slug, status: 'queued', received: 0, total: g.sizeBytes, speed: 0, meta: { ...dl({}).meta, title: g.title, cover: g.cover }, libraryDir: a.folder ?? null }))
    }
    previewBus.emit('downloads', clone(downloadList))
    previewBus.emit('collections-state', clone(collectionsPreview))
    return (a.slugs as string[]).length
  },
  collection_later: (a) => {
    collectionsPreview.updates = collectionsPreview.updates.filter((u) => u.id !== a.id)
    previewBus.emit('collections-state', clone(collectionsPreview))
    return null
  },
  collection_forget: (a) => {
    collectionsPreview.installed = collectionsPreview.installed.filter((i) => i !== a.id)
    collectionsPreview.updates = collectionsPreview.updates.filter((u) => u.id !== a.id)
    previewBus.emit('collections-state', clone(collectionsPreview))
    return null
  },
  storage_move: () => null,
  logs_tail: () => LOG.join('\n'),
  logs_folder: () => 'C:\\Users\\you\\AppData\\Roaming\\to.kryo.desktop\\logs',
  logs_send: () => 1,
  phone_status: () => ({ ...phone, addresses: phone.lan ? ['192.168.1.20'] : [], codeExpiresIn: phone.code ? 300 : null, devices: [] }),
  phone_set_lan: (a) => {
    phone.lan = !!a.on
    return null
  },
  phone_new_code: () => {
    phone.code = '123456'
    phone.pairUri = 'kryoto://pair?code=123456'
    return phone.code
  },
  phone_revoke: () => null,
  // Remote (remote.rs): off until turned on, as for a fresh account.
  remote_status: () => ({ ...remoteConsent }),
  remote_set_consent: (a) => {
    Object.assign(remoteConsent, { enabled: a.enabled === true, decided: true })
    setTimeout(() => {
      previewBus.emit('remote-consent', { ...remoteConsent })
      previewBus.emit('remote-consent-done', { ok: true, error: null })
    }, 300)
    return null
  },
  log_write: () => null,
  shell_ready: () => null,
  download_pause: (a) => {
    const d = downloadList.find((x) => x.id === a.id)
    if (d) Object.assign(d, { status: 'paused', speed: 0 })
    previewBus.emit('downloads', clone(downloadList))
    return null
  },
  download_install_anyway: (a) => {
    const d = downloadList.find((x) => x.id === a.id)
    if (d) Object.assign(d, { status: 'extracting', hashMismatch: false, error: null })
    previewBus.emit('downloads', clone(downloadList))
    return null
  },
  download_resume: (a) => {
    const d = downloadList.find((x) => x.id === a.id)
    if (d) Object.assign(d, d.hashMismatch ? { status: 'downloading', hashMismatch: false, received: 0, error: null } : { status: 'downloading' })
    previewBus.emit('downloads', clone(downloadList))
    return null
  },
  download_pick_folder: (a) => {
    const d = downloadList.find((x) => x.id === a.id)
    if (a.alwaysDefault) settings.askInstallFolder = false
    if (d) Object.assign(d, { askFolder: false, libraryDir: a.folder === settings.libraryDir ? null : a.folder, status: 'downloading', speed: 36_000_000 })
    previewBus.emit('downloads', clone(downloadList))
    return null
  },
  download_cancel: (a) => {
    const d = downloadList.find((x) => x.id === a.id)
    if (d) Object.assign(d, { status: 'canceled', speed: 0, received: 0, askFolder: false })
    previewBus.emit('downloads', clone(downloadList))
    return null
  },
  download_remove: (a) => {
    const i = downloadList.findIndex((x) => x.id === a.id)
    if (i >= 0) downloadList.splice(i, 1)
    previewBus.emit('downloads', clone(downloadList))
    return null
  },
}

/**
 * Controllers: a DualSense SDL knows, and a SteelSeries pad it does not (as
 * Windows reports one: a placeholder name and no mapping), which the setup
 * is for. An Xbox pad was seen before.
 */
const previewPads = [
  {
    id: 0,
    guid: '030000004c050000e60c000011810000',
    name: 'DualSense',
    systemName: 'PS5 Controller',
    vendor: 0x054c,
    product: 0x0ce6,
    model: { family: 'playstation', name: 'DualSense', features: { touchpad: true, gyro: true, paddles: 0, misc: true, analogTriggers: true }, matched: 'ids' },
    mapped: true,
    custom: false,
    xinput: false,
    rumble: true,
    battery: 80,
    charging: false,
    buttons: 15,
    axes: 6,
    hats: 0,
  },
  {
    id: 1,
    guid: '03008231110100003414000000000000',
    name: 'SteelSeries controller',
    systemName: 'As: 6 knop: 16 gamepad met kapschakelaar',
    vendor: 0x0111,
    product: 0x1434,
    model: { family: 'xbox', name: 'SteelSeries controller', features: { touchpad: false, gyro: false, paddles: 0, misc: false, analogTriggers: true }, matched: 'maker' },
    mapped: false,
    custom: false,
    xinput: false,
    rumble: false,
    battery: null,
    charging: false,
    buttons: 16,
    axes: 6,
    hats: 1,
  },
]
let previewPadConfig = {
  enabled: true,
  navigate: true,
  notify: false,
  games: true,
  haptics: true,
  virtualPad: true,
  pads: {
    '030000004c050000e60c000011810000': { name: 'DualSense', family: null, remap: {}, mapping: null, base: null, sdl: null },
    '03008231110100003414000000000000': { name: 'SteelSeries controller', family: null, remap: {}, mapping: null, base: null, sdl: null },
    '030000005e040000120b00000b050000': { name: 'Xbox Series X|S Controller', family: null, remap: { a: 'b', b: 'a' }, mapping: null, base: null, sdl: null },
  } as Record<string, Record<string, unknown>>,
}
const previewRepairs = new Map<string, { source: unknown; online: unknown; report: { game: { title: string; slug: string }; source: string; version: string; warnings: string[] } }>()

Object.assign(HANDLERS, {
  pad_start: () => { previewBus.emit('pad-list', clone(previewPads)); return previewPads },
  pad_stop: () => null,
  pad_list: () => previewPads,
  pad_config_get: () => previewPadConfig,
  pad_config_set: (a: Record<string, unknown>) => (previewPadConfig = a.config as typeof previewPadConfig),
  pad_rumble: () => null,
  pad_haptic: () => null,
  pad_capture: (a: Record<string, unknown>) => {
    if (a.id != null) setTimeout(() => previewBus.emit('pad-raw-state', { id: a.id, axes: [0, 0, 0, 0, -1, -1], hats: [0] }), 30)
    return null
  },
  pad_mapping_set: (a: Record<string, unknown>) => {
    const guid = String(a.guid)
    const prefs = previewPadConfig.pads[guid] ?? {}
    const line = a.bindings ? `${guid},${String(a.name)},${Object.entries(a.bindings as Record<string, string>).map(([k, v]) => `${k}:${v}`).join(',')},` : null
    previewPadConfig = { ...previewPadConfig, pads: { ...previewPadConfig.pads, [guid]: { ...prefs, mapping: line } } }
    const pad = previewPads.find((p) => p.guid === guid)
    if (pad) {
      pad.mapped = !!line || pad.id === 0
      pad.custom = !!line
      previewBus.emit('pad-list', clone(previewPads))
    }
    return previewPadConfig
  },
  pad_virtual_driver: () => false,
  // pad_games.rs: a game following the shared default, one layout to pick.
  pad_game_get: () => {
    const profile = { mode: 'keyboard', template: 'shooter', remap: {}, binds: { righttrigger: 'mouse1', a: 'space' }, leftStick: 'wasd', rightStick: 'mouse', sensitivity: 1600, deadzone: 0.15, invertY: false, pads: {} }
    return {
      profile,
      own: false,
      default: profile,
      antiCheat: null,
      outputProblem: null,
      runAsAdmin: false,
      templates: [{ id: 'shooter', label: 'First-person shooter', hint: 'Left stick walks, right stick aims, triggers aim and fire.' }],
      keys: [{ name: 'mouse1', label: 'Left click' }, { name: 'space', label: 'Space' }],
      layouts: { shooter: profile },
    }
  },
  pad_game_set: () => undefined,
  pad_game_default_set: () => undefined,
  // Two controllers on one game: an Xbox pad and a DualSense.
  pad_game_seen: () => ({
    summary: 'The game sees one keyboard and one mouse, not separate players: every controller presses the same keys and moves the same mouse. For a game where two people share a keyboard, give one controller its own keys.',
    pads: [
      { key: 'xbox', name: 'Xbox Series X|S Controller', sees: 'Presses the keys below.', player: null, removed: false, canRemove: true, own: false, canOwn: true },
      { key: 'sense', name: 'DualSense', sees: 'Presses the keys below.', player: null, removed: false, canRemove: true, own: false, canOwn: true },
    ],
  }),
  pad_game_configured: () => ['celeste'],
  // vr.rs: a PC with SteamVR and Quest Link, SteamVR not running yet.
  vr_status: () => ({
    runtimes: [
      { id: 'steamvr', name: 'SteamVR', manifest: 'C:\\Program Files (x86)\\Steam\\steamapps\\common\\SteamVR\\steamxr_win64.json', starter: 'C:\\Program Files (x86)\\Steam\\steamapps\\common\\SteamVR\\bin\\win64\\vrstartup.exe', active: true },
      { id: 'meta', name: 'Meta Quest Link', manifest: 'C:\\Program Files\\Meta Horizon\\Support\\oculus-runtime\\oculus_openxr_64.json', starter: null, active: false },
    ],
    steamvrInstalled: true,
    steamvrRunning: false,
    experimental: false,
  }),
  game_launch_vr: () => undefined,
  // defender.rs: Defender on, exclusions hidden without administrator; adding asks UAC, here declined.
  defender_status: () => ({ supported: true, realtime: true, passive: false, others: [], excluded: null, folders: ['C:\\Kryoto Games', 'D:\\Games'], drives: ['D:\\'] }),
  defender_script: (a: { kind?: string }) => `$ErrorActionPreference = 'Stop'\n$out = '%TEMP%\\kryoto-defender.json'\ntry {\n    Add-MpPreference -ExclusionPath @(${(a.kind === 'drives' ? ['D:\\'] : ['C:\\Kryoto Games', 'D:\\Games']).map((p) => `'${p}'`).join(', ')})\n    ...\n}`,
  defender_exclude: () => ({ ok: false, reason: 'declined', message: 'The administrator prompt was declined.', excluded: null }),
  // library.rs: the first game is offered the hours its folder remembers, once.
  // Bridge (src-tauri bridge/): a small library with one of each kind of row.
  discord_games_status: () => ({
    on: true,
    open: true,
    games: games.map((g, i) => ({ id: g.id, title: g.title, why: g.nsfw ? 'adult' : i === 2 ? 'game' : null, playing: false, now: false, set: i === 2 })),
  }),
  discord_game_set: () => null,
  discord_presence_status: () => ({
    connected: true,
    preview: {
      details: 'Hollow Knight',
      state: '45 hours played',
      assets: { large_image: games.find((g) => g.title === 'Hollow Knight')?.cover ?? null },
      timestamps: { start: Math.floor(Date.now() / 1000) },
      buttons: [{ label: 'Get Kryoto Desktop', url: 'https://kryo.to/desktop' }, { label: 'Open in Kryoto', url: 'https://kryo.to/game/hollow-knight' }],
    },
  }),
  bridge_launchers: () => [
    { id: 'hydra', name: 'Hydra', dir: 'C:\\Users\\you\\AppData\\Roaming\\hydralauncher\\hydra-db', running: false, keeps: false, exports: false },
    { id: 'steam', name: 'Steam', dir: 'C:\\Program Files (x86)\\Steam', running: false, keeps: true, exports: true },
  ],
  bridge_detect: (a: Record<string, unknown>) => ({ id: a.launcherId, name: String(a.launcherId), dir: a.dir, running: false, keeps: a.launcherId === 'steam', exports: a.launcherId === 'steam' }),
  bridge_export_list: () => ({
    users: [{ id: '1', name: 'you' }],
    games: games.slice(0, 3).map((g) => ({ id: g.id, title: g.title, cover: g.cover ?? null, nsfw: !!g.nsfw, sent: false })),
    note: null,
    undoable: 0,
  }),
  bridge_export: (a: Record<string, unknown>) => ({ added: (a.ids as string[]).length, updated: 0, pictures: (a.ids as string[]).length * 3, backup: 'C:\\Users\\you\\AppData\\Roaming\\to.kryo.desktop\\bridge-backups\\shortcuts-1.vdf' }),
  bridge_export_undo: () => 2,
  bridge_scan: () => ({
    dir: 'C:\\Users\\you\\AppData\\Roaming\\hydralauncher\\hydra-db',
    running: false,
    catalog: true,
    rows: [
      { key: 'steam:1145360', title: 'Hades', playtimeSeconds: 45_000, lastPlayed: Math.floor(Date.now() / 1000) - 86_400, executable: 'D:\\Games\\Hades\\x64\\Hades.exe', cover: null, kind: 'matched', options: [{ slug: 'hades', title: 'Hades', developer: 'Supergiant Games', year: 2020, nsfw: false }], how: 'steamAppid', joins: null, remembered: false },
      { key: 'custom:1', title: 'Hollow Knight: Godmaster', playtimeSeconds: 1_800, lastPlayed: null, executable: 'D:\\HK\\hollow_knight.exe', cover: null, kind: 'ask', options: [{ slug: 'hollow-knight', title: 'Hollow Knight', developer: 'Team Cherry', year: 2017, nsfw: false }], how: null, joins: null, remembered: false },
      { key: 'custom:2', title: 'Lantern Rite Test', playtimeSeconds: 5_400, lastPlayed: Math.floor(Date.now() / 1000) - 3_600, executable: 'D:\\Games\\Lantern Rite\\LanternRite.exe', cover: null, kind: 'own', options: [], how: null, joins: null, remembered: false },
      { key: 'steam:504230', title: 'Celeste', playtimeSeconds: 900, lastPlayed: null, executable: null, cover: null, kind: 'missing', options: [], how: null, joins: null, remembered: false },
    ],
  }),
  bridge_import: (a: Record<string, unknown>) => ({ linked: 1, own: (a.decisions as { slug: string | null }[]).filter((d) => !d.slug).length, joined: 0, seconds: 52_200 }),
  library_merge_offers: () => [],
  store_report_carry: () => null,
  playtime_offer: (a: Record<string, unknown>) => (a.id === games[0]?.id && !games[0]?.playtimePrompt ? { seconds: 45_900, source: 'folder' } : null),
  playtime_answer: (a: Record<string, unknown>) => {
    const g = games.find((x) => x.id === a.id)
    if (!g) throw new Error('That game is no longer in the library.')
    if (a.answer === 'yes') { g.playtimeSeconds = Math.max(Number(g.playtimeSeconds) || 0, Number(a.seconds)); g.playtimePrompt = 'restored' }
    if (a.answer === 'no') g.playtimePrompt = 'never'
    return clone(g)
  },
  // The resolvers' table (src-tauri/src/resolvers/mod.rs), for a game's Builds.
  mirror_hosts: (a: Record<string, unknown>) =>
    (a.urls as string[]).map((u) => {
      const host = (() => { try { return new URL(u).hostname.replace(/^www[.]/, '') } catch { return '' } })()
      const page: Record<string, string> = { 'vikingfile.com': 'VikingFile', 'mocha.my': 'Mocha', 'dropdrive.org': 'DropDrive', 'fileditch.com': 'FileDitch', '1fichier.com': '1fichier' }
      const api: Record<string, string> = { 'pixeldrain.com': 'Pixeldrain', 'gofile.io': 'Gofile', 'buzzheavier.com': 'Buzzheavier', 'mediafire.com': 'MediaFire' }
      if (page[host]) return { host: page[host], kind: 'page' }
      if (api[host]) return { host: api[host], kind: 'api' }
      if (host === '0807.st') return { host: '0807.st', kind: 'direct' }
      // filez.stream: a file link with its key is fetched and decrypted by the app.
      if (host === 'filez.stream' && /#[A-Za-z0-9_-]{43}$/.test(u)) return { host: 'filez.stream', kind: 'api' }
      return null
    }),
  // A mirror download: its page is solved in the background, then it comes down like ours.
  download_mirror: (a: Record<string, unknown>) => {
    const id = `dl-${Date.now()}`
    const item = dl({
      id,
      slug: a.slug,
      url: a.url,
      fileName: `${String(a.title ?? a.slug)} - Kryoto.7z`,
      status: 'resolving',
      received: 0,
      total: 600_000_000,
      speed: 0,
      addedAt: now(),
      mirror: { page: a.url, host: String(a.url).includes('vikingfile') ? 'VikingFile' : 'Mirror' },
      meta: (() => {
        const g = games.find((x) => x.slug === a.slug) as Record<string, unknown> | undefined
        return { ...dl({}).meta, title: String(a.title ?? a.slug), cover: g?.cover ?? null, hero: g?.hero ?? null }
      })(),
    })
    // Two library folders and a game not installed yet: it asks where first (download_pick_folder).
    if (settings.askInstallFolder && (settings.libraryFolders as string[]).length && !games.some((g) => g.slug === a.slug)) {
      Object.assign(item, { status: 'queued', askFolder: true })
      downloadList.unshift(item)
      previewBus.emit('downloads', downloadList.map((x) => ({ ...x })))
      return null
    }
    downloadList.unshift(item)
    previewBus.emit('downloads', downloadList.map((x) => ({ ...x })))
    setTimeout(() => {
      item.status = 'downloading'
      item.speed = 36_000_000
      previewBus.emit('downloads', downloadList.map((x) => ({ ...x })))
    }, 1800)
    return null
  },
  // Achievements and Repair use the same flow as the native app, with sample saves.
  game_achievements: (a: Record<string, unknown>) => {
    const g = games.find((g) => g.id === a.id) as Record<string, unknown> | undefined
    if (!g) throw new Error('That game is no longer in the library.')
    const unsupported = !!g.online || /kryoto[ _-]*online|kryotoo\b/i.test(String(g.source ?? ''))
    const list = unsupported ? [] : [
      { name: 'PREVIEW_FIRST', displayName: 'First steps', description: 'Complete the first chapter.', hidden: false, icon: null, iconGray: null, percent: 75, unlocked: true, unlockedAt: now() - 86400 },
      { name: 'PREVIEW_SECRET', displayName: 'A hidden discovery', description: 'Find a secret.', hidden: true, icon: null, iconGray: null, percent: 12, unlocked: false, unlockedAt: null },
    ]
    return { appid: null, total: list.length, unlocked: list.filter((a) => a.unlocked).length, list, tracking: unsupported ? 'unsupported' : 'supported', emulator: unsupported ? 'Kryoto Online' : 'gbe_fork / Goldberg' }
  },
  repair_sources: () => [
    { id: 'gbe_fork', label: 'gbe_fork', online: false },
    { id: 'online', label: 'Kryoto Online', online: true },
    { id: 'rune', label: 'RUNE', online: false },
    { id: 'rune_steak', label: 'RUNE (Steakclient)', online: false },
    { id: 'rune_steamclient', label: 'RUNE (Steamclient)', online: false },
  ],
  repair_state: (a: Record<string, unknown>) => {
    const saved = previewRepairs.get(String(a.gameId))
    return { undo_available: !!saved, interrupted: false, report: saved?.report ?? null }
  },
  repair_apply: (a: Record<string, unknown>) => {
    const id = String(a.gameId)
    const g = games.find((g) => g.id === id) as Record<string, unknown> | undefined
    if (!g) throw new Error('That game is no longer in the library.')
    if (running.has(id)) throw new Error('Close the game before changing its emulator.')
    if (previewRepairs.has(id)) throw new Error('A previous repair has a backup here. Undo it before applying another source.')
    const label = ({ gbe_fork: 'gbe_fork', online: 'Kryoto Online', rune: 'RUNE', rune_steak: 'RUNE (Steakclient)', rune_steamclient: 'RUNE (Steamclient)' } as Record<string, string>)[String(a.source)]
    if (!label) throw new Error('Choose a repair source.')
    const report = { game: { title: String(g.title), slug: String(g.slug) }, source: label, version: 'preview', warnings: [] }
    previewRepairs.set(id, { source: g.source, online: g.online, report })
    g.source = `Steam + ${label}`
    g.online = a.source === 'online' ? { version: 'preview' } : null
    previewBus.emit('repair-progress', { gameId: id, text: 'Previous emulator files backed up. New emulator installed.', percent: 100 })
    previewBus.emit('library-changed', null)
    return report
  },
  repair_undo: (a: Record<string, unknown>) => {
    const id = String(a.gameId)
    const g = games.find((g) => g.id === id) as Record<string, unknown> | undefined
    const saved = previewRepairs.get(id)
    if (!g || !saved) throw new Error('No repair backup is available.')
    if (running.has(id)) throw new Error('Close the game before Undo.')
    g.source = saved.source
    g.online = saved.online
    previewRepairs.delete(id)
    previewBus.emit('library-changed', null)
    return null
  },
})

export function previewCall<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  const handler = HANDLERS[command]
  if (!handler) return Promise.reject(new Error(`${command} needs the desktop app.`))
  return new Promise((resolve, reject) =>
    setTimeout(() => {
      try {
        resolve(handler(args) as T)
      } catch (e) {
        reject(e)
      }
    }, 60),
  )
}
