import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { ArrowLeft, ArrowRight, Library, Store, Users, type LucideIcon } from '@/ui/pixel-icons'
import { IconButton } from '@/ui'
import { cn } from '@/lib/utils'
import { FOLD } from '@/shell/TitleBar'

export type TopTab = 'store' | 'library' | 'community' | 'profile'

/** Store and Library are the mobile app's icons; Community has its own. */
const TAB_ICONS: Record<TopTab, LucideIcon> = { store: Store, library: Library, community: Users, profile: Users }

/** A tab's few pages, shown as a small panel. */
export type NavTabPanel = {
  heading: string
  items: { label: string; icon?: ReactNode; onSelect: () => void }[]
}

export type NavTabSpec = {
  id: TopTab
  label: string
  onOpen: () => void
  /** Shown on hover as a small panel. */
  panel?: NavTabPanel
}

/**
 * Back / forward, then Store · Library · Community as one sliding pill. A tab
 * with its own pages (Community) shows them in a small panel on hover.
 */
export function NavBar({
  current,
  tabs,
  canBack,
  canForward,
  onBack,
  onForward,
  right,
  slotClass = 'w-xl',
  overStore = false,
  fold = 0,
}: {
  current: TopTab
  tabs: NavTabSpec[]
  canBack: boolean
  canForward: boolean
  onBack: () => void
  onForward: () => void
  right?: ReactNode
  /** The row's field width. */
  slotClass?: string
  /** The Store's web view would cover a panel. */
  overStore?: boolean
  /** How much of the row has folded away (`useRowFold`). */
  fold?: number
}) {
  // The pill moves on press and holds until the page catches up.
  const [target, setTarget] = useState<TopTab | null>(null)
  useEffect(() => {
    if (!target) return
    if (current === target) return setTarget(null)
    const t = window.setTimeout(() => setTarget(null), 4000)
    return () => window.clearTimeout(t)
  }, [current, target])
  const active = target ?? current

  const track = useRef<HTMLDivElement | null>(null)
  const slots = useRef(new Map<TopTab, HTMLElement>())
  const [pill, setPill] = useState<{ left: number; width: number } | null>(null)
  const measure = useCallback(() => {
    const el = slots.current.get(active)
    if (!el) return setPill(null)
    setPill((p) => (p && p.left === el.offsetLeft && p.width === el.offsetWidth ? p : { left: el.offsetLeft, width: el.offsetWidth }))
  }, [active])
  useLayoutEffect(measure)
  useEffect(() => {
    if (!track.current) return
    // On the next frame, as every observer here does (KB-0251).
    let frame = 0
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(measure)
    })
    ro.observe(track.current)
    slots.current.forEach((el) => ro.observe(el))
    return () => {
      cancelAnimationFrame(frame)
      ro.disconnect()
    }
  }, [measure, tabs.length])

  const iconOnly = fold >= FOLD.labels

  return (
    <nav aria-label="Main" className="no-drag flex min-w-0 items-center gap-2">
      <div className="flex shrink-0 gap-1">
        <IconButton label="Back (Alt+Left)" disabled={!canBack} onClick={onBack}>
          <ArrowLeft className="size-4" />
        </IconButton>
        <IconButton label="Forward (Alt+Right)" disabled={!canForward} onClick={onForward}>
          <ArrowRight className="size-4" />
        </IconButton>
      </div>
      <div ref={track} className="kryo-pill relative flex shrink-0 items-center gap-1 border border-border bg-card p-1">
        {pill ? (
          <span
            aria-hidden
            className="kryo-pill pointer-events-none absolute bottom-1 top-1 bg-primary transition-[left,width] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)]"
            style={{ left: pill.left, width: pill.width }}
          />
        ) : null}
        {tabs.map((t) => (
          <NavTab
            key={t.id}
            tab={t}
            iconOnly={iconOnly}
            noIcon={fold >= FOLD.tabIcons && !iconOnly}
            current={active === t.id}
            panel={!overStore && !iconOnly && t.panel ? t.panel : undefined}
            slot={(el) => {
              if (el) slots.current.set(t.id, el)
              else slots.current.delete(t.id)
            }}
            onPress={() => {
              // The pill follows the press; the press opens the place.
              setTarget(t.id)
              t.onOpen()
            }}
          />
        ))}
      </div>
      {right ? <div className={cn('flex min-w-0 items-center gap-2', slotClass)}>{right}</div> : null}
    </nav>
  )
}

function NavTab({
  tab,
  iconOnly,
  noIcon,
  current,
  panel,
  slot,
  onPress,
}: {
  tab: NavTabSpec
  /** Short of room: the icon alone. */
  iconOnly: boolean
  /** Short of room: the label alone. */
  noIcon: boolean
  current: boolean
  /** Its few pages, when they can be shown here. */
  panel?: NavTabPanel
  slot: (el: HTMLElement | null) => void
  onPress: () => void
}) {
  const TabIcon = TAB_ICONS[tab.id]
  const [open, setOpen] = useState(false)
  const button = useRef<HTMLButtonElement | null>(null)
  const panelEl = useRef<HTMLDivElement | null>(null)
  const timer = useRef<number | null>(null)
  const onTab = useRef(false)
  const inPanel = useRef(false)

  const clear = () => {
    if (timer.current) window.clearTimeout(timer.current)
    timer.current = null
  }
  const show = (delay: number, focusFirst = false) => {
    clear()
    timer.current = window.setTimeout(() => {
      if (!onTab.current && !inPanel.current && !focusFirst) return
      setOpen(true)
      if (focusFirst) requestAnimationFrame(() => panelEl.current?.querySelector<HTMLButtonElement>('button')?.focus())
    }, delay)
  }
  const hideSoon = (delay: number) => {
    clear()
    timer.current = window.setTimeout(() => {
      if (onTab.current || inPanel.current) return
      setOpen(false)
    }, delay)
  }

  useEffect(() => () => clear(), [])
  // Away from it, or the tab went: the panel goes.
  useEffect(() => {
    if (!open) return
    const away = (e: PointerEvent) => {
      if (panelEl.current?.contains(e.target as Node) || button.current?.contains(e.target as Node)) return
      setOpen(false)
    }
    window.addEventListener('pointerdown', away)
    return () => window.removeEventListener('pointerdown', away)
  }, [open])
  useEffect(() => {
    if (!panel) setOpen(false)
  }, [panel])
  // Escape closes it from anywhere.
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      setOpen(false)
      button.current?.focus()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])

  return (
    // Measure the wrapper: offsetLeft is read against the positioned parent, so
    // the button's own box would say 0.
    <div className="relative" ref={slot}>
      <button
        ref={button}
        type="button"
        aria-current={current ? 'page' : undefined}
        aria-haspopup={panel ? 'menu' : undefined}
        aria-expanded={panel ? open : undefined}
        aria-label={iconOnly ? tab.label : undefined}
        title={iconOnly ? tab.label : undefined}
        onPointerEnter={() => {
          onTab.current = true
          if (panel && !open) show(240)
        }}
        onPointerLeave={() => {
          onTab.current = false
          if (panel && open) hideSoon(180)
        }}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown' && panel) {
            e.preventDefault()
            onTab.current = true
            show(0, true)
          } else if (e.key === 'Escape' && open) {
            setOpen(false)
          }
        }}
        onClick={() => {
          clear()
          setOpen(false)
          onPress()
        }}
        className={cn(
          'kryo-pill relative flex h-9 max-w-56 items-center gap-2 text-xs font-bold uppercase tracking-[0.16em] transition-colors duration-300',
          iconOnly ? 'px-3' : 'px-3.5',
          current ? 'text-primary-foreground' : 'text-muted-foreground hover:bg-secondary hover:text-foreground',
        )}
      >
        {noIcon ? null : <TabIcon className="size-3.5 shrink-0" />}
        {iconOnly ? null : <span className="truncate">{tab.label}</span>}
      </button>
      {panel && open ? (
        <div
          ref={panelEl}
          role="menu"
          aria-label={`${tab.label} pages`}
          onPointerEnter={() => {
            inPanel.current = true
            clear()
          }}
          onPointerLeave={() => {
            inPanel.current = false
            hideSoon(180)
          }}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.preventDefault()
              setOpen(false)
              button.current?.focus()
              return
            }
            if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
            e.preventDefault()
            const items = [...(panelEl.current?.querySelectorAll<HTMLButtonElement>('button') ?? [])]
            if (!items.length) return
            const at = items.indexOf(document.activeElement as HTMLButtonElement)
            const step = e.key === 'ArrowDown' ? 1 : -1
            items[(at + step + items.length) % items.length]?.focus()
          }}
          className="kryo-radius absolute left-0 top-[calc(100%+8px)] z-50 w-[19rem] border border-border bg-popover p-1 shadow-2xl shadow-black/40"
        >
          <p className="px-2 pb-1 pt-1.5 text-[9px] uppercase tracking-[0.18em] text-muted-foreground">{panel.heading}</p>
          <div className="grid grid-cols-2 gap-1">
            {panel.items.map((item) => (
              <button
                key={item.label}
                type="button"
                role="menuitem"
                onClick={() => {
                  setOpen(false)
                  item.onSelect()
                }}
                className="kryo-radius flex items-center gap-2 px-2 py-1.5 text-left text-[11px] text-foreground transition-colors hover:bg-secondary focus-visible:bg-secondary"
              >
                {item.icon ? <span className="shrink-0 text-muted-foreground">{item.icon}</span> : null}
                <span className="truncate">{item.label}</span>
              </button>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  )
}
