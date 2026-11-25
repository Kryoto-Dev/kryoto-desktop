// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { iconsFor } from './game-icon.ts'

test("the player's icon first, then Steam's, then the cover", () => {
  const game = { customIcon: 'kryoto-art://local/a', icon: 'https://x/icon.jpg', customCover: 'kryoto-art://local/c', cover: 'https://x/cover.jpg' }
  assert.deepEqual(iconsFor(game), ['kryoto-art://local/a', 'https://x/icon.jpg', 'kryoto-art://local/c'])
  assert.deepEqual(iconsFor({ ...game, customIcon: null }), ['https://x/icon.jpg', 'kryoto-art://local/c'])
})

test('a game from before icons draws its cover, as the list did', () => {
  assert.deepEqual(iconsFor({ cover: 'https://x/cover.jpg' }), ['https://x/cover.jpg'])
  assert.deepEqual(iconsFor({ cover: 'https://x/cover.jpg', customCover: 'kryoto-art://local/c' }), ['kryoto-art://local/c'])
  assert.deepEqual(iconsFor({ cover: null }), [])
})

test('the same picture is tried once', () => {
  assert.deepEqual(iconsFor({ icon: 'https://x/a.jpg', cover: 'https://x/a.jpg' }), ['https://x/a.jpg'])
})
