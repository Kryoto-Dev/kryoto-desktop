// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { autofixBlock, NO_AUTOFIX_FIELD, NO_AUTOFIX_TEXT } from './autofix.ts'

test('a normal release, or an older kryo.to without the field, keeps the auto-fixer', () => {
  assert.equal(autofixBlock({ title: 'Hades' }), null)
  assert.equal(autofixBlock(null), null)
  assert.equal(autofixBlock({ [NO_AUTOFIX_FIELD]: false }), null)
  // Only a real true counts: a string or a number from somewhere else is not the mark.
  assert.equal(autofixBlock({ [NO_AUTOFIX_FIELD]: 'true' }), null)
  assert.equal(autofixBlock({ [NO_AUTOFIX_FIELD]: 1 }), null)
})

test('a hypervisor release shows its notice instead of the auto-fixer', () => {
  assert.deepEqual(autofixBlock({ title: 'Phantom Blade Zero', [NO_AUTOFIX_FIELD]: true }), { notice: NO_AUTOFIX_TEXT })
  assert.match(NO_AUTOFIX_TEXT, /hypervisor/)
})
