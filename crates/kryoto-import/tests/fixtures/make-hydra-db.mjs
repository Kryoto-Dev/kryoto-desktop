// Writes a Hydra library the way Hydra itself does: classic-level (the
// LevelDB binding Hydra uses), sublevels with JSON values, and the `Game`
// shape of hydralauncher/hydra `src/types/level.types.ts`.
//
//   npm i classic-level@2.0.0   (in a scratch folder, not this repo)
//   node make-hydra-db.mjs <out folder> [<games folder>]
//
// With only <out folder> it writes the unit-test fixture (`hydra-db/` here).
// With a <games folder> too it writes an end-to-end library whose exes are
// real files in that folder (stand-ins copied from Windows' ping.exe), for
// importing on the Desktop dev build.
import { createRequire } from 'node:module'
import fs from 'node:fs'
import path from 'node:path'

const require = createRequire(path.join(process.cwd(), 'x.js'))
const { ClassicLevel } = require('classic-level')
const [out, gamesRoot] = process.argv.slice(2)
if (!out) throw new Error('usage: make-hydra-db.mjs <out> [<games folder>]')
fs.rmSync(out, { recursive: true, force: true })

const open = () => new ClassicLevel(out, { valueEncoding: 'json' })
const subs = (db) => ({
  games: db.sublevel('games', { valueEncoding: 'json' }),
  assets: db.sublevel('gameShopAssets', { valueEncoding: 'json' }),
  cache: db.sublevel('gameShopCache', { valueEncoding: 'json' }),
})
const base = { iconUrl: null, libraryHeroImageUrl: null, logoImageUrl: null, remoteId: null, isDeleted: false, favorite: false, automaticCloudSync: false, hasManuallyUpdatedPlaytime: false }

function exe(rel) {
  if (!gamesRoot) return 'D:\\Games\\' + rel.replaceAll('/', '\\')
  const file = path.join(gamesRoot, rel)
  fs.mkdirSync(path.dirname(file), { recursive: true })
  fs.copyFileSync(String.raw`C:\Windows\System32\PING.EXE`, file)
  return file
}

const games = gamesRoot
  ? [
      // Matched by AppID (kryo-web-local has Hades as 1145360).
      ['steam', '1145360', { title: 'Hades', playTimeInMilliseconds: 12_600_000, lastTimePlayed: '2026-10-20T19:00:00.000Z', executablePath: exe('Hades/x64/Hades.exe'), launchOptions: '-n 75 127.0.0.1' }],
      // Matched by title.
      ['custom', '5f0c1a52-6d1e-4b0a-9a51-3f6d2c1e0a11', { title: 'Stardew Valley', playTimeInMilliseconds: 3_600_000, lastTimePlayed: '2026-10-01T10:00:00.000Z', executablePath: exe('Stardew Valley/Stardew Valley.exe') }],
      // Kryoto's exe under another name: asked.
      ['custom', '0b9e6f4e-2f0c-4a43-8c8a-2a7f4d3b9c22', { title: 'OA nightly', playTimeInMilliseconds: 600_000, lastTimePlayed: null, executablePath: exe('OA/Order Automatica.exe') }],
      // Close but not sure: asked.
      ['custom', 'e3a1d7c4-1b0e-4c9e-9d3a-6f2b8c4d1e33', { title: 'Hollow Knight: Godmaster', playTimeInMilliseconds: 1_800_000, lastTimePlayed: '2026-09-10T08:00:00.000Z', executablePath: exe('HK/hollow_knight.exe') }],
      // Not on Kryoto: kept as the player's own, with Hydra's data. Its
      // catalogue link (AppID 3999991) is what a later listing matches.
      ['custom', 'c7d2e9a0-4f1b-4e8c-a2d6-1b3f5e7a9c44', { title: 'Lantern Rite Test', playTimeInMilliseconds: 5_400_000, lastTimePlayed: '2026-10-25T21:30:00.000Z', executablePath: exe('Lantern Rite/LanternRite.exe'), launchOptions: '-n 70 127.0.0.1', catalogueGame: { shop: 'steam', objectId: '3999991' }, libraryHeroImageUrl: 'https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/1145360/library_hero.jpg' }],
      // In Hydra, but not installed on this PC.
      ['steam', '504230', { title: 'Celeste', playTimeInMilliseconds: 900_000, lastTimePlayed: null, executablePath: null }],
      // Removed from Hydra's library: never imported.
      ['steam', '753640', { title: 'Outer Wilds', playTimeInMilliseconds: 1, isDeleted: true, executablePath: exe('Outer Wilds/OuterWilds.exe') }],
    ]
  : [
      ['steam', '1145360', { title: 'Hades', playTimeInMilliseconds: 1_000, lastTimePlayed: null, executablePath: String.raw`D:\Games\Hades\x64\Hades.exe` }],
      ['custom', '5f0c1a52-6d1e-4b0a-9a51-3f6d2c1e0a11', { title: 'Stardew Valley', playTimeInMilliseconds: 3_600_000, lastTimePlayed: '2026-10-01T10:00:00.000Z', executablePath: String.raw`D:\Games\Stardew Valley\Stardew Valley.exe`, customIconUrl: String.raw`local:C:\Users\Player\AppData\Roaming\hydralauncher\Assets\custom-games\sdv.png` }],
      ['steam', '70', { title: 'Half-Life', playTimeInMilliseconds: 5, isDeleted: true }],
      ['steam', '220', { title: 'Half-Life 2', playTimeInMilliseconds: 5 }],
    ]

// First session: everything as written, then closed. Opening again folds
// that log into a sorted table, so the fixture has both kinds of file.
let db = open()
let s = subs(db)
for (const [shop, objectId, g] of games) {
  await s.games.put(`${shop}:${objectId}`, { ...base, shop, objectId, addedToLibraryAt: '2026-06-01T12:00:00.000Z', ...g })
}
await s.assets.put('steam:1145360', { objectId: '1145360', shop: 'steam', title: 'Hades', iconUrl: null, libraryHeroImageUrl: 'https://cdn.example/hades-hero.jpg', libraryImageUrl: '', logoImageUrl: 'https://cdn.example/hades-logo.png', logoPosition: null, coverImageUrl: 'https://cdn.example/hades-cover.jpg', downloadSources: [], updatedAt: 1 })
// A long store description, as Hydra caches one, so the table block compresses.
await s.cache.put('steam:1145360:english', { objectId: '1145360', developers: ['Supergiant Games'], release_date: { coming_soon: false, date: '17 Sep, 2020' }, detailed_description: 'Defy the god of the dead. '.repeat(400) })
await db.close()

db = open()
await db.open()
s = subs(db)
if (!gamesRoot) {
  // Second session, left in the log: an update and a removal.
  const hades = await s.games.get('steam:1145360')
  await s.games.put('steam:1145360', { ...hades, playTimeInMilliseconds: 7_260_500, lastTimePlayed: '2026-08-21T12:42:10.123Z', launchOptions: '-dx11' })
  await s.games.del('steam:220')
}
await db.close()
console.log('wrote', out, fs.readdirSync(out).join(' '))
