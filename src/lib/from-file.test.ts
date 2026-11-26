// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { fileVerdict, ownFileLatest, pickDropped, type ArchiveLook } from './from-file.ts'

const look = (over: Partial<ArchiveLook> = {}): ArchiveLook => ({
  path: 'D:\\Downloads\\Captain Hardcore - kryoto.7z',
  fileName: 'Captain Hardcore - kryoto.7z',
  size: 1000,
  slug: 'captain-hardcore',
  title: 'Captain Hardcore',
  cover: null,
  nsfw: false,
  version: '1.2',
  addon: null,
  sizeMatch: true,
  installed: false,
  reachable: true,
  ...over,
})

test('a file the size of a listed build is offered with one Install, checked as it goes', () => {
  const v = fileVerdict(look())
  assert.equal(v.kind, 'install')
  assert.ok(v.kind === 'install' && !v.anyway && v.button === 'Install')
  assert.equal(v.line, "This looks like Captain Hardcore, build 1.2. Kryoto checks it against kryo.to's copy as it installs.")
})

test('a file no listed size fits says so plainly and never claims to be verified', () => {
  const v = fileVerdict(look({ sizeMatch: false, version: null }))
  assert.ok(v.kind === 'install' && v.anyway)
  assert.match(v.line, /not Kryoto's copy/)
  assert.match(v.line, /not verified against Kryoto's copy/)
  assert.doesNotMatch(v.line, /checks it against/)
})

test('a file whose game is unknown asks which game it is', () => {
  assert.equal(fileVerdict(look({ slug: null, title: null, reachable: false })).kind, 'pick')
})

test('an add-on installs into its game, or names the game to install first', () => {
  const into = fileVerdict(look({ addon: 'DLC: Firebird', installed: true, version: null }))
  assert.ok(into.kind === 'install' && into.line.startsWith('This looks like the Firebird DLC for Captain Hardcore.'))
  const missing = fileVerdict(look({ addon: 'Language: Polish', installed: false }))
  assert.equal(missing.kind, 'needs-game')
  assert.equal(missing.line, 'This is Language: Polish for Captain Hardcore. Install Captain Hardcore first, then this file.')
})

test('offline, the file still installs and is checked when kryo.to can be asked', () => {
  const v = fileVerdict(look({ reachable: false, sizeMatch: null, version: null }))
  assert.ok(v.kind === 'install' && !v.anyway)
  assert.match(v.line, /did not answer/)
})

test('of several dropped files, the archive is taken', () => {
  assert.equal(pickDropped(['C:\\a\\notes.txt', 'C:\\a\\Game - kryoto.7z']), 'C:\\a\\Game - kryoto.7z')
  assert.equal(pickDropped(['C:\\a\\notes.txt']), 'C:\\a\\notes.txt')
  assert.equal(pickDropped([]), null)
})

test("a copy from the player's own file is offered kryo.to's build, even under the same number", () => {
  const mark = ' (new upload)'
  // "Game v1.2 - kryoto.7z", an older upload of 1.2 kryo.to has since replaced.
  assert.equal(ownFileLatest('1.2', '1.2', mark), '1.2 (new upload)')
  assert.equal(ownFileLatest(null, null, mark), 'This build (new upload)')
  assert.equal(ownFileLatest(null, '1.2', mark), '1.2')
  assert.equal(ownFileLatest('1.1', '1.2', mark), '1.2')
})
