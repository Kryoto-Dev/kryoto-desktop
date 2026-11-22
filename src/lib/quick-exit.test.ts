// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { quickExit } from './quick-exit.ts'

test('a game that asked Steam says why, before anything its exit code says', () => {
  const why = "Steam opened because Online-Fix's own files are gone from the game's folder (OnlineFix64.dll), usually taken by an antivirus."
  assert.deepEqual(quickExit(0, undefined, why), [why, false])
  assert.deepEqual(quickExit(53, 'wine: some line', why), [why, false])
})

test("Steam's DRM code says No licenses and what to do", () => {
  const [text, quiet] = quickExit(53)
  assert.match(text, /No licenses/)
  assert.match(text, /Kryoto Online/)
  assert.match(text, /\(exit code 53\)$/)
  assert.equal(quiet, false)
})

test('the other answers are as before', () => {
  assert.equal(quickExit(1223)[1], true)
  assert.match(quickExit(-1073741515)[0], /Visual C\+\+/)
  assert.match(quickExit(0)[0], /without an error/)
  assert.match(quickExit(1, 'err: x.')[0], /Its last lines: err: x\./)
  assert.match(quickExit(7)[0], /exit code 7/)
})
