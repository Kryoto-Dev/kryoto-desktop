/**
 * A release's own file list (src-tauri release_files.rs): Check files, the
 * files it names, and getting them back. The words for what each file is live
 * here, so the game page's antivirus card, Properties > Emulator & fixes and
 * the notification after a failed start say it the same way.
 *
 * Pure apart from `files`, for the tests beside it.
 */

/** What a listed file is (kryo.to lib/release-files-summary.ts `ROLES`). */
export type FileRole = 'game' | 'emulator-added' | 'emulator-replaced' | 'fix' | 'kryoto' | 'other'
export type FileProblem = 'missing' | 'broken' | 'changed'
export type FileFinding = { path: string; role: FileRole; problem: FileProblem }

/** release_files.rs `CheckReport`. */
export type FilesReport = {
  releaseId: string
  at: number
  full: boolean
  files: number
  hashed: number
  skipped: number
  missing: FileFinding[]
  broken: FileFinding[]
  changed: number
  changedSample: FileFinding[]
  cancelled: boolean
}

/** release_files.rs `FilesState`. */
export type FilesState = {
  list: 'yes' | 'none' | 'unknown'
  files: number
  report: FilesReport | null
  checking: boolean
  downloadSize: string | null
}

/** release_files.rs `Restored`. */
export type FilesRestored = { restored: string[]; fromArchive: number; left: string[]; downloadSize: string | null }

export type FilesProgress = { gameId: string; done: number; total: number; bytesDone: number; bytesTotal: number }
export type RestoreProgress = { gameId: string; text: string; percent: number | null }

/** What each kind of file is, in a few words. */
export function roleWords(role: FileRole): string {
  switch (role) {
    case 'emulator-added':
      return 'added by the emulator'
    case 'emulator-replaced':
      return "the emulator's Steam file"
    case 'fix':
      return "part of the game's fix"
    case 'kryoto':
      return "Kryoto's start file"
    default:
      return 'part of the game'
  }
}

/** The file's own name, without its folder. */
export const fileName = (path: string) => path.split('/').pop() || path

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many)

/** "2 files are missing", "1 file is damaged". */
export function countLine(n: number, problem: 'missing' | 'broken'): string {
  const what = problem === 'missing' ? 'missing' : 'damaged or replaced'
  return `${n.toLocaleString('en-US')} ${plural(n, 'file is', 'files are')} ${what}`
}

/** Only what raises the card: missing files and broken emulator programs. */
export function alarms(findings: FileFinding[]): FileFinding[] {
  return findings.filter((f) => f.problem === 'missing' || f.problem === 'broken')
}

/**
 * One plain sentence per problem, for a notification or a log line:
 * "2 files are missing: steam_api64.dll (added by the emulator), PlayGTAV.exe (part of the game)."
 * `max` names at most that many files in each, then "and N more".
 */
export function plainSummary(findings: FileFinding[], max = 3): string {
  const parts: string[] = []
  for (const problem of ['missing', 'broken'] as const) {
    const these = findings.filter((f) => f.problem === problem)
    if (!these.length) continue
    const named = these.slice(0, max).map((f) => `${fileName(f.path)} (${roleWords(f.role)})`)
    const more = these.length > max ? ` and ${these.length - max} more` : ''
    parts.push(`${countLine(these.length, problem)}: ${named.join(', ')}${more}.`)
  }
  return parts.join(' ')
}

/** "just now", "5 minutes ago", "on 3 Nov": when the last check ran. */
export function checkedWhen(at: number, now = Date.now() / 1000): string {
  const s = Math.max(0, Math.round(now - at))
  if (s < 60) return 'just now'
  if (s < 3600) return `${Math.round(s / 60)} ${plural(Math.round(s / 60), 'minute', 'minutes')} ago`
  if (s < 86400) return `${Math.round(s / 3600)} ${plural(Math.round(s / 3600), 'hour', 'hours')} ago`
  return `on ${new Date(at * 1000).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })}`
}

/** What a finished Get the files back says. */
export function restoredLine(r: FilesRestored): string {
  const back = r.restored.length
  const parts: string[] = []
  if (back) {
    const from = r.fromArchive === back ? ' from the download Kryoto kept' : r.fromArchive ? `, ${r.fromArchive} from the download Kryoto kept` : ''
    parts.push(`${back.toLocaleString('en-US')} ${plural(back, 'file is', 'files are')} back${from}.`)
  }
  if (r.left.length) {
    const size = r.downloadSize ? ` (${r.downloadSize})` : ''
    parts.push(
      `${r.left.length.toLocaleString('en-US')} ${plural(r.left.length, 'file is', 'files are')} not on this PC any more. Downloading the game again${size} brings ${plural(r.left.length, 'it', 'them')} back: your saves and settings stay.`,
    )
  }
  return parts.join(' ') || 'Nothing was missing.'
}
