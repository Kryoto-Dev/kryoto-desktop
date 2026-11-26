// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { clearDraft, draftKey, readDraft, saveDraft } from './report-draft.ts'

function memory() {
  const items = new Map<string, string>()
  return {
    items,
    getItem: (k: string) => items.get(k) ?? null,
    setItem: (k: string, v: string) => void items.set(k, v),
    removeItem: (k: string) => void items.delete(k),
  }
}

test('a saved draft is read back as it was written', () => {
  const store = memory()
  saveDraft(store, 'tess', { kind: 'bug', what: 'The Store went white', steps: 'Open Downloads' })
  assert.deepEqual(readDraft(store, 'tess'), { kind: 'bug', what: 'The Store went white', steps: 'Open Downloads' })
  saveDraft(store, 'tess', { kind: 'idea', what: 'A dark cover wall', steps: '' })
  assert.deepEqual(readDraft(store, 'tess'), { kind: 'idea', what: 'A dark cover wall', steps: '' })
})

test('empty boxes keep nothing, and take an older draft away', () => {
  const store = memory()
  saveDraft(store, 'tess', { kind: 'bug', what: '  ', steps: '' })
  assert.equal(store.items.size, 0)
  saveDraft(store, 'tess', { kind: 'bug', what: 'Crash', steps: '' })
  saveDraft(store, 'tess', { kind: 'bug', what: '', steps: '\n' })
  assert.equal(readDraft(store, 'tess'), null)
  assert.equal(store.items.size, 0)
})

test('steps alone are kept, so they are not lost before the first box is filled', () => {
  const store = memory()
  saveDraft(store, 'tess', { kind: 'bug', what: '', steps: '1. Open Settings' })
  assert.deepEqual(readDraft(store, 'tess'), { kind: 'bug', what: '', steps: '1. Open Settings' })
})

test('clearing after a send leaves nothing to restore', () => {
  const store = memory()
  saveDraft(store, 'tess', { kind: 'bug', what: 'Crash', steps: '' })
  clearDraft(store, 'tess')
  assert.equal(readDraft(store, 'tess'), null)
})

test('broken or foreign storage gives no draft instead of throwing', () => {
  const store = memory()
  store.setItem(draftKey('tess'), '{not json')
  assert.equal(readDraft(store, 'tess'), null)
  store.setItem(draftKey('tess'), '"text"')
  assert.equal(readDraft(store, 'tess'), null)
  store.setItem(draftKey('tess'), JSON.stringify({ kind: 'yours', what: 7, steps: 'Step' }))
  assert.deepEqual(readDraft(store, 'tess'), { kind: 'bug', what: '', steps: 'Step' })
  store.setItem(draftKey('tess'), JSON.stringify({ kind: 'bug', what: 'x'.repeat(9000) }))
  assert.equal(readDraft(store, 'tess')?.what.length, 4000)
  assert.equal(readDraft(null, 'tess'), null)
})

test('a storage that refuses every call does not stop the report', () => {
  const refuses = {
    getItem: () => { throw new Error('denied') },
    setItem: () => { throw new Error('quota') },
    removeItem: () => { throw new Error('denied') },
  }
  assert.equal(readDraft(refuses, 'tess'), null)
  assert.doesNotThrow(() => saveDraft(refuses, 'tess', { kind: 'bug', what: 'Crash', steps: '' }))
  assert.doesNotThrow(() => clearDraft(refuses, 'tess'))
})

test("one account's draft is not another's, so it is never sent under the wrong name", () => {
  const store = memory()
  saveDraft(store, 'tess', { kind: 'bug', what: 'Crash', steps: '' })
  assert.equal(readDraft(store, 'sam'), null)
  assert.deepEqual(readDraft(store, 'Tess'), { kind: 'bug', what: 'Crash', steps: '' })
  clearDraft(store, 'sam')
  assert.equal(readDraft(store, 'tess')?.what, 'Crash')
})
