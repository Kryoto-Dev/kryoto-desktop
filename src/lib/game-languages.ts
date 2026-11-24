/**
 * Steam's language codes, as Settings > Game language and a game's Language
 * in Properties offer them, and as kryo.to names a language pack
 * (`Language: <name>`, kryo.to lib/game-languages.ts). Pure, so the plain
 * node tests can read it. Every language Steam has, by the name the emulators
 * hand a game (src-tauri player_name.rs).
 */
export const GAME_LANGUAGES: { value: string; label: string }[] = [
  { value: 'arabic', label: 'Arabic' },
  { value: 'bulgarian', label: 'Bulgarian' },
  { value: 'schinese', label: 'Chinese (Simplified)' },
  { value: 'tchinese', label: 'Chinese (Traditional)' },
  { value: 'czech', label: 'Czech' },
  { value: 'danish', label: 'Danish' },
  { value: 'dutch', label: 'Dutch' },
  { value: 'english', label: 'English' },
  { value: 'finnish', label: 'Finnish' },
  { value: 'french', label: 'French' },
  { value: 'german', label: 'German' },
  { value: 'greek', label: 'Greek' },
  { value: 'hungarian', label: 'Hungarian' },
  { value: 'indonesian', label: 'Indonesian' },
  { value: 'italian', label: 'Italian' },
  { value: 'japanese', label: 'Japanese' },
  { value: 'koreana', label: 'Korean' },
  { value: 'norwegian', label: 'Norwegian' },
  { value: 'polish', label: 'Polish' },
  { value: 'portuguese', label: 'Portuguese' },
  { value: 'brazilian', label: 'Portuguese (Brazil)' },
  { value: 'romanian', label: 'Romanian' },
  { value: 'russian', label: 'Russian' },
  { value: 'spanish', label: 'Spanish' },
  { value: 'latam', label: 'Spanish (Latin America)' },
  { value: 'swedish', label: 'Swedish' },
  { value: 'thai', label: 'Thai' },
  { value: 'turkish', label: 'Turkish' },
  { value: 'ukrainian', label: 'Ukrainian' },
  { value: 'vietnamese', label: 'Vietnamese' },
]
