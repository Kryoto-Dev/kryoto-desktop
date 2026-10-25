import { useSyncExternalStore, type ReactNode } from 'react'
import { TeamGem } from '@/ui/PixelBadges'
import { catalogApiUrl } from '@/lib/endpoint'
import { isTauri, isWeb } from '@/lib/bridge'
import { openStorePage } from '@/lib/window'

const EMPTY: ReadonlySet<string> = new Set()
let members: ReadonlySet<string> = EMPTY
let inflight: Promise<void> | null = null
let timer: ReturnType<typeof setInterval> | undefined
const listeners = new Set<() => void>()
async function refresh() {
  if (inflight) return inflight
  inflight = (async () => {
    try {
      // Browser fixture identities must never be confused with real accounts.
      if (!isTauri() && !isWeb()) { members = EMPTY; return }
      const response = await fetch(await catalogApiUrl('/api/community/team'), { cache: 'no-store', credentials: 'omit' })
      if (!response.ok) throw new Error('Team unavailable')
      const data: { usernames?: unknown } = await response.json()
      members = new Set(Array.isArray(data.usernames) ? data.usernames.filter((name): name is string => typeof name === 'string' && name.length > 0).map(name => name.toLowerCase()) : [])
    } catch { members = EMPTY }
    finally { listeners.forEach(listener => listener()) }
  })().finally(() => { inflight = null })
  return inflight
}
function subscribe(listener: () => void) {
  listeners.add(listener)
  if (listeners.size === 1) {
    void refresh()
    timer = setInterval(() => { if (document.visibilityState === 'visible') void refresh() }, 60_000)
    window.addEventListener('focus', focus)
  }
  return () => {
    listeners.delete(listener)
    if (!listeners.size) { clearInterval(timer); window.removeEventListener('focus', focus); members = EMPTY }
  }
}
function focus() { void refresh() }
const none = () => () => {}
const snapshot = () => members
const serverSnapshot = () => EMPTY

/** Canonical usernames only; hidden identities and account-less labels pass null. */
export function TeamName({ username, children }: { username: string | null | undefined; children: ReactNode }) {
  const team = useSyncExternalStore(username ? subscribe : none, snapshot, serverSnapshot)
  return <>{children}{username && team.has(username.toLowerCase()) ? <TeamGem className="ml-[0.3em] align-[-0.08em]" /> : null}</>
}

/** Use around an identity, beside its actions, so a name never nests a link inside a button. */
export function ProfileLink({ username, children, className, onProfile }: { username: string | null | undefined; children: ReactNode; className?: string; onProfile?: () => void }) {
  if (!username || (!isTauri() && !isWeb())) return <span className={className}>{children}</span>
  const path = `/user/${encodeURIComponent(username)}`
  return <a href={`https://kryo.to${path}`} className={className} onClick={event => { event.preventDefault(); if (onProfile) onProfile(); else openStorePage(path) }}>{children}</a>
}
