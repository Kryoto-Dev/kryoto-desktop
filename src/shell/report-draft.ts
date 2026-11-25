/**
 * What a Preview report says before it is sent (shell/PreviewReport.tsx), kept
 * so a stray click outside the dialog or its Close button does not throw it
 * away. The dialog reads it back when it opens and forgets it once the report
 * is sent. Only the words: the picture and added files are too big to keep.
 * One per account, since the report goes out as whoever is signed in when it
 * is sent. Pure over the storage it is given, for `pnpm test`.
 */

export type ReportDraft = { kind: 'bug' | 'idea'; what: string; steps: string }
type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>

/** Where the account's draft is kept: another account signed in on this PC never sees it. */
export const draftKey = (username: string) => `kryo:preview-report-draft:${username.toLowerCase()}`
/** The dialog's own limit on each box. */
const MAX = 4000

/** The kept draft, or none: nothing kept, or something this app did not write. */
export function readDraft(store: Store | null, username: string): ReportDraft | null {
  try {
    const raw: unknown = JSON.parse(store?.getItem(draftKey(username)) ?? 'null')
    if (!raw || typeof raw !== 'object') return null
    const { kind, what, steps } = raw as Record<string, unknown>
    const text = (v: unknown) => (typeof v === 'string' ? v.slice(0, MAX) : '')
    const draft: ReportDraft = { kind: kind === 'idea' ? 'idea' : 'bug', what: text(what), steps: text(steps) }
    return draft.what.trim() || draft.steps.trim() ? draft : null
  } catch {
    return null
  }
}

/** Keep it; boxes left empty keep nothing, so the next report starts clean. */
export function saveDraft(store: Store | null, username: string, draft: ReportDraft): void {
  try {
    if (!draft.what.trim() && !draft.steps.trim()) store?.removeItem(draftKey(username))
    else store?.setItem(draftKey(username), JSON.stringify(draft))
  } catch {
    // Storage refused (full, or turned off): the report still sends.
  }
}

/** Sent: the next report starts empty. */
export function clearDraft(store: Store | null, username: string): void {
  try {
    store?.removeItem(draftKey(username))
  } catch {
    // As above.
  }
}
