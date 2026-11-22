// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { autofixBlock, NO_AUTOFIX_FIELD, NO_AUTOFIX_NOTICE_FIELD, NO_AUTOFIX_TEXT } from './autofix.ts'

test('a normal release, or an older kryo.to without the field, keeps the auto-fixer', () => {
  assert.equal(autofixBlock({ title: 'Hades' }), null)
  assert.equal(autofixBlock(null), null)
  assert.equal(autofixBlock({ [NO_AUTOFIX_FIELD]: false }), null)
  assert.equal(autofixBlock({ [NO_AUTOFIX_FIELD]: '  ' }), null)
  assert.equal(autofixBlock({ [NO_AUTOFIX_FIELD]: 1 }), null)
})

test('a marked release shows its own notice instead', () => {
  assert.deepEqual(autofixBlock({ [NO_AUTOFIX_FIELD]: true, [NO_AUTOFIX_NOTICE_FIELD]: 'Run setup.exe as administrator first.' }), {
    notice: 'Run setup.exe as administrator first.',
  })
  assert.deepEqual(autofixBlock({ [NO_AUTOFIX_FIELD]: 'Needs the hypervisor steps on the page.' }), { notice: 'Needs the hypervisor steps on the page.' })
})

test('marked with no words of its own, it still says what to do', () => {
  assert.deepEqual(autofixBlock({ [NO_AUTOFIX_FIELD]: true }), { notice: NO_AUTOFIX_TEXT })
})
