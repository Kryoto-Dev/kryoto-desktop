// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { afterNothingEncrypted, NO_CHAT, STILL_STARTING } from './send-rules.ts'

test('a friend with a chat device is never told chat is off (KB-0256)', () => {
  // Their device exists but took nothing yet: once more, then "still starting".
  assert.equal(afterNothingEncrypted(false, false, 0), 'retry')
  assert.equal(afterNothingEncrypted(false, false, 1), 'starting')
  assert.notEqual(STILL_STARTING, NO_CHAT)
  // Only ourselves, or a group nobody else in can be reached: fine.
  assert.equal(afterNothingEncrypted(false, true, 0), 'done')
  assert.equal(afterNothingEncrypted(true, false, 1), 'done')
})
