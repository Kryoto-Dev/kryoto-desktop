/**
 * Installing a game from an archive the player already downloaded (src-tauri
 * from_file.rs): what the dialog says about a file before it is installed, in
 * a player's words. Pure, for `pnpm test`.
 */

/** from_file.rs `Look`. */
export type ArchiveLook = {
  path: string
  fileName: string
  size: number
  /** The kryo.to game it is for; null until the player picks one. */
  slug: string | null
  title: string | null
  cover: string | null
  nsfw: boolean
  /** The build its size points to. */
  version: string | null
  /** It is one of the game's add-ons: its name ("DLC: Firebird"). */
  addon: string | null
  /** true: kryo.to lists a file of exactly this size; false: it lists sizes and none fits; null: it does not say. */
  sizeMatch: boolean | null
  /** The game (for an add-on, the game it goes into) is in the library. */
  installed: boolean
  /** kryo.to answered about the game. */
  reachable: boolean
}

export type FileVerdict =
  /** No game found by the file's name: "Which game is this?" */
  | { kind: 'pick'; line: string }
  /** An add-on whose game is not installed: that game first. */
  | { kind: 'needs-game'; line: string }
  /** Ready: `anyway` when it will go in marked as not verified. */
  | { kind: 'install'; line: string; button: string; anyway: boolean }

/** "DLC: Firebird" reads "the Firebird DLC"; a language pack reads as it is. */
function addonName(label: string): string {
  const dlc = /^dlc:\s*(.+)$/i.exec(label.trim())
  return dlc ? `the ${dlc[1]} DLC` : label.trim()
}

export function fileVerdict(look: ArchiveLook): FileVerdict {
  if (!look.slug) {
    return {
      kind: 'pick',
      line: look.reachable || look.title
        ? 'Kryoto could not tell which game this file is from its name. Which game is it?'
        : 'Which game is this? Kryoto could not tell from the file name.',
    }
  }
  const title = look.title?.trim() || look.slug
  if (look.addon && !look.installed) {
    return { kind: 'needs-game', line: `This is ${addonName(look.addon)} for ${title}. Install ${title} first, then this file.` }
  }
  if (look.sizeMatch === false) {
    return {
      kind: 'install',
      anyway: true,
      button: 'Install, not verified',
      line: `No file kryo.to lists for ${title} is this size, so this is not Kryoto's copy: an older build, a re-pack or a changed copy. It installs marked as not verified against Kryoto's copy.`,
    }
  }
  const what = look.addon ? `${addonName(look.addon)} for ${title}` : look.version ? `${title}, build ${look.version}` : title
  return {
    kind: 'install',
    anyway: false,
    button: 'Install',
    line: look.reachable
      ? `${look.sizeMatch ? 'This looks like' : 'This is for'} ${what}. Kryoto checks it against kryo.to's copy as it installs.`
      : `${what}. kryo.to did not answer, so the file is checked against its copy when it can be.`,
  }
}

/** The kinds of file a drop or the picker hands over: Kryoto's own downloads are .7z. */
export const ARCHIVE_EXTENSIONS = ['7z']

/** The file a drop of several takes: the first .7z, else the first file (the app says why it cannot take it). */
export function pickDropped(paths: readonly string[]): string | null {
  return paths.find((p) => /\.7z$/i.test(p)) ?? paths[0] ?? null
}
