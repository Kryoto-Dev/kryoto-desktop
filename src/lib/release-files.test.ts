// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { alarms, checkedWhen, countLine, fileName, plainSummary, restoredLine, roleWords, type FileFinding } from './release-files.ts'

const f = (path: string, role: FileFinding['role'], problem: FileFinding['problem']): FileFinding => ({ path, role, problem })

test('each kind of file is said in a few words', () => {
  assert.equal(roleWords('game'), 'part of the game')
  assert.equal(roleWords('emulator-added'), 'added by the emulator')
  assert.equal(roleWords('emulator-replaced'), "the emulator's Steam file")
  assert.equal(roleWords('fix'), "part of the game's fix")
  assert.equal(roleWords('other'), 'part of the game', 'a role from a newer Forge reads as the game')
  assert.equal(fileName('bin/x64/steam_api64.dll'), 'steam_api64.dll')
  assert.equal(fileName('PlayGTAV.exe'), 'PlayGTAV.exe')
})

test('the notification names exactly which files, and what each is', () => {
  const found = [f('bin/steam_api64.dll', 'emulator-replaced', 'missing'), f('PlayGTAV.exe', 'game', 'missing')]
  assert.equal(
    plainSummary(found),
    "2 files are missing: steam_api64.dll (the emulator's Steam file), PlayGTAV.exe (part of the game).",
  )
  assert.equal(plainSummary([f('steam_api64.dll', 'emulator-replaced', 'broken')]), "1 file is damaged or replaced: steam_api64.dll (the emulator's Steam file).")
  const many = ['a', 'b', 'c', 'd', 'e'].map((n) => f(`${n}.dll`, 'game', 'missing'))
  assert.match(plainSummary(many), /^5 files are missing: a\.dll .*c\.dll \(part of the game\) and 2 more\.$/)
  assert.equal(countLine(1, 'missing'), '1 file is missing')
  assert.equal(plainSummary([]), '')
})

test("a mod's changed files never raise the card", () => {
  const found = [f('data.pak', 'game', 'changed'), f('x.dll', 'game', 'missing'), f('steam_api64.dll', 'emulator-replaced', 'broken')]
  assert.deepEqual(alarms(found).map((x) => x.path), ['x.dll', 'steam_api64.dll'])
})

test('a restore says what came back and what it takes to get the rest', () => {
  assert.equal(restoredLine({ restored: ['a.dll', 'b.exe'], fromArchive: 2, left: [], downloadSize: null }), '2 files are back from the download Kryoto kept.')
  assert.equal(
    restoredLine({ restored: ['a.dll'], fromArchive: 0, left: ['PlayGTAV.exe'], downloadSize: '95.3 GB' }),
    '1 file is back. 1 file is not on this PC any more. Downloading the game again (95.3 GB) brings it back: your saves and settings stay.',
  )
  assert.equal(restoredLine({ restored: [], fromArchive: 0, left: [], downloadSize: null }), 'Nothing was missing.')
})

test('when the last check ran', () => {
  assert.equal(checkedWhen(1000, 1030), 'just now')
  assert.equal(checkedWhen(1000, 1000 + 300), '5 minutes ago')
  assert.equal(checkedWhen(1000, 1000 + 3600), '1 hour ago')
  assert.match(checkedWhen(0, 10 * 86400), /^on 1 Jan$/)
})
