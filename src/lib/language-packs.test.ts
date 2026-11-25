// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { isLanguagePack, languageAfterRemoving, orderPacks, packLanguage, packName, preferredLanguage, steamCodeForLocale } from './language-packs.ts'

test('taking a language pack out puts the game back on Settings when it was in that language', () => {
  const polish = { label: 'Language: Polish' }
  assert.equal(languageAfterRemoving('polish', polish, []), null)
  // Another pack of the same language is still there, or the game is in another language.
  assert.equal(languageAfterRemoving('polish', polish, [polish]), 'polish')
  assert.equal(languageAfterRemoving('german', polish, []), 'german')
  // A DLC is not a language; following Settings stays so.
  assert.equal(languageAfterRemoving('polish', { label: 'DLC: Soundtrack' }, []), 'polish')
  assert.equal(languageAfterRemoving(null, polish, []), null)
})

test('only a language the pack set goes with it, by the code Add said', () => {
  // kryo.to files it as Brazilian under a label that reads Portuguese.
  const brazilian = { label: 'Language: Portuguese', language: 'brazilian', setLanguage: true }
  assert.equal(languageAfterRemoving('brazilian', brazilian, []), null)
  assert.equal(languageAfterRemoving('brazilian', brazilian, [{ label: 'Language: Portuguese (Brazil)' }]), 'brazilian')
  // The player picked Polish in Properties before adding the pack: it stays.
  assert.equal(languageAfterRemoving('polish', { label: 'Language: Polish', language: 'polish', setLanguage: false }, []), 'polish')
})

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
