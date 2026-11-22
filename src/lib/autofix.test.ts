// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { AUTOFIX_FIELD, AUTOFIX_REASON_FIELD, autofixBlock, BLOCKED_TEXT, NO_AUTOFIX_FIELD, NO_AUTOFIX_TEXT } from './autofix.ts'

test('a normal release, or an older kryo.to without the fields, keeps the auto-fixer', () => {
  assert.equal(autofixBlock({ title: 'Hades' }), null)
  assert.equal(autofixBlock(null), null)
  assert.equal(autofixBlock({ [NO_AUTOFIX_FIELD]: false }), null)
  assert.equal(autofixBlock({ [AUTOFIX_FIELD]: 'allowed', [AUTOFIX_REASON_FIELD]: null }), null)
  // Only a real true counts: a string or a number from somewhere else is not the mark.
  assert.equal(autofixBlock({ [NO_AUTOFIX_FIELD]: 'true' }), null)
  assert.equal(autofixBlock({ [NO_AUTOFIX_FIELD]: 1 }), null)
  assert.equal(autofixBlock({ [AUTOFIX_FIELD]: true }), null)
})

test('a hypervisor release shows its notice instead of the auto-fixer', () => {
  assert.deepEqual(autofixBlock({ title: 'Phantom Blade Zero', [NO_AUTOFIX_FIELD]: true }), { notice: NO_AUTOFIX_TEXT, reason: null })
  assert.match(NO_AUTOFIX_TEXT, /hypervisor/)
  // kryo.to sends both for it; the reason only repeats the notice.
  assert.deepEqual(
    autofixBlock({ [NO_AUTOFIX_FIELD]: true, [AUTOFIX_FIELD]: 'blocked', [AUTOFIX_REASON_FIELD]: 'Hypervisor release' }),
    { notice: NO_AUTOFIX_TEXT, reason: null },
  )
})

test('a release staff blocked hides the auto-fixer and says why', () => {
  assert.deepEqual(autofixBlock({ [AUTOFIX_FIELD]: 'blocked', [AUTOFIX_REASON_FIELD]: ' Denuvo ' }), { notice: BLOCKED_TEXT, reason: 'Denuvo' })
  assert.deepEqual(autofixBlock({ [AUTOFIX_FIELD]: 'blocked', [AUTOFIX_REASON_FIELD]: null }), { notice: BLOCKED_TEXT, reason: null })
  assert.equal(autofixBlock({ [AUTOFIX_FIELD]: 'blocked', [AUTOFIX_REASON_FIELD]: 'x'.repeat(500) })?.reason?.length, 120)
})
