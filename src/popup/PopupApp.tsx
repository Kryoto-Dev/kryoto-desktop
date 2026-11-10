import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { Check as CheckIcon } from '@/ui/pixel-icons'
import { call, on } from '@/lib/bridge'
import type { PopupItem, PopupPayload } from '@/lib/popup'
import { cn } from '@/lib/utils'
import { NewDot } from '@/ui/FeatureNotice'

/**
 * The menu view's page: draws the menu (or the notification list) the shell
 * sent, says how big it came out so the view can be sized to it, and reports
 * the pick. Arrow keys, Enter and Escape work as in any menu, and focus
 * leaving the view (a click on the shell or the Store, another app) closes
 * it.
 */
export function PopupApp() {
  const [payload, setPayload] = useState<PopupPayload | null>(null)
  const [seq, setSeq] = useState(0)
  const box = useRef<HTMLDivElement | null>(null)
  // Focus has reached the view since this menu opened, so losing it means
  // the reader went somewhere else (and not that it never arrived).
  const focused = useRef(false)
  // Which menu this page is showing, so losing focus closes that one and never
  // the next one, asked for while this view was hidden to redraw.
  const showing = useRef<string | null>(null)
  // Counts the menus drawn, so a close decided for one never lands on the next.
  const asked = useRef(0)

  useEffect(() => {
    const take = (p: PopupPayload | null) => {
      if (!p) return
      const html = document.documentElement
      html.classList.toggle('dark', p.look.theme !== 'light')
      // Light on Linux whatever the theme: see applyLook (KB-0006).
      html.style.colorScheme = p.look.theme === 'light' || html.dataset.os === 'linux' ? 'light' : 'dark'
      html.dataset.palette = p.look.palette
      html.dataset.radius = p.look.radius
      if (p.look.season) html.dataset.season = p.look.season
      else delete html.dataset.season
      if (p.look.font) html.dataset.font = p.look.font
      else delete html.dataset.font
      focused.current = false
      showing.current = p.menu
      asked.current += 1
      setPayload(p)
      setSeq((n) => n + 1)
    }
    let stop: (() => void) | undefined
    let cancelled = false
    // Listening first: a menu asked for in between must not fall in the gap.
    void on<PopupPayload>('menu-show', take)
      .then((fn) => (cancelled ? fn() : (stop = fn)))
      .then(() => call<PopupPayload | null>('menu_payload'))
      .then((p) => p && take(p))
    let leaving: number | undefined
    const onFocus = () => {
      focused.current = true
      window.clearTimeout(leaving)
    }
    // Focus has to stay gone. The view can bounce it for a few milliseconds as
    // it comes up (on a busy PC, or with the window not in front), and closing
    // on the first blur took the menu away as it opened.
    const onBlur = () => {
      const menu = showing.current
      // Hidden, it can still get focus back from the window, and lose it to the
      // press that opens the next menu: that is no reason to close that one.
      if (!focused.current || !menu || document.visibilityState !== 'visible') return
      const request = asked.current
      window.clearTimeout(leaving)
      leaving = window.setTimeout(() => {
        if (document.hasFocus() || asked.current !== request) return
        focused.current = false
        void call('menu_close', { menu })
      }, 120)
    }
    window.addEventListener('focus', onFocus)
    window.addEventListener('blur', onBlur)
    return () => {
      cancelled = true
      stop?.()
      window.clearTimeout(leaving)
      window.removeEventListener('focus', onFocus)
      window.removeEventListener('blur', onBlur)
    }
  }, [])

  // Measure each new menu once its fonts are in, then have it placed and shown.
  // offsetWidth/Height, not getBoundingClientRect: the opening animation
  // scales the box, and a measure mid-scale came out a few pixels short.
  useLayoutEffect(() => {
    if (!payload || !box.current) return
    let cancelled = false
    // Not for ever: a hidden page can leave this waiting for a layout that never comes.
    void Promise.race([document.fonts.ready, new Promise((done) => window.setTimeout(done, 300))]).then(() => {
      const el = box.current
      if (cancelled || !el) return
      // With this page's own pixel ratio: it can differ from the shell's.
      void call('menu_ready', { menu: payload.menu, width: el.offsetWidth, height: el.offsetHeight, scale: window.devicePixelRatio })
        .then(() => {
          if (cancelled || !box.current || payload.kind === 'tip') return
          if (document.hasFocus()) focused.current = true
          box.current.focus({ preventScroll: true })
        })
        .catch(() => {})
    })
    return () => {
      cancelled = true
    }
  }, [payload, seq])

  const select = useCallback((id: string) => void call('menu_pick', { id }), [])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') return void call('menu_close', { menu: showing.current, back: true })
      if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
      e.preventDefault()
      const items = [...(box.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled)') ?? [])]
      const at = items.indexOf(document.activeElement as HTMLElement)
      const step = e.key === 'ArrowDown' ? 1 : -1
      const next = at < 0 ? items[step > 0 ? 0 : items.length - 1] : items[(at + step + items.length) % items.length]
      next?.focus()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  if (!payload) return null
  if (payload.kind === 'tip') {
    return (
      <div ref={box} key={seq} role="tooltip" className="kryo-square w-max max-w-64 border border-border bg-popover px-2 py-1 text-[10px] leading-snug text-popover-foreground">
        {payload.text}
      </div>
    )
  }
  return (
    <div
      ref={box}
      key={seq}
      role="menu"
      tabIndex={-1}
      onPointerEnter={() => void call('menu_hover', { inside: true })}
      onPointerLeave={() => {
        box.current?.focus({ preventScroll: true })
        void call('menu_hover', { inside: false })
      }}
      // One item is lit at a time: the pointer moves focus, and focus is the
      // only highlight, so the keyboard and the pointer never light two rows.
      onPointerMove={(e) => {
        const item = (e.target as HTMLElement).closest<HTMLElement>('[role="menuitem"]:not(:disabled)')
        if (item && document.activeElement !== item) item.focus({ preventScroll: true })
      }}
      // One column that keeps to the panel: left to its content, a long title
      // stretched it past the edge, cut the text and pushed Mark all read out of sight.
      className="kryo-square grid w-max grid-cols-[minmax(0,1fr)] gap-0.5 overflow-hidden border border-border bg-popover p-1 outline-none"
      style={payload.kind === 'menu' ? { minWidth: payload.minWidth ?? 200, maxWidth: 420 } : { width: 320 }}
    >
      {payload.kind === 'menu' ? <Items items={payload.items} onSelect={select} /> : <InboxList payload={payload} onSelect={select} />}
    </div>
  )
}

function Items({ items, onSelect }: { items: PopupItem[]; onSelect: (id: string) => void }) {
  return (
    <>
      {items.map((item, i) =>
        'separator' in item ? (
          <div key={i} className="mx-2 my-1 h-px bg-border" />
        ) : 'heading' in item ? (
          <div key={i} className="px-3 pb-0.5 pt-1.5 text-[10px] uppercase tracking-wider text-muted-foreground">
            {item.heading}
          </div>
        ) : (
          <button
            key={item.id}
            type="button"
            role="menuitem"
            disabled={item.disabled}
            onClick={() => onSelect(item.id)}
            className={cn(
              'flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs outline-none disabled:opacity-40',
              item.danger
                ? 'text-destructive focus:bg-destructive focus:text-destructive-foreground'
                : 'text-foreground focus:bg-primary focus:text-primary-foreground',
            )}
          >
            <span
              className="grid size-3.5 shrink-0 place-items-center [&>svg]:size-3.5"
              // Our own lucide markup, rendered by the main window.
              dangerouslySetInnerHTML={item.icon ? { __html: item.icon } : undefined}
            >
              {item.icon ? undefined : item.checked ? <CheckIcon className="size-3.5" /> : null}
            </span>
            <span className="min-w-0 grow truncate pr-4">{item.label}</span>
            {item.hint ? <span className="whitespace-nowrap text-[10px] tracking-wider opacity-60">{item.hint}</span> : null}
          </button>
        ),
      )}
    </>
  )
}

function InboxList({ payload, onSelect }: { payload: Extract<PopupPayload, { kind: 'inbox' }>; onSelect: (id: string) => void }) {
  const { inbox, menu } = payload
  return (
    <>
      <div className="flex items-center justify-between gap-3 py-1 pl-3 pr-1">
        <span className="min-w-0 flex-1 truncate text-[10px] uppercase tracking-[0.25em] text-primary">Notifications</span>
        {inbox.unreadCount ? (
          <button
            type="button"
            role="menuitem"
            className="shrink-0 whitespace-nowrap px-2.5 py-1.5 text-[10px] uppercase tracking-wider text-muted-foreground outline-none focus:bg-secondary focus:text-foreground"
            onClick={() => onSelect(`${menu}:read`)}
          >
            Mark all read
          </button>
        ) : null}
      </div>
      {inbox.notifications.length === 0 ? (
        <p className="px-3 pb-3 pt-1 text-xs text-muted-foreground">Nothing new.</p>
      ) : (
        inbox.notifications.map((n, i) => (
          <button
            key={n.id}
            type="button"
            role="menuitem"
            onClick={() => onSelect(`${menu}:open:${i}`)}
            className="flex w-full items-start gap-2.5 px-3 py-2 text-left outline-none focus:bg-secondary"
          >
            <span className="min-w-0 flex-1 [overflow-wrap:anywhere]">
              <span className="line-clamp-2 text-xs text-foreground">{n.title}</span>
              <span className="line-clamp-2 text-[11px] leading-snug text-muted-foreground">{n.body}</span>
            </span>
            {n.readAt ? null : <NewDot className="mt-1.5" />}
          </button>
        ))
      )}
      <div className="mt-0.5 border-t border-border pt-0.5">
        <button
          type="button"
          role="menuitem"
          className="w-full px-3 py-2 text-left text-[10px] uppercase tracking-wider text-muted-foreground outline-none focus:bg-secondary focus:text-foreground"
          onClick={() => onSelect(`${menu}:all`)}
        >
          See all notifications
        </button>
      </div>
    </>
  )
}
