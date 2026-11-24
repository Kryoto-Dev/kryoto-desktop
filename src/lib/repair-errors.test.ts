// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { stuckCacheFolder } from './repair-errors.ts'

test('a Repair error that names a stuck emulator cache gives its folder', () => {
  const dir = String.raw`C:\Users\Player\AppData\Roaming\to.kryo.desktop\repair-tools\9f2c in dir`
  const text = `Kryoto's copy of gbe_fork in ${dir} changed on this PC and could not be replaced (Access is denied. (os error 5)). Open that folder, delete it, then press Repair again.`
  assert.equal(stuckCacheFolder(text), dir)
  assert.equal(stuckCacheFolder('Emulator checksum verification failed. No game files were changed.'), null)
  assert.equal(stuckCacheFolder(null), null)
})
