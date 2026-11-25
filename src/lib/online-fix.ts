/**
 * Properties > Emulator & fixes > Online fix (src-tauri online_fix.rs), in a
 * player's words: what Online-Fix's fix is, whether this release has one,
 * and what online-fix.me has for it. Pure, for `pnpm test`.
 */

/** online_fix.rs `Lookup`. */
export type OnlineFixLookup = {
  status: 'found' | 'none' | 'unreachable' | 'signin' | ''
  page: string | null
  file: string | null
  size: string | null
  madeFor: string | null
  newerNeeded: boolean
  detail: string | null
}

/** online_fix.rs `OnlineFixState`. */
export type OnlineFixState = {
  blocked: string | null
  carried: boolean
  applied: boolean
  sevenZip: boolean
  installed: string | null
  lookup: OnlineFixLookup | null
}

/** What the row says it is, once, in its hint. */
export const ONLINE_FIX_HINT =
  "Online-Fix's fix lets you play online, through Steam, with friends who use the same fix. It is not Kryoto Online. Everyone needs the same game version. Kryoto keeps a copy of every file it replaces."

/** The row's main line, and what can be done from it. */
export type OnlineFixView = {
  line: string
  /** A second, quieter line: the version it is made for. */
  version: string | null
  /** Get the online fix (from online-fix.me, in a window). */
  get: 'primary' | 'again' | null
  /** I have the file. */
  fromFile: boolean
  /** Open the game's page on online-fix.me. */
  openPage: boolean
  /** Look again. */
  retry: boolean
  /** Remove it (Back to the release's own files). */
  remove: boolean
}

const NONE: Omit<OnlineFixView, 'line'> = { version: null, get: null, fromFile: false, openPage: false, retry: false, remove: false }

/** "Made for version 1.0.11." and, when it is newer than the one installed, what that means. */
export function versionLine(l: OnlineFixLookup | null, installed: string | null): string | null {
  if (!l?.madeFor) return null
  if (l.newerNeeded) return `Made for version ${l.madeFor}, newer than yours (${installed ?? 'unknown'}). Update the game first, or it may not start or connect.`
  return `Made for version ${l.madeFor}. It may need that same version.`
}

/** Null while online-fix.me is being asked (and nothing else is known). */
export function onlineFixView(s: OnlineFixState): OnlineFixView | null {
  const l = s.lookup
  if (s.applied) {
    return { ...NONE, line: "On: Online-Fix's fix, put in by Kryoto. Start the game with Steam running.", version: versionLine(l, s.installed), remove: true }
  }
  if (s.carried) {
    return {
      ...NONE,
      line: 'This release already has an online fix. Start the game with Steam running.',
      version: versionLine(l, s.installed),
      // Its files again, from online-fix.me: what puts them back after an antivirus took them.
      get: l?.status === 'found' ? 'again' : null,
    }
  }
  if (!l) return null
  switch (l.status) {
    case 'found':
      return {
        ...NONE,
        line: `online-fix.me has an online fix for this game${l.size ? ` (${l.size.replace(/([0-9])([KMG])$/, '$1 $2B')})` : ''}.`,
        version: versionLine(l, s.installed),
        get: 'primary',
        fromFile: true,
      }
    case 'signin':
      return { ...NONE, line: "online-fix.me wants you to sign in before it shows this game's files. Open its page, sign in there (a free account), then look again.", openPage: true, retry: true, fromFile: true }
    case 'unreachable':
      return { ...NONE, line: 'online-fix.me did not answer. It may be down or blocked where you are. Try again later, or download the fix in your browser and use I have the file.', retry: true, openPage: true, fromFile: true }
    default:
      // The reason when the folder gives one (only an Xbox app fix there).
      return { ...NONE, line: l.detail ?? 'online-fix.me has no online fix for this game.' }
  }
}
