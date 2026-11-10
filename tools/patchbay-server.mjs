#!/usr/bin/env node
// A Patchbay server of your own: cloud saves and achievements for Kryoto
// Desktop, kept in a folder on this machine. No dependencies beyond Node 20.
//
//   node tools/patchbay-server.mjs --dir ./patchbay-data --port 8787 --token <secret>
//
// Then in Kryoto Desktop: Settings > Patchbay, address http://<this PC>:8787
// (HTTPS through a reverse proxy when it is not on your own network), and the
// same token. The protocol is in docs/patchbay.md; this is the whole of it,
// meant to be read and to be run as it is for one person.
//
// Options (or environment): --dir (PATCHBAY_DIR), --port (PATCHBAY_PORT),
// --token (PATCHBAY_TOKEN, none: anyone who can reach it can use it),
// --public-url (PATCHBAY_PUBLIC_URL, the address Desktop reaches it at, for
// the upload and download links it hands out; default: the request's host),
// --keep (PATCHBAY_KEEP, versions kept per game, default 10).

import { createHash, createHmac, randomBytes, randomUUID, timingSafeEqual } from 'node:crypto'
import { createReadStream, createWriteStream, existsSync, mkdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { join, resolve } from 'node:path'

const arg = (name, env, fallback) => {
  const i = process.argv.indexOf(`--${name}`)
  return i > 0 ? process.argv[i + 1] : (process.env[env] ?? fallback)
}
const DIR = resolve(arg('dir', 'PATCHBAY_DIR', './patchbay-data'))
const PORT = Number(arg('port', 'PATCHBAY_PORT', '8787'))
const TOKEN = arg('token', 'PATCHBAY_TOKEN', '')
const PUBLIC_URL = arg('public-url', 'PATCHBAY_PUBLIC_URL', '').replace(/\/+$/, '')
// Versions kept per game, newest first (Desktop keeps the same on this PC).
const KEEP = Math.min(100, Math.max(1, Number(arg('keep', 'PATCHBAY_KEEP', '10')) || 10))

const ALLOWANCE = { totalBytes: 50 * 2 ** 30, versionsPerGame: KEEP, maxBackupBytes: 2 * 2 ** 30 }
// Signs upload and download links: Desktop fetches those without the token.
const SECRET = randomBytes(32)

mkdirSync(join(DIR, 'files'), { recursive: true })
const INDEX = join(DIR, 'index.json')
const db = existsSync(INDEX) ? JSON.parse(readFileSync(INDEX, 'utf8')) : { games: {}, achievements: {} }
const uploads = new Map()

function persist() {
  writeFileSync(`${INDEX}.tmp`, JSON.stringify(db, null, 2))
  renameSync(`${INDEX}.tmp`, INDEX)
}

const sign = (what) => createHmac('sha256', SECRET).update(what).digest('hex').slice(0, 32)
const signedOk = (what, key) => typeof key === 'string' && key.length === 32 && timingSafeEqual(Buffer.from(sign(what)), Buffer.from(key))

function authorized(req) {
  if (!TOKEN) return true
  const got = Buffer.from(String(req.headers.authorization ?? ''))
  const want = Buffer.from(`Bearer ${TOKEN}`)
  return got.length === want.length && timingSafeEqual(got, want)
}

function send(res, status, body) {
  res.writeHead(status, { 'content-type': 'application/json' })
  res.end(JSON.stringify(body))
}

async function json(req) {
  let text = ''
  for await (const chunk of req) {
    text += chunk
    if (text.length > 4 << 20) throw new Error('too big')
  }
  return text ? JSON.parse(text) : {}
}

const origin = (req) => PUBLIC_URL || `http://${req.headers.host}`
const head = (gameKey) => db.games[gameKey]?.versions[0] ?? null
const usage = () => {
  const games = Object.values(db.games)
  const versions = games.flatMap((g) => g.versions)
  return { bytes: versions.reduce((n, v) => n + v.size, 0), games: games.length, versions: versions.length }
}
const keyOk = (k) => typeof k === 'string' && /^[\w.:-]{1,200}$/.test(k)

async function handle(req, res) {
  const url = new URL(req.url, 'http://x')
  const path = url.pathname

  // Upload and download links carry their own signature, not the token.
  const upload = path.match(/^\/upload\/([\w-]+)$/)
  if (upload && req.method === 'PUT') {
    const u = uploads.get(upload[1])
    if (!u || !signedOk(`up:${u.id}`, url.searchParams.get('key'))) return send(res, 403, { error: 'Unknown upload.' })
    const tmp = join(DIR, 'files', `${u.id}.part`)
    const hash = createHash('sha256')
    let size = 0
    await new Promise((ok, fail) => {
      const out = createWriteStream(tmp)
      req.on('data', (c) => {
        size += c.length
        hash.update(c)
      })
      req.pipe(out)
      out.on('finish', ok)
      out.on('error', fail)
    })
    if (size !== u.size || hash.digest('hex') !== u.sha256) {
      rmSync(tmp, { force: true })
      return send(res, 422, { error: 'The backup did not arrive as it was sent.' })
    }
    u.received = true
    res.writeHead(200).end()
    return
  }
  const download = path.match(/^\/download\/([\w-]+)$/)
  if (download && req.method === 'GET') {
    const file = join(DIR, 'files', `${download[1]}.ksave`)
    if (!signedOk(`down:${download[1]}`, url.searchParams.get('key')) || !existsSync(file)) return send(res, 404, { error: 'No such backup.' })
    res.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': statSync(file).size })
    createReadStream(file).pipe(res)
    return
  }

  if (!path.startsWith('/api/')) return send(res, 404, { error: 'Not here.' })
  if (!authorized(req)) return send(res, 401, { error: 'Wrong or missing token.' })

  if (path === '/api/saves' && req.method === 'GET') {
    const gameKey = url.searchParams.get('gameKey')
    if (gameKey) return send(res, 200, { perk: true, allowance: ALLOWANCE, head: head(gameKey) })
    const games = Object.entries(db.games).map(([gameKey, g]) => ({ gameKey, aliases: g.aliases, slug: g.slug, title: g.title, bytes: g.versions.reduce((n, v) => n + v.size, 0), versions: g.versions }))
    return send(res, 200, { perk: true, allowance: ALLOWANCE, usage: usage(), games })
  }
  if (path === '/api/saves/prepare' && req.method === 'POST') {
    const b = await json(req)
    if (!keyOk(b.gameKey)) return send(res, 400, { error: 'Which game?' })
    if (!(b.size > 0) || !/^[0-9a-f]{64}$/.test(b.sha256 ?? '')) return send(res, 400, { error: 'The backup has no size or checksum.' })
    if (b.size > ALLOWANCE.maxBackupBytes) return send(res, 413, { error: 'This backup is too big.' })
    const h = head(b.gameKey)
    if ((h?.id ?? null) !== (b.parentVersion ?? null)) return send(res, 409, { error: 'Another PC backed up first.', head: h })
    const id = randomUUID()
    uploads.set(id, { id, ...b, received: false })
    return send(res, 200, { uploadId: id, url: `${origin(req)}/upload/${id}?key=${sign(`up:${id}`)}`, headers: {} })
  }
  if (path === '/api/saves/claim' && req.method === 'POST') {
    const { uploadId } = await json(req)
    const u = uploads.get(uploadId)
    if (!u?.received) return send(res, 422, { error: 'That upload never arrived.' })
    uploads.delete(uploadId)
    const h = head(u.gameKey)
    if ((h?.id ?? null) !== (u.parentVersion ?? null)) {
      rmSync(join(DIR, 'files', `${u.id}.part`), { force: true })
      return send(res, 409, { error: 'Another PC backed up first.', head: h })
    }
    renameSync(join(DIR, 'files', `${u.id}.part`), join(DIR, 'files', `${u.id}.ksave`))
    const version = {
      id: u.id,
      size: u.size,
      sha256: u.sha256,
      md5: u.md5 ?? '',
      fileCount: u.manifest?.files?.length ?? 0,
      machine: u.machine ?? null,
      createdAt: Math.floor(Date.now() / 1000),
      parentId: u.parentVersion ?? null,
      manifest: u.manifest ?? null,
    }
    const game = (db.games[u.gameKey] ??= { aliases: [], slug: null, title: '', versions: [] })
    Object.assign(game, { aliases: u.aliases ?? game.aliases, slug: u.slug ?? game.slug, title: u.title || game.title })
    game.versions.unshift(version)
    for (const old of game.versions.splice(ALLOWANCE.versionsPerGame)) rmSync(join(DIR, 'files', `${old.id}.ksave`), { force: true })
    persist()
    return send(res, 200, { version })
  }
  if (path === '/api/saves/link' && req.method === 'POST') {
    const { versionId } = await json(req)
    const v = Object.values(db.games).flatMap((g) => g.versions).find((x) => x.id === versionId)
    if (!v) return send(res, 404, { error: 'No such backup.' })
    return send(res, 200, { url: `${origin(req)}/download/${v.id}?key=${sign(`down:${v.id}`)}`, size: v.size, sha256: v.sha256 })
  }
  // Where games save is not kept here: Desktop finds it on its own.
  if (path === '/api/saves/rules' && req.method === 'GET') return send(res, 200, { rules: {} })
  if (path === '/api/saves/rules/report' && req.method === 'POST') return send(res, 200, { accepted: 0 })

  if (path === '/api/achievements/cloud' && req.method === 'GET') {
    const ids = (url.searchParams.get('appids') ?? '').split(',').filter(Boolean)
    const pick = ids.length ? ids : Object.keys(db.achievements)
    return send(res, 200, { perk: true, games: Object.fromEntries(pick.map((id) => [id, db.achievements[id]?.unlocks ?? []])) })
  }
  if (path === '/api/achievements/cloud' && req.method === 'POST') {
    const { games = [] } = await json(req)
    const saved = []
    for (const g of games.slice(0, 50)) {
      if (!/^\d{1,10}$/.test(String(g.appid))) continue
      const entry = (db.achievements[g.appid] ??= { title: g.title ?? '', unlocks: [] })
      for (const u of g.unlocks ?? []) {
        if (typeof u.name !== 'string' || entry.unlocks.some((x) => x.name === u.name)) continue
        entry.unlocks.push({ name: u.name.slice(0, 200), unlockedAt: Number(u.unlockedAt) || null })
      }
      saved.push(String(g.appid))
    }
    persist()
    return send(res, 200, { perk: true, games: saved })
  }
  return send(res, 404, { error: 'Not here.' })
}

createServer((req, res) => {
  handle(req, res).catch((e) => send(res, 400, { error: String(e?.message ?? e) }))
}).listen(PORT, () => {
  console.log(`Patchbay server on :${PORT}, keeping everything in ${DIR}${TOKEN ? '' : ' (no token: anyone who can reach it can use it)'}`)
})
