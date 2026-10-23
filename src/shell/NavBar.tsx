import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { ArrowLeft, ArrowRight, Library, Store, User, Users, type LucideIcon } from 'lucide-react'
import { IconButton, MenuList, useDismiss, useNativeOpen, type MenuEntry } from '@/ui'
import { isTauri } from '@/lib/bridge'
import { closeMenu, onPopupHover, openMenu, popupOpen } from '@/lib/popup'
import { cn } from '@/lib/utils'

export type TopTab = 'store' | 'library' | 'community' | 'profile'

/** The tab icons: Store, Library and User are the mobile app's; Community has no tab there. */
const TAB_ICONS: Record<TopTab, LucideIcon> = { store: Store, library: Library, community: Users, profile: User }

export type NavTabSpec = {
  id: TopTab
  label: string
  onOpen: () => void
  items: MenuEntry[]
}

/**
 * Back / forward, then STORE · LIBRARY · COMMUNITY · <NAME> as kryo.to's
 * segmented pill - the white segment is where you are. Each tab opens its
 * section on click and shows its pages on hover, after a short pause so a
 * pointer sweeping across does not flash menus. Local pages keep menus in
 * the shell; over the Store they use the native menu view. The right side
 * carries whatever the current page adds: the Store's address, the Library's
 * actions.
 */
export function NavBar({
  current,
  tabs,
  canBack,
  canForward,
  onBack,
  onForward,
  overStore = false,
  right,
}: {
  current: TopTab
  tabs: NavTabSpec[]
  canBack: boolean
  canForward: boolean
  onBack: () => void
  onForward: () => void
  /** Only the visible Store needs menus in a separate native web view. */
  overStore?: boolean
  right?: ReactNode
}) {
  // The pill follows a press at once and stays on it while the page loads, so
  // the selection never lags behind the click; it lets go when the page has
  // caught up (or after a few seconds, if the press went nowhere).
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
    const ro = new ResizeObserver(measure)
    ro.observe(track.current)
    slots.current.forEach((el) => ro.observe(el))
    return () => ro.disconnect()
  }, [measure, tabs.length])

  return (
    <nav aria-label="Main" className="relative z-40 flex h-14 shrink-0 items-center gap-3 border-b border-border bg-background px-3">
      <div className="flex gap-1.5">
        <IconButton label="Back (Alt+Left)" disabled={!canBack} onClick={onBack}>
          <ArrowLeft className="size-4" />
        </IconButton>
        <IconButton label="Forward (Alt+Right)" disabled={!canForward} onClick={onForward}>
          <ArrowRight className="size-4" />
        </IconButton>
      </div>
      <div ref={track} className="kryo-pill relative flex items-center gap-1 border border-border bg-card p-1">
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
            current={active === t.id}
            native={overStore && isTauri()}
            slot={(el) => {
              if (el) slots.current.set(t.id, el)
              else slots.current.delete(t.id)
            }}
            onPress={() => setTarget(t.id)}
          />
        ))}
      </div>
      <div className="flex min-w-0 grow items-center justify-end gap-2">{right}</div>
    </nav>
  )
}

function NavTab({
  tab,
  current,
  native,
  slot,
  onPress,
}: {
  tab: NavTabSpec
  current: boolean
  native: boolean
  slot: (el: HTMLElement | null) => void
  onPress: () => void
}) {
  const TabIcon = TAB_ICONS[tab.id]
  const menuId = `tab:${tab.id}`
  const [domOpen, setDomOpen] = useState(false)
  /** Opened from the keyboard: the first row is lit. Opened by the pointer: nothing is. */
  const [byKeyboard, setByKeyboard] = useState(false)
  const nativeOpen = useNativeOpen(native ? menuId : undefined)
  const open = native ? nativeOpen : domOpen
  const timer = useRef<number | null>(null)
  const onTab = useRef(false)
  const inPopup = useRef(false)
  const button = useRef<HTMLButtonElement | null>(null)
  const wrapper = useRef<HTMLDivElement | null>(null)
  const closeLocal = () => {
    setDomOpen(false)
    if (wrapper.current?.contains(document.activeElement)) button.current?.focus({ preventScroll: true })
  }
  useDismiss(!native && domOpen, wrapper, closeLocal)

  const clear = () => {
    if (timer.current) window.clearTimeout(timer.current)
    timer.current = null
  }
  const show = () => {
    if (!tab.items.length) return
    if (!native) return setDomOpen(true)
    const r = button.current?.getBoundingClientRect()
    if (r) void openMenu(menuId, r, tab.items)
  }
  const hideSoon = (delay: number) => {
    clear()
    timer.current = window.setTimeout(() => {
      if (onTab.current || inPopup.current) return
      if (!native) closeLocal()
      else if (popupOpen() === menuId) closeMenu()
    }, delay)
  }

  // Leaving the menu (not into the tab) closes it, like leaving the tab.
  useEffect(() => {
    if (!native) return
    return onPopupHover((inside) => {
      if (popupOpen() !== menuId) return
      inPopup.current = inside
      if (!inside) hideSoon(220)
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [native, menuId])
  useEffect(() => () => {
    clear()
    if (native && popupOpen() === menuId) closeMenu()
  }, [native, menuId])
  useEffect(() => { if (!open) inPopup.current = false }, [open])
  useEffect(() => { setDomOpen(false) }, [native])

  return (
    <div
      ref={(el) => {
        wrapper.current = el
        slot(el)
      }}
      className="relative"
      onPointerEnter={() => {
        onTab.current = true
        clear()
        if (open) return
        // Moving over from another tab's open menu switches straight away.
        const other = native && popupOpen()?.startsWith('tab:')
        timer.current = window.setTimeout(() => {
          setByKeyboard(false)
          show()
        }, other ? 0 : 280)
      }}
      onPointerLeave={() => {
        onTab.current = false
        // Time to cross the gap into the menu.
        hideSoon(native ? 350 : 160)
      }}
    >
      <button
        ref={button}
        type="button"
        data-menu={native ? menuId : undefined}
        aria-current={current ? 'page' : undefined}
        aria-haspopup={tab.items.length ? 'menu' : undefined}
        aria-expanded={tab.items.length ? open : undefined}
        onKeyDown={(e) => {
          if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
          e.preventDefault()
          clear()
          setByKeyboard(true)
          show()
        }}
        onClick={() => {
          clear()
          if (native) closeMenu()
          else setDomOpen(false)
          onPress()
          tab.onOpen()
        }}
        className={cn(
          'kryo-pill relative flex h-9 max-w-56 items-center gap-2 px-4 text-xs font-bold uppercase tracking-[0.2em] transition-colors duration-300',
          current ? 'text-primary-foreground' : 'text-muted-foreground hover:bg-secondary hover:text-foreground',
        )}
      >
        <TabIcon className="size-3.5 shrink-0" />
        <span className="truncate">{tab.label}</span>
      </button>
      {!native && domOpen ? (
        <div
          role="menu"
          className="absolute left-0 top-full z-[400] min-w-48 pt-2"
        >
          <div className="kryo-pop kryo-radius overflow-hidden border border-border bg-popover py-1 shadow-2xl shadow-black/60">
            <MenuList items={tab.items} onDone={() => setDomOpen(false)} autoFocus={byKeyboard} />
          </div>
        </div>
      ) : null}
    </div>
  )
}
