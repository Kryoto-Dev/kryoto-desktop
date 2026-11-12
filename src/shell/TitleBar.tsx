import { NewDot } from '@/ui/FeatureNotice'
import { ProfileLink, TeamName } from '@/ui/TeamName'
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react'
import { flushSync } from 'react-dom'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Bell, ChevronDown, Expand, Megaphone, Shrink } from '@/ui/pixel-icons'
import { artSrc } from '@/lib/art'
import { Matrix, MenuButton, MenuList, type MenuEntry } from '@/ui'
import { KryoMark } from '@/ui/ascii/KryoMark'
import { DevEndpointNotice } from '@/ui/DevEndpointNotice'
import { PreviewReportButton } from '@/shell/PreviewReport'
import { KryosCoin, compactKryos } from '@/ui/KryosCoin'
import { closeWindow, isTauri, minimizeWindow, toggleFullscreen, toggleMaximize, useWindowState } from '@/lib/window'
import { openInbox } from '@/lib/popup'
import { cn } from '@/lib/utils'
import { isWindowsHost } from '@/lib/library'
import type { Account } from '@/hooks/useAccount'
import type { Inbox, News } from '@/hooks/useInbox'

/**
 * What gives way when the row is short, in order. First room is won back: the
 * tabs drop their icons, Add to library becomes its icon, the address may get
 * narrower than it likes. Then the menus (Kryoto, View, Games, Help) fold into
 * one menu on the K// mark, and the rest folds into the account menu. The row
 * never wraps: each step is one thing a menu can do too, so a narrow window
 * loses room and not features.
 */
export const FOLD = { tabIcons: 1, addIcon: 2, menus: 3, address: 4, news: 5, kryos: 6, fullscreen: 7, profile: 8, labels: 9 }

/**
 * How far the row has to fold to fit, measured and not guessed from the window:
 * a long name or a big font needs it sooner. `content` is whatever changes the
 * row's width without the window changing.
 */
export function useRowFold(content: string) {
  const row = useRef<HTMLElement | null>(null)
  const [fold, setFold] = useState(0)
  const [width, setWidth] = useState(0)
  useEffect(() => {
    if (!row.current) return
    // In step with the resize, so a frame never shows the row unfolded.
    const el = row.current
    const watch = new ResizeObserver(() =>
      flushSync(() => {
        setFold(0)
        setWidth(el.clientWidth)
      }),
    )
    watch.observe(el)
    return () => watch.disconnect()
  }, [])
  useLayoutEffect(() => setFold(0), [content])
  useLayoutEffect(() => {
    // The navigation takes what is left, so it is where the row runs out.
    const nav = row.current?.querySelector('nav')
    if (nav && nav.scrollWidth > nav.clientWidth && fold < FOLD.labels) setFold(fold + 1)
  }, [fold, width, content])
  return { row, fold }
}

/**
 * The window's one row: the mark, the menus, the client's navigation (`nav`), then what is
 * new on kryo.to, the bell, the Kryos, the account, full screen and the window
 * buttons. The empty parts drag the window and a double-click on them
 * maximizes. They are `.drag` areas beside the controls, never around them:
 * inside a drag area a control loses its cursor. Menus open in the menu view,
 * or the Store's page when the Store is showing, over it without hiding it.
 */
export function TitleBar({
  offline = false,
  account,
  inbox,
  news,
  menus,
  accountMenu,
  newSettings = false,
  nav,
  rowRef,
  fold = 0,
  onNews,
  onOpenNotification,
  onMarkRead,
  onAllNotifications,
  onKryos,
  where = '',
}: {
  /** No connection: a quiet pill says so, and what needs kryo.to waits. */
  offline?: boolean
  account: Account
  inbox: Inbox
  news: News | null
  /** Kryoto, View, Games, Help: their own buttons, or one menu on the mark when the row is short. */
  menus: { label: string; items: MenuEntry[] }[]
  accountMenu: MenuEntry[]
  /** A dot on the account menu: something new in Settings the player has not opened. */
  newSettings?: boolean
  /** Back, forward, the tabs and the address: `NavBar`. */
  nav: ReactNode
  rowRef: RefObject<HTMLElement | null>
  /** How much has folded away (`useRowFold`). */
  fold?: number
  onNews: () => void
  /** `id`: the notification opened, so it is marked read (KB-0291). */
  onOpenNotification: (url: string | null, id?: string) => void
  onMarkRead: () => void
  onAllNotifications: () => void
  /** The coin beside the account: opens the Kryos wallet in the Store. */
  onKryos: () => void
  /** Where the player is, for Kryoto Desktop Preview's Report button: a Store page or a tab. */
  where?: string
}) {
  const { fullscreen } = useWindowState()

  const initial = (account.displayName || account.username || '?').slice(0, 1).toUpperCase()

  return (
    <header ref={rowRef} className="relative z-50 flex h-13 shrink-0 select-none items-stretch border-b border-border bg-background">
      {fold >= FOLD.menus ? (
        <div className="no-drag flex shrink-0 items-center pl-2 pr-1">
          <MenuButton
            native="title:menus"
            label="Menu"
            trigger={<KryoMark className="pointer-events-none h-3.5" />}
            items={menus.flatMap((m, i) => [...(i ? [{ separator: true } as MenuEntry] : []), { heading: m.label } as MenuEntry, ...m.items])}
            className="kryo-pill flex h-8 items-center px-1.5 hover:bg-secondary aria-expanded:bg-secondary"
          />
        </div>
      ) : (
        <>
          <div data-maximize className="drag flex shrink-0 items-center pl-3.5 pr-2">
            <KryoMark className="pointer-events-none h-3.5" />
          </div>
          <div role="group" aria-label="Menus" className="no-drag flex shrink-0 items-center pr-2">
            {menus.map((m) => (
              <MenuButton
                key={m.label}
                native={`title:${m.label}`}
                trigger={m.label}
                items={m.items}
                className="kryo-pill h-7 px-2.5 text-[11px] uppercase tracking-wider text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground aria-expanded:bg-secondary aria-expanded:text-foreground"
              />
            ))}
          </div>
        </>
      )}
      {nav}
      <div data-maximize className="drag min-w-6 flex-1" />

      <div className="no-drag flex shrink-0 items-center gap-2 pr-1.5">
        {offline ? (
          <span
            title="No connection. The library, your games and settings work as usual; the Store and downloads wait for it."
            className="kryo-pill flex h-6 items-center gap-1.5 border border-border px-2 text-[10px] uppercase tracking-wider text-muted-foreground"
          >
            <Matrix state="connect" className="size-3" />
            Offline
          </span>
        ) : null}
        <DevEndpointNotice compact />
        {/* Kryoto Desktop Preview only (src/lib/endpoint.ts IS_PREVIEW_BUILD). */}
        <PreviewReportButton page={where} username={account.guest ? null : account.username} />
        {news?.version && fold < FOLD.news ? (
          <button
            type="button"
            onClick={onNews}
            title={`What's new on kryo.to - ${news.version}`}
            className="kryo-pill flex h-7 items-center gap-1.5 px-2.5 text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
          >
            <Megaphone className="size-3.5" />
            <span>{news.version}</span>
          </button>
        ) : null}

        {account.guest ? null : (
          <MenuButton
            native="bell"
            label={`Notifications${inbox.unreadCount ? `, ${inbox.unreadCount} unread` : ''}`}
            align="right"
            nativeOpen={(anchor) =>
              void openInbox('bell', anchor, inbox, { open: onOpenNotification, markRead: onMarkRead, all: onAllNotifications })
            }
            className="kryo-pill relative grid size-7 place-items-center text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground aria-expanded:bg-secondary"
            trigger={
              <>
                <Bell className="size-3.5" />
                {inbox.unreadCount ? (
                  <span className="kryo-pill absolute -right-0.5 -top-0.5 grid h-3.5 min-w-3.5 place-items-center bg-primary px-1 text-[8px] font-bold text-primary-foreground">
                    {inbox.unreadCount > 9 ? '9+' : inbox.unreadCount}
                  </span>
                ) : null}
              </>
            }
            panel={(close) => (
              <div className="w-80">
                <MenuList
                  onDone={close}
                  items={[
                    { heading: 'Notifications' },
                    ...inbox.notifications.map((n) => ({ label: n.title, onSelect: () => onOpenNotification(n.url, n.id) })),
                    { separator: true },
                    ...(inbox.unreadCount ? [{ label: 'Mark all read', onSelect: onMarkRead }] : []),
                    { label: 'See all', onSelect: onAllNotifications },
                  ]}
                />
              </div>
            )}
          />
        )}

        {/* Your Kryos, as on kryo.to: the coin and the amount. A click opens
            the wallet, where they are claimed and spent. */}
        {!account.guest && typeof account.kryos === 'number' && fold < FOLD.kryos ? (
          <button
            type="button"
            onClick={onKryos}
            title={`${account.kryos.toLocaleString('en-GB')} Kryos · claim and spend them`}
            aria-label={`You have ${account.kryos.toLocaleString('en-GB')} Kryos. Open your wallet`}
            className="kryo-pill flex h-7 items-center gap-1.5 border border-border px-2.5 text-[11px] tabular-nums text-foreground transition-colors hover:border-foreground"
          >
            <KryosCoin size={14} className="shrink-0" />
            {compactKryos(account.kryos)}
          </button>
        ) : null}

        <div className="kryo-pill flex h-7 items-center gap-1 border border-border pl-0.5 pr-1 text-[11px] text-foreground">
          <ProfileLink username={account.guest ? null : account.username} className="kryo-pill inline-flex min-w-0 items-center gap-2 hover:text-primary focus-visible:outline-2 focus-visible:outline-primary">
              {account.avatarUrl ? (
                <img decoding="async" src={artSrc(account.avatarUrl) ?? undefined} alt="" className="kryo-pill size-6 object-cover" />
              ) : (
                <span className="kryo-pill grid size-6 place-items-center bg-secondary text-[10px] font-bold">{initial}</span>
              )}
              <span className="max-w-40 truncate"><TeamName username={account.guest ? null : account.username}>{account.displayName || account.username}</TeamName></span>
          </ProfileLink>
          <MenuButton native="account" label="Account menu" align="right" items={accountMenu} className="kryo-pill p-1 text-muted-foreground hover:text-foreground" trigger={<span className="relative inline-flex"><ChevronDown className="size-3" aria-hidden />{newSettings ? <NewDot className="absolute -right-1 -top-1" /> : null}</span>} />
        </div>

        {isTauri() && fold < FOLD.fullscreen ? (
          <button
            type="button"
            aria-label={fullscreen ? 'Leave full screen' : 'Full screen'}
            title={fullscreen ? 'Leave full screen (F11)' : 'Full screen (F11)'}
            onClick={() => void toggleFullscreen()}
            className="kryo-pill grid size-7 place-items-center text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
          >
            {fullscreen ? <Shrink className="size-3.5" /> : <Expand className="size-3.5" />}
          </button>
        ) : null}
      </div>

      {isTauri() ? <WindowControls /> : null}
    </header>
  )
}

/**
 * Minimize, maximize or restore, and close, as Windows 11 draws them: its own
 * glyphs (Segoe Fluent Icons, or MDL2 Assets on Windows 10), 46 pixels wide
 * and the bar's full height, the red close, and dimmed while the window is
 * not the one in front. Elsewhere the same buttons drawn as one-pixel lines.
 * Maximize follows the window (a drag to the top of the screen, a double-click
 * on the bar, Win+Up), not just this button, and hides in full screen.
 */
export function WindowControls() {
  const { maximized, fullscreen } = useWindowState()
  const active = useWindowFocus()
  const glyphs = isWindowsHost()
  return (
    <div className={cn('no-drag flex items-stretch', !active && 'opacity-60')}>
      <WindowButton label="Minimize" onClick={() => void minimizeWindow()} glyph={glyphs ? '' : undefined}>
        <path d="M0 5.5h10" />
      </WindowButton>
      {fullscreen ? null : (
        <WindowButton
          label={maximized ? 'Restore down' : 'Maximize'}
          onClick={() => void toggleMaximize()}
          glyph={glyphs ? (maximized ? '' : '') : undefined}
        >
          {maximized ? (
            <>
              <path d="M2.5 2.5V0.5h7v7h-2" />
              <rect x="0.5" y="2.5" width="7" height="7" />
            </>
          ) : (
            <rect x="0.5" y="0.5" width="9" height="9" />
          )}
        </WindowButton>
      )}
      <WindowButton label="Close" onClick={() => void closeWindow()} danger glyph={glyphs ? '' : undefined}>
        <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" />
      </WindowButton>
    </div>
  )
}

/** Whether this window is the one in front, as Windows dims its buttons when not. */
function useWindowFocus() {
  const [active, setActive] = useState(() => (typeof document === 'undefined' ? true : document.hasFocus()))
  useEffect(() => {
    if (!isTauri()) return
    let stop: (() => void) | undefined
    let cancelled = false
    void getCurrentWindow()
      .onFocusChanged(({ payload }) => setActive(payload))
      .then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])
  return active
}

function WindowButton({
  label,
  onClick,
  danger = false,
  glyph,
  children,
}: {
  label: string
  onClick: () => void
  danger?: boolean
  /** A Segoe glyph (Windows), else `children` as one-pixel lines. */
  glyph?: string
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      // A press that turns into a drag would otherwise start moving the
      // window from its button.
      onMouseDown={(e) => e.stopPropagation()}
      onClick={onClick}
      className={cn(
        'kryo-square grid w-[46px] place-items-center text-foreground/90 transition-colors duration-75',
        danger
          ? 'hover:bg-[#c42b1c] hover:text-white active:bg-[#c42b1c]/90 active:text-white/80'
          : 'hover:bg-foreground/[0.06] active:bg-foreground/[0.04] active:text-foreground/60',
      )}
    >
      {glyph ? (
        <span className="kryo-caption" aria-hidden>
          {glyph}
        </span>
      ) : (
        // One device pixel wide at any display scale.
        <svg
          viewBox="0 0 10 10"
          className="size-2.5 overflow-visible [&_*]:[vector-effect:non-scaling-stroke]"
          fill="none"
          stroke="currentColor"
          strokeWidth="1"
          aria-hidden
        >
          {children}
        </svg>
      )}
    </button>
  )
}
