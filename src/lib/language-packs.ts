import { GAME_LANGUAGES } from './game-languages.ts'

/**
 * Language packs: a language Steam keeps in a depot of its own (Cyberpunk
 * 2077's voice packs, Yakuza 0's Japanese), filed on kryo.to as an add-on
 * labelled `Language: <name>` beside the game (kryo.to lib/game-languages.ts,
 * the same convention as `DLC: <name>`). The downloads API says each pack's
 * Steam code as `language`; the label is the fallback.
 */

const PREFIX = /^language:\s*/i

export const isLanguagePack = (a: { label: string | null }) => PREFIX.test(a.label ?? '')

/** "Polish", out of "Language: Polish". */
export const packName = (label: string | null) => (label ?? '').replace(PREFIX, '').trim() || 'Language pack'

/** The pack's Steam code: what kryo.to says, else read from its label. */
export function packLanguage(a: { label: string | null; language?: string | null }): string | null {
  if (a.language) return a.language
  const name = packName(a.label).toLowerCase()
  return GAME_LANGUAGES.find((l) => l.label.toLowerCase() === name)?.value ?? null
}

/** A locale (`pl-PL`, `pt-BR`, `zh-TW`) as Steam's code, or null (kryo.to `steamCodeForLocale`). */
export function steamCodeForLocale(locale: string | null | undefined): string | null {
  const tag = (locale ?? '').trim().toLowerCase().replace('_', '-')
  if (!tag) return null
  const [lang = '', region = ''] = tag.split('-')
  if (lang === 'zh') return ['tw', 'hk', 'mo', 'hant'].includes(region) ? 'tchinese' : 'schinese'
  if (lang === 'pt') return region === 'br' ? 'brazilian' : 'portuguese'
  if (lang === 'es') return region === '' || region === 'es' ? 'spanish' : 'latam'
  if (lang === 'ko') return 'koreana'
  if (lang === 'nb' || lang === 'nn' || lang === 'no') return 'norwegian'
  const byIso: Record<string, string> = {
    ar: 'arabic', bg: 'bulgarian', cs: 'czech', da: 'danish', nl: 'dutch', en: 'english',
    fi: 'finnish', fr: 'french', de: 'german', el: 'greek', hu: 'hungarian', id: 'indonesian',
    it: 'italian', ja: 'japanese', pl: 'polish', ro: 'romanian', ru: 'russian', sv: 'swedish',
    th: 'thai', tr: 'turkish', uk: 'ukrainian', vi: 'vietnamese',
  }
  return byIso[lang] ?? null
}

/**
 * The language to lead a game's packs with: the game's own Language in
 * Properties, else Settings > Game language, else this PC's language.
 */
export function preferredLanguage(
  gameLanguage: string | null | undefined,
  settingsLanguage: string | null | undefined,
  locales: readonly string[],
): string | null {
  if (gameLanguage) return gameLanguage
  if (settingsLanguage) return settingsLanguage
  for (const l of locales) {
    const code = steamCodeForLocale(l)
    if (code) return code
  }
  return null
}

/** The preferred language's pack first, then by name. */
export function orderPacks<T extends { label: string | null; language?: string | null }>(packs: T[], preferred: string | null): T[] {
  return [...packs].sort((a, b) => {
    const mineA = preferred !== null && packLanguage(a) === preferred
    const mineB = preferred !== null && packLanguage(b) === preferred
    if (mineA !== mineB) return mineA ? -1 : 1
    return packName(a.label).localeCompare(packName(b.label))
  })
}
