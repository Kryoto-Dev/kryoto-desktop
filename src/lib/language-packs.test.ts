// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { isLanguagePack, orderPacks, packLanguage, packName, preferredLanguage, steamCodeForLocale } from './language-packs.ts'

test('a language pack is known by its label, like a DLC', () => {
  assert.equal(isLanguagePack({ label: 'Language: Polish' }), true)
  assert.equal(isLanguagePack({ label: 'DLC: Polish voices' }), false)
  assert.equal(isLanguagePack({ label: null }), false)
  assert.equal(packName('language:  Chinese (Simplified) '), 'Chinese (Simplified)')
  assert.equal(packLanguage({ label: 'Language: Chinese (Simplified)' }), 'schinese')
  // What kryo.to says wins over the label.
  assert.equal(packLanguage({ label: 'Language: Portuguese', language: 'brazilian' }), 'brazilian')
  assert.equal(packLanguage({ label: 'Language: Old Norse' }), null)
})

test('the game first, then Settings, then this PC, says whose language leads', () => {
  assert.equal(preferredLanguage('japanese', 'german', ['pl-PL']), 'japanese')
  assert.equal(preferredLanguage(null, 'german', ['pl-PL']), 'german')
  // Empty is "Same as Settings" / "Game's own": not a language.
  assert.equal(preferredLanguage('', '', ['xx', 'pt-BR']), 'brazilian')
  assert.equal(preferredLanguage(null, '', []), null)
  assert.equal(steamCodeForLocale('zh-TW'), 'tchinese')
  assert.equal(steamCodeForLocale('es-419'), 'latam')
})

test('the preferred language leads, the rest by name', () => {
  const packs = [{ label: 'Language: Russian' }, { label: 'Language: Japanese' }, { label: 'Language: Polish', language: 'polish' }]
  assert.deepEqual(orderPacks(packs, 'polish').map((p) => packName(p.label)), ['Polish', 'Japanese', 'Russian'])
  assert.deepEqual(orderPacks(packs, null).map((p) => packName(p.label)), ['Japanese', 'Polish', 'Russian'])
})
