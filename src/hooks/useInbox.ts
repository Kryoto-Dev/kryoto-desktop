import { isExpectedReportError } from '@/lib/play-reports'
import { useCallback, useEffect, useState } from 'react'
import { call, isTauri, on } from '@/lib/bridge'
import { logError } from '@/lib/log'
import { notify } from '@/lib/notify'

export type KryoNotification = {
  id: string
  title: string
  body: string
  url: string | null
  image: string | null
  readAt: string | null
  createdAt: string
  /** Already shown as a system notification on another device (kryo.to). */
  shown?: boolean
}

export type Inbox = { unreadCount: number; notifications: KryoNotification[] }
export type News = { version: string | null; date: string | null }

const PREVIEW_INBOX: Inbox = {
  unreadCount: 2,
  notifications: [
    {
      id: 'p1',
      title: 'Captain Hardcore was updated',
      body: 'A new build is up - get it from the game page.',
      url: '/game/captain-hardcore',
      image: null,
      readAt: null,
      createdAt: new Date().toISOString(),
    },
    {
      id: 'p2',
      title: 'Your request was filled',
      body: 'Hades II is on kryo.to now.',
      url: '/game/hades-ii',
      image: null,
      readAt: null,
      createdAt: new Date().toISOString(),
    },
  ],
}

/**
 * kryo.to's notification bell and "what's new", as the Store's page reports
 * them - read with the reader's own session, so nothing here holds a token.
 */
export function useInbox() {
  const [inbox, setInbox] = useState<Inbox>(isTauri() ? { unreadCount: 0, notifications: [] } : PREVIEW_INBOX)
  const [news, setNews] = useState<News | null>(isTauri() ? null : { version: '0.4.1', date: null })
  useEffect(() => {
    const stops: Array<() => void> = []
    let cancelled = false
    // The first list is what was already there; after that, anything unread
    // and new is also shown by the system, as the browser would.
    let seen: Set<string> | null = null
    void on<Inbox>('inbox-state', (i) => {
      const list = i.notifications ?? []
      if (seen) {
        // Only what no other device showed already (a browser push, the
        // phone); and tell kryo.to, so those skip what is shown here.
        const fresh = list.filter((n) => !n.readAt && !n.shown && !seen!.has(n.id))
        for (const n of fresh) void notify(n.title, n.body)
        if (fresh.length && isTauri()) void call('store_notifications_shown', { ids: fresh.map((n) => n.id) }).catch(() => {})
      }
      seen = new Set([...(seen ?? []), ...list.map((n) => n.id)])
      setInbox({ unreadCount: i.unreadCount ?? 0, notifications: list })
    }).then(
      (fn) => (cancelled ? fn() : stops.push(fn)),
    )
    void on<News>('news-state', setNews).then((fn) => (cancelled ? fn() : stops.push(fn)))
    return () => {
      cancelled = true
      stops.forEach((s) => s())
    }
  }, [])
  // Shown as read straight away; the page reports the real list once kryo.to
  // has it (store_mark_read asks it to look again).
  const markAllRead = useCallback(() => {
    const now = new Date().toISOString()
    setInbox((i) => ({ unreadCount: 0, notifications: i.notifications.map((n) => (n.readAt ? n : { ...n, readAt: now })) }))
    void call('store_mark_read').catch((e) => (isExpectedReportError(e) ? undefined : logError('inbox', e)))
  }, [])
  return { inbox, news, markAllRead }
}
