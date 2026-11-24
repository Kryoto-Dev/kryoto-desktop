import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { artSrc } from '@/lib/art'
import { catalogApiUrl } from '@/lib/endpoint'
import { cn } from '@/lib/utils'
import { Search, X } from '@/ui/pixel-icons'

/** A game already on this PC (or in the account's kryo.to library). */
export type SearchGame = { id: string; title: string; cover: string | null; onOpen: () => void }

/** A place the search can take you: a Settings section, a page, a dialog. */
export type SearchAction = {
  id: string
  label: string
  /** 'Settings', 'Pages': the heading it sits under. */
  group: string
  /** Words that also find it: "theme" for Appearance. */
  keywords?: string
  hint?: string
  onSelect: () => void
}

/** What kryo.to's own search answers with (as `AddGameDialog` reads it). */
type Hit = { slug: string; title: string; developer: string | null; year: number | null; cover: string | null; nsfw: boolean }

type Row = { key: string; label: string; group: string; hint?: string; cover?: string | null; run: () => void }

const RECENT_KEY = 'kryoto.search.recent'
const RECENT_MAX = 6
/** Rows one group may show. */
const GROUP_MAX = 5

function loadRecent(): string[] {
  try {
    const raw = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]') as unknown
    return Array.isArray(raw) ? raw.filter((v): v is string => typeof v === 'string').slice(0, RECENT_MAX) : []
  } catch {
    return []
  }
}

function saveRecent(query: string) {
  const q = query.trim()
  if (q.length < 2) return
  const next = [q, ...loadRecent().filter((v) => v.toLowerCase() !== q.toLowerCase())].slice(0, RECENT_MAX)
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(next))
  } catch {
    /* not important */
  }
}

/** "themes" is "theme"; "Saves" is "save". Both sides of a match get this. */
function singular(word: string): string {
  return word.length > 3 && word.endsWith('s') && !word.endsWith('ss') ? word.slice(0, -1) : word
}

/** Words too common to mean a place: "a", "the", "for". */
const STOP = new Set(['a', 'an', 'the', 'of', 'for', 'and', 'to', 'in', 'on', 'at', 'my', 'your'])

function words(text: string): string[] {
  return text
    .toLowerCase()
    .split(/[\s&:/.(),-]+/)
    .map(singular)
    .filter((w) => w.length > 1 && !STOP.has(w))
}

/** How many letters apart two words are, given up early past `max`. */
function apart(a: string, b: string, max: number): number {
  if (Math.abs(a.length - b.length) > max) return max + 1
  let prev = Array.from({ length: b.length + 1 }, (_, j) => j)
  for (let i = 1; i <= a.length; i++) {
    const cur = [i]
    let rowMin = i
    for (let j = 1; j <= b.length; j++) {
      const step = a[i - 1] === b[j - 1] ? 0 : 1
      cur[j] = Math.min(prev[j]! + 1, cur[j - 1]! + 1, prev[j - 1]! + step)
      rowMin = Math.min(rowMin, cur[j]!)
    }
    if (rowMin > max) return max + 1
    prev = cur
  }
  return prev[b.length]!
}

/** Letters that may be mistyped in a word this long. */
const allowance = (word: string) => (word.length >= 8 ? 3 : word.length >= 6 ? 2 : word.length >= 4 ? 1 : 0)

/** 0 exact, 1 a start, 2+ a miss - the closer the spelling, the lower. */
function fit(typed: string, word: string): number | undefined {
  if (typed === word) return 0
  if (typed.length < 2 || word.length < 2) return undefined
  if (word.startsWith(typed) || typed.startsWith(word)) return 1
  const slack = allowance(typed)
  if (!slack) return undefined
  const missed = apart(typed, word, slack)
  return missed <= slack ? 2 + missed : undefined
}

/** Lower is closer; undefined is no match. A typed name beats its keywords. */
function score(label: string, keywords: string | undefined, q: string): number | undefined {
  const name = label.toLowerCase()
  if (name === q) return 0
  if (name.startsWith(q)) return 1
  if (name.includes(q)) return 2
  const typed = q.split(/\s+/).filter(Boolean).map(singular)
  let best: number | undefined
  const consider = (text: string | undefined, penalty: number) => {
    if (!text) return
    for (const word of words(text)) {
      for (const t of typed) {
        const at = fit(t, word)
        if (at === undefined) continue
        const scored = at + penalty
        if (best === undefined || scored < best) best = scored
      }
    }
  }
  // Its own words first, then the keywords, so a typed name still wins.
  consider(label, 0)
  consider(keywords, 2)
  return best
}

/** The typed letters, bold, wherever they are in a name. */
function lit(label: string, q: string): ReactNode {
  if (!q) return label
  const at = label.toLowerCase().indexOf(q)
  if (at < 0) return label
  return (
    <>
      {label.slice(0, at)}
      <span className="font-bold">{label.slice(at, at + q.length)}</span>
      {label.slice(at + q.length)}
    </>
  )
}

/**
 * The row's search: games, Settings and pages, grouped and ranked, openable
 * from anywhere with `/` or Ctrl+K.
 */
export function RowSearch({
  games,
  actions,
  onOpenSlug,
  onOpenChange,
}: {
  games: SearchGame[]
  actions: SearchAction[]
  onOpenSlug: (slug: string) => void
  /** The shell moves the Store out from under the findings. */
  onOpenChange?: (open: boolean) => void
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [hits, setHits] = useState<Hit[]>([])
  const [busy, setBusy] = useState(false)
  const [active, setActive] = useState(0)
  const [recent, setRecent] = useState<string[]>(loadRecent)
  const field = useRef<HTMLInputElement | null>(null)
  const trigger = useRef<HTMLButtonElement | null>(null)
  const wrap = useRef<HTMLDivElement | null>(null)
  const asked = useRef(0)

  const show = useCallback(() => {
    setOpen(true)
    setActive(0)
    onOpenChange?.(true)
    requestAnimationFrame(() => field.current?.focus())
  }, [onOpenChange])

  const hide = useCallback(
    (refocus = true) => {
      setOpen(false)
      setQuery('')
      setHits([])
      setActive(0)
      setRecent(loadRecent())
      onOpenChange?.(false)
      if (refocus) trigger.current?.focus()
    },
    [onOpenChange],
  )

  // `/` or Ctrl+K, from anywhere in the client's pages.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null
      const typing = !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.isContentEditable)
      const slash = e.key === '/' && !typing && !e.ctrlKey && !e.metaKey && !e.altKey
      // Ctrl+L too: this field took the address bar's place.
      const chord = (e.ctrlKey || e.metaKey) && !e.altKey && ['k', 'l'].includes(e.key.toLowerCase())
      if (!slash && !chord) return
      e.preventDefault()
      show()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [show])

  // A press outside closes it.
  useEffect(() => {
    if (!open) return
    const away = (e: PointerEvent) => {
      if (!wrap.current?.contains(e.target as Node)) hide(false)
    }
    window.addEventListener('pointerdown', away)
    return () => window.removeEventListener('pointerdown', away)
  }, [open, hide])

  // kryo.to's answers, asked once the typing stops.
  const q = query.trim().toLowerCase()
  useEffect(() => {
    if (!open || q.length < 2) {
      setHits([])
      setBusy(false)
      return
    }
    const mine = ++asked.current
    setBusy(true)
    const timer = window.setTimeout(() => {
      void catalogSearch(q)
        .then((found) => {
          if (asked.current === mine) setHits(found)
        })
        .catch(() => {
          if (asked.current === mine) setHits([])
        })
        .finally(() => {
          if (asked.current === mine) setBusy(false)
        })
    }, 200)
    return () => window.clearTimeout(timer)
  }, [open, q])

  const rows = useMemo<Row[]>(() => {
    if (!open) return []

    if (!q) {
      const remembered = recent.map<Row>((text) => ({
        key: `recent:${text}`,
        label: text,
        group: 'Recent',
        hint: 'again',
        run: () => {
          setQuery(text)
          requestAnimationFrame(() => field.current?.focus())
        },
      }))
      const playable = games.slice(0, 4).map<Row>((g) => ({
        key: `game:${g.id}`,
        label: g.title,
        group: 'Games',
        hint: 'On this PC',
        cover: g.cover,
        run: g.onOpen,
      }))
      const known = actions.filter((a) => a.group === 'Settings').slice(0, 3).map<Row>((a) => ({
        key: a.id,
        label: a.label,
        group: 'Settings',
        hint: a.hint,
        run: a.onSelect,
      }))
      return [...remembered, ...playable, ...known]
    }

    const mine = games
      .map((g) => ({ g, at: score(g.title, undefined, q) }))
      .filter((x): x is { g: SearchGame; at: number } => x.at !== undefined)
      .sort((a, b) => a.at - b.at || a.g.title.localeCompare(b.g.title))
      .slice(0, GROUP_MAX)
      .map<Row>(({ g }) => ({ key: `game:${g.id}`, label: g.title, group: 'Games', hint: 'On this PC', cover: g.cover, run: g.onOpen }))

    const here = new Set(mine.map((r) => r.label.toLowerCase()))
    const catalog = hits
      .filter((h) => !here.has(h.title.toLowerCase()))
      .slice(0, GROUP_MAX)
      .map<Row>((h) => ({
        key: `hit:${h.slug}`,
        label: h.title,
        group: 'kryo.to',
        hint: [h.developer, h.year ? String(h.year) : null].filter(Boolean).join(' · ') || 'catalogue',
        cover: h.cover,
        run: () => onOpenSlug(h.slug),
      }))

    const scored = actions
      .map((a) => ({ a, at: score(a.label, a.keywords, q) }))
      .filter((x): x is { a: SearchAction; at: number } => x.at !== undefined)
      .sort((x, y) => x.at - y.at)
    // A loose spelling is a guess: show it only when nothing fits better.
    const found = scored.length && scored[0]!.at <= 2 ? scored.filter((x) => x.at <= 3) : scored

    const group = (name: string) =>
      found
        .filter(({ a }) => a.group === name)
        .slice(0, GROUP_MAX)
        .map<Row>(({ a }) => ({ key: a.id, label: a.label, group: a.group, hint: a.hint, run: a.onSelect }))

    // Settings before the catalogue: it answers late, so it cannot move rows
    // under the pointer.
    return [...mine, ...group('Settings'), ...group('Pages'), ...catalog]
  }, [open, q, recent, games, hits, actions, onOpenSlug])

  // Keep the lit row in view.
  useEffect(() => {
    if (!open) return
    const el = rows[active] ? document.getElementById(`row-search-${rows[active].key}`) : null
    el?.scrollIntoView({ block: 'nearest' })
  }, [active, open, rows])

  const take = (row: Row) => {
    if (row.group !== 'Recent') saveRecent(q || row.label)
    hide()
    row.run()
  }

  const activeRow = rows[active]

  return (
    <div ref={wrap} className="relative flex min-w-0 items-center">
      <div
        className={cn(
          'kryo-pill flex h-9 items-center overflow-hidden border bg-card transition-[width,border-color] duration-200 ease-[cubic-bezier(0.32,0.72,0,1)]',
          open ? 'w-[23rem] border-foreground/60' : 'w-9 border-border hover:border-foreground',
        )}
      >
        <button
          ref={trigger}
          type="button"
          aria-label="Search"
          title="Search everything (Ctrl K or /)"
          aria-expanded={open}
          onClick={() => (open ? hide() : show())}
          className={cn('grid size-9 shrink-0 place-items-center text-muted-foreground transition-colors hover:text-foreground', open && 'text-foreground')}
        >
          <Search className="size-3.5" />
        </button>
        {open ? (
          <>
            <input
              ref={field}
              role="combobox"
              aria-expanded={rows.length > 0}
              aria-controls="row-search-results"
              aria-activedescendant={activeRow ? `row-search-${activeRow.key}` : undefined}
              aria-autocomplete="list"
              aria-label="Search games, settings and pages"
              spellCheck={false}
              placeholder="Games, settings, pages"
              value={query}
              onChange={(e) => {
                setQuery(e.target.value)
                setActive(0)
              }}
              onKeyDown={(e) => {
                if (e.key === 'Escape') {
                  e.preventDefault()
                  hide()
                } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                  e.preventDefault()
                  if (!rows.length) return
                  const step = e.key === 'ArrowDown' ? 1 : -1
                  setActive((i) => (i + step + rows.length) % rows.length)
                } else if (e.key === 'Enter') {
                  const row = rows[active]
                  if (row) {
                    e.preventDefault()
                    take(row)
                  }
                }
              }}
              className="kryo-square min-w-0 grow bg-transparent pr-2 text-[11px] text-foreground outline-none placeholder:text-muted-foreground"
            />
            {query ? (
              <button
                type="button"
                aria-label="Clear"
                onClick={() => {
                  setQuery('')
                  field.current?.focus()
                }}
                className="grid size-7 shrink-0 place-items-center text-muted-foreground hover:text-foreground"
              >
                <X className="size-3.5" />
              </button>
            ) : null}
          </>
        ) : null}
      </div>

      {open ? (
        <div
          id="row-search-results"
          role="listbox"
          aria-label="Search results"
          className="kryo-radius absolute left-0 top-[calc(100%+8px)] z-50 w-[23rem] overflow-hidden border border-border bg-popover p-1 shadow-2xl shadow-black/40"
        >
          <div className="max-h-[21rem] overflow-y-auto">
            {rows.length ? (
              rows.map((row, i) => {
                const heading = i === 0 || rows[i - 1]?.group !== row.group
                return (
                  <div key={row.key}>
                    {heading ? (
                      <p className="px-2 pb-1 pt-2 text-[9px] uppercase tracking-[0.18em] text-muted-foreground">
                        {row.group}
                        {row.group === 'kryo.to' && busy ? <span className="ml-2 normal-case tracking-normal">searching…</span> : null}
                      </p>
                    ) : null}
                    <button
                      id={`row-search-${row.key}`}
                      type="button"
                      role="option"
                      aria-selected={i === active}
                      onPointerEnter={() => setActive(i)}
                      // Before blur, or closing the field swallows the press.
                      onMouseDown={(e) => {
                        e.preventDefault()
                        take(row)
                      }}
                      className={cn(
                        'kryo-radius flex w-full items-center gap-2.5 px-2 py-1.5 text-left text-xs transition-colors',
                        i === active ? 'bg-primary text-primary-foreground' : 'text-foreground hover:bg-secondary',
                      )}
                    >
                      {row.cover ? (
                        <img src={artSrc(row.cover) ?? undefined} alt="" className="kryo-radius h-8 w-6 shrink-0 object-cover" decoding="async" />
                      ) : null}
                      <span className="min-w-0 grow truncate">{lit(row.label, q)}</span>
                      {row.hint ? (
                        <span
                          className={cn(
                            'shrink-0 text-[9px] uppercase tracking-wider',
                            i === active ? 'text-primary-foreground/70' : 'text-muted-foreground',
                          )}
                        >
                          {row.hint}
                        </span>
                      ) : null}
                    </button>
                  </div>
                )
              })
            ) : (
              <p className="px-2 py-3 text-[11px] text-muted-foreground">
                {q === '' ? 'Type to search games, settings and pages.' : busy ? 'Looking through kryo.to…' : `Nothing for “${q}”.`}
              </p>
            )}
          </div>
          <p className="mt-1 flex items-center gap-3 border-t border-border px-2 pb-0.5 pt-1.5 text-[9px] uppercase tracking-wider text-muted-foreground">
            <span>↑↓ move</span>
            <span>Enter open</span>
            <span>Esc close</span>
          </p>
        </div>
      ) : null}
    </div>
  )
}

/** kryo.to's catalogue search, as the Add a game dialog asks it. */
async function catalogSearch(q: string): Promise<Hit[]> {
  const res = await fetch(await catalogApiUrl(`/api/games/search?q=${encodeURIComponent(q)}&limit=6`))
  if (!res.ok) throw new Error('Search is unavailable right now.')
  return ((await res.json()) as { results?: Hit[] }).results ?? []
}
