#!/usr/bin/env node
// A release file list for testing Kryoto Desktop's Check files, Get the files
// back and the emulator swap (src-tauri/src/release_files.rs) against a LOCAL
// kryo.to, made from a game folder as Kryoto Desktop installed it.
//
//   node tools/fake-release-files.mjs <game folder> <release id> <slug> [--emulator gbe_fork:experimental] [--off] [--excluded bin/steam_api64.dll]
//
// It reads the folder (nothing is written there) and prints one SQL statement
// for the local games database (`kryo_games`, games/0086 applied), which
// stores the list the way kryo.to's lib/release-files.ts does: gzipped JSON
// with its SHA-256 as the ETag. It never connects to a database itself: run
// what it prints with psql against the LOCAL URL only, never kryo.to/.env.local.
//
// Roles, as Forge gives them: steam_api.dll / steam_api64.dll are
// `emulator-replaced` (their `.kryoto` sibling, when there, is the shipped
// backup of Valve's original); `steam_settings/`, `*.kryoto`, `*.rne` and the
// emulator's own dlls are `emulator-added`; `RunMe.bat`, `*.lnk` and
// `.kryoto-release.json` are `kryoto`; everything else is `game`. Kryoto's own
// bookkeeping folders (`.kryoto-repair`, `.kryoto-orig`) are left out.

import { createHash } from 'node:crypto'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative, sep } from 'node:path'
import { gzipSync } from 'node:zlib'

const args = process.argv.slice(2)
const flag = (name) => {
  const i = args.indexOf(name)
  if (i < 0) return null
  const [v] = args.splice(i, 2).slice(1)
  return v ?? ''
}
const off = args.includes('--off')
if (off) args.splice(args.indexOf('--off'), 1)
const emulatorArg = flag('--emulator')
const excluded = (flag('--excluded') ?? '').split(',').filter(Boolean)
const [folder, releaseId, slug] = args
if (!folder || !/^\d{1,19}$/.test(releaseId ?? '') || !slug) {
  console.error('usage: node tools/fake-release-files.mjs <game folder> <release id> <slug> [--emulator name:variant] [--off] [--excluded a,b]')
  process.exit(2)
}

const sha = (bytes) => createHash('sha256').update(bytes).digest('hex')
const EMULATOR_DLLS = /(^|\/)(steamclient(64)?|kryotoO(32)?|rune(64)?|steakclient64|GameOverlayRenderer(64)?)\.dll$|(^|\/)(steam_emu|steak_emu|kryoto-online)\.ini$|(^|\/)steam_appid\.txt$/i

function walk(dir, out = []) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name)
    const rel = relative(folder, p).split(sep).join('/')
    if (rel.split('/')[0].startsWith('.kryoto-') && rel !== '.kryoto-release.json') continue
    if (e.isDirectory()) walk(p, out)
    else if (e.isFile()) out.push(rel)
  }
  return out
}

const files = walk(folder)
  .sort()
  .map((path) => {
    const bytes = readFileSync(join(folder, path))
    const entry = { path, size: bytes.length, sha256: sha(bytes), role: 'game' }
    if (/(^|\/)steam_api(64)?\.dll$/i.test(path) && !excluded.includes(path)) {
      entry.role = 'emulator-replaced'
      try {
        const original = readFileSync(join(folder, `${path}.kryoto`))
        entry.original_sha256 = sha(original)
        entry.original_size = original.length
        entry.backup = `${path}.kryoto`
      } catch {
        /* no shipped backup */
      }
    } else if (/(^|\/)steam_settings\//i.test(path) || /\.(kryoto|rne)$/i.test(path) || EMULATOR_DLLS.test(path)) {
      entry.role = 'emulator-added'
    } else if (/(^|\/)RunMe\.bat$|\.lnk$|^\.kryoto-release\.json$/i.test(path)) {
      entry.role = 'kryoto'
    }
    return entry
  })

const [name, variant] = (emulatorArg ?? 'gbe_fork').split(':')
const emulator = off ? null : { name, label: name, ...(variant ? { variant } : {}) }
const doc = {
  schema: 1,
  release_id: releaseId,
  slug,
  applied_by_forge: !off,
  emulator_off: off,
  excluded,
  emulator,
  files,
  removed: [],
}
const body = gzipSync(Buffer.from(JSON.stringify(doc), 'utf8'), { level: 9 })
const counts = {}
let total = 0
for (const f of files) {
  counts[f.role] = (counts[f.role] ?? 0) + 1
  total += f.size
}
const q = (s) => `'${String(s).replace(/'/g, "''")}'`
console.log(
  `INSERT INTO release_files (version_id, schema_version, file_count, total_bytes, role_counts, applied_by_forge, emulator_off, excluded, emulator, body, body_bytes, etag, created_at)
VALUES (${releaseId}, 1, ${files.length}, ${total}, ${q(JSON.stringify(counts))}::jsonb, ${!off}, ${off}, ${q(JSON.stringify(excluded))}::jsonb, ${emulator ? `${q(JSON.stringify(emulator))}::jsonb` : 'NULL'}, '\\x${body.toString('hex')}'::bytea, ${body.length}, ${q(sha(body))}, now())
ON CONFLICT (version_id) DO UPDATE SET file_count = EXCLUDED.file_count, total_bytes = EXCLUDED.total_bytes, role_counts = EXCLUDED.role_counts,
  applied_by_forge = EXCLUDED.applied_by_forge, emulator_off = EXCLUDED.emulator_off, excluded = EXCLUDED.excluded, emulator = EXCLUDED.emulator,
  body = EXCLUDED.body, body_bytes = EXCLUDED.body_bytes, etag = EXCLUDED.etag, created_at = now();`,
)
console.error(`${files.length} files (${Object.entries(counts).map(([r, n]) => `${n} ${r}`).join(', ')}), ${body.length} bytes gzipped`)
