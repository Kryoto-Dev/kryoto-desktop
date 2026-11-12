import { call, errorText, isTauri, on } from '@/lib/bridge'
import { logError, logWarn } from '@/lib/log'

/**
 * Play sessions on their way to kryo.to, kept until they arrive.
 *
 * A session is reported through the Store page's own session, so a game closed
 * while that page was not loaded ("Open kryo.to first", 16 reports in 0.2.3), or
 * while kryo.to was down, used to lose its play time for good. It waits here
 * instead, in this PC's storage, until kryo.to has answered for it. The key on
 * each makes a second send harmless: kryo.to counts a session once.
 */

type PendingPlay = { slug: string; startedAt: number; seconds: number; key: string }

const STORE_KEY = 'kryoto.pendingPlays'
/** Older than this and the site would not take it anyway. */
const MAX_AGE_SECS = 14 * 86_400

function read(): PendingPlay[] {
  try {
    const parsed = JSON.parse(localStorage.getItem(STORE_KEY) ?? '[]') as PendingPlay[]
    return Array.isArray(parsed) ? parsed : []
  } catch {
    return []
  }
}

function write(list: PendingPlay[]) {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(list.slice(-200)))
  } catch {
    // Storage full or off: the session is lost, as it was before.
  }
}

/** Expected while the Store is not loaded, or for a game kryo.to does not list. */
export function isExpectedReportError(e: unknown): boolean {
  const text = errorText(e)
  return text.startsWith('Open kryo.to first') || text.startsWith('Not a kryo.to game')
}

/** Seconds of this game's sessions still on their way to kryo.to (`playtime-sync.ts`). */
export function pendingPlaySeconds(slug: string): number {
  return read()
    .filter((p) => p.slug === slug)
    .reduce((sum, p) => sum + (p.seconds || 0), 0)
}

function done(key: string) {
  write(read().filter((p) => p.key !== key))
}

// kryo.to has the session, or has refused it for good (store_play_done). Until
// then it stays here: a page sending to a site that is down hears nothing back.
if (isTauri()) void on<string>('play-reported', done)

async function send(play: PendingPlay) {
  try {
    await call('store_report_play', play)
  } catch (e) {
    if (errorText(e).startsWith('Open kryo.to first')) return logWarn('playtime', `${play.slug}: kept for later (${errorText(e)})`)
    // Not one kryo.to can ever take.
    done(play.key)
    if (isExpectedReportError(e)) logWarn('playtime', `${play.slug}: ${errorText(e)}`)
    else logError('playtime', e)
  }
}

/** Keep one session, and send it. */
export async function reportPlay(play: PendingPlay): Promise<void> {
  write([...read().filter((p) => p.key !== play.key), play])
  await send(play)
}

/** Try every kept session again: on a timer, and when kryo.to is back. Quiet when there is nothing. */
export async function flushPlays(): Promise<void> {
  const now = Math.floor(Date.now() / 1000)
  const list = read().filter((p) => now - p.startedAt < MAX_AGE_SECS)
  write(list)
  for (const play of list) await send(play)
}
