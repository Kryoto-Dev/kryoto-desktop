import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type RefObject,
  type TextareaHTMLAttributes,
} from 'react'
import { Bold, Code, EyeOff, Italic, Lock, Search, Smile, Strikethrough } from '@/ui/pixel-icons'
import { Floating } from '@/ui'
import { openStorePage } from '@/lib/window'
import { cn } from '@/lib/utils'
import { FxSpan } from './RichText'
import { MENTION_LIMITS, mentionCount, rawMention, typedMentions } from './mentions'
import {
  EMOJI,
  EMOJI_GROUPS,
  EMOJI_RANK_GROUPS,
  RANKS,
  RANK_LABEL,
  STICKERS,
  canUseRank,
  emojiFor,
  emojiPath,
  emojiToken,
  isEmojiName,
  searchEmoji,
  stickerPath,
  stickerToken,
  type EmojiEntry,
  type RankId,
} from './rich-catalog'
import { frequentEmoji, noteEmojiUse } from './emoji-usage'

const isRank = (id: string): id is RankId => (RANKS as readonly string[]).includes(id)

/** A set's heading; a locked set says who it is for and opens /donate in the Store. */
function RankHeading({ label, rank, locked }: { label: string; rank: RankId | null; locked: boolean }) {
  return (
    <p className="mb-1 flex items-center gap-1.5 text-[10px] uppercase tracking-wider text-muted-foreground">
      {locked ? <Lock className="size-3" aria-hidden /> : null}
      {label}
      {locked && rank ? (
        <button type="button" onClick={() => openStorePage('/donate')} className="ml-auto normal-case tracking-normal text-primary underline underline-offset-2">
          {RANK_LABEL[rank]}s and up
        </button>
      ) : null}
    </p>
  )
}
import { STANDARD_TEXT_EFFECTS, TEXT_EFFECT_LABEL } from './text-effects'
import { useAssetBase } from './asset-base'

/**
 * The editor for chat (and anything else Desktop lets people write), the
 * counterpart of kryo.to/components/rich-editor.tsx on the same parser:
 *
 *   <RichTextarea value={v} onValueChange={setV} people={find} effects={owned} />
 *     `@` opens the people `find(query)` returns and puts in `@username`;
 *     selected words get the style picker (bold, italic, struck, spoiler,
 *     code, text effects; without the perk the effects are a preview that
 *     links to /perks and is never applied).
 *   <EmojiButton onPick={(token, kind) => ...} />  or with target/value/onValueChange
 *     The animated emoji and stickers, from kryo.to.
 *   storedMentions(text, known)
 *     `@username` of people the chat knows as `<@id>`, for text no server
 *     will read (end-to-end chat), and the ids it mentions.
 */

export type MentionCandidate = { id: string; username: string; displayName: string | null; avatarUrl: string | null }

type TextareaProps = Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, 'value' | 'onChange'> & {
  value: string
  onValueChange: (next: string) => void
  /** People for the `@` list; leave out for no list. */
  people?: (query: string) => Promise<MentionCandidate[]> | MentionCandidate[]
  /** The writer owns the text effects (the perk); otherwise they are a preview. */
  effects?: boolean
  /** The writer's supporter rank, for which rank emoji the `:` list offers. */
  rank?: string | null
  styles?: boolean
  wrapperClassName?: string
}

const TYPING_MENTION = /(^|[^A-Za-z0-9_.\-@/\\])@([A-Za-z0-9_.-]{0,32})$/

/** `:` and at least two letters of a shortcode, right before the cursor. */
const TYPING_SHORTCODE = /(^|[\s([{>"'])(:([a-z0-9_+-]{2,32}))$/i
/** A whole `:shortcode:` just closed, right before the cursor. */
const CLOSED_SHORTCODE = /(^|[\s([{>"'])(:([a-z0-9_+-]{2,32}):)$/i

type ShortcodeModule = typeof import('./emoji-shortcodes')
let shortcodeTable: ShortcodeModule | null = null
let shortcodeLoading: Promise<ShortcodeModule> | null = null
/** The standard emoji list, loaded once somebody types `:`. */
function loadShortcodes(): Promise<ShortcodeModule> {
  shortcodeLoading ??= import('./emoji-shortcodes').then((m) => (shortcodeTable = m))
  return shortcodeLoading
}

/**
 * The text as it is sent: every whole standard `:shortcode:` as its emoji, a
 * pasted one too, like Discord. Kryoto's own emoji stay `:name:`. For text no
 * server sees (end-to-end chat); kryo.to does the same on save.
 */
export async function withShortcodes(text: string): Promise<string> {
  if (!/:[a-z0-9_+-]{2,32}:/i.test(text)) return text
  const table = await loadShortcodes().catch(() => null)
  return table ? table.replaceShortcodes(text, isEmojiName) : text
}

/** One line of the `:` list: our emoji first, then the standard ones. */
type CodeHit = { key: string; insert: string; label: string; img: string | null; char: string | null }

function codeHits(query: string, rank: string | null, table: ShortcodeModule | null): CodeHit[] {
  const q = query.toLowerCase()
  const used = frequentEmoji(40)
  const usage = (name: string) => (used.includes(name) ? used.length - used.indexOf(name) : 0)
  const ours = searchEmoji(q)
    .filter((e) => canUseRank(e.rank, rank))
    .sort((a, b) => Number(b.name.startsWith(q)) - Number(a.name.startsWith(q)) || usage(b.name) - usage(a.name))
    .slice(0, 6)
    .map((e): CodeHit => ({ key: e.name, insert: `:${e.name}: `, label: `:${e.name}:`, img: emojiPath(e.name), char: null }))
  const standard = (table?.searchShortcodes(q, 8) ?? []).map(([code, char]): CodeHit => ({ key: `std:${code}`, insert: `${char} `, label: `:${code}:`, img: null, char }))
  return [...ours, ...standard].slice(0, 8)
}

export const RichTextarea = forwardRef<HTMLTextAreaElement, TextareaProps>(function RichTextarea(
  { value, onValueChange, people, effects = false, rank = null, styles = true, wrapperClassName, maxLength, onKeyDown, onSelect, onBlur, onKeyUp, onMouseUp, ...rest },
  outer,
) {
  const ref = useRef<HTMLTextAreaElement>(null)
  useImperativeHandle(outer, () => ref.current as HTMLTextAreaElement)
  const [mention, setMention] = useState<{ start: number; query: string; top: number; left: number } | null>(null)
  const [found, setFound] = useState<MentionCandidate[]>([])
  const [active, setActive] = useState(0)
  const [selection, setSelection] = useState<{ start: number; end: number; top: number; left: number } | null>(null)
  const [code, setCode] = useState<{ start: number; query: string; top: number; left: number } | null>(null)
  const [hits, setHits] = useState<CodeHit[]>([])
  const base = useAssetBase()

  const sync = useCallback(() => {
    const el = ref.current
    if (!el) return
    const { selectionStart: s, selectionEnd: e } = el
    if (styles && e > s && document.activeElement === el) {
      const at = caretPoint(el, s)
      setSelection({ start: s, end: e, top: at.top, left: Math.min(at.left, Math.max(0, el.clientWidth - 260)) })
    } else setSelection(null)
    const coded = e === s ? TYPING_SHORTCODE.exec(el.value.slice(0, s)) : null
    if (coded) {
      void loadShortcodes()
      const query = coded[3]!
      const start = s - query.length - 1
      const at = caretPoint(el, start)
      setCode((c) => (c && c.query === query && c.start === start ? c : { start, query, top: at.top, left: Math.min(at.left, Math.max(0, el.clientWidth - 240)) }))
    } else setCode(null)
    if (!people || e !== s) {
      setMention(null)
      return
    }
    const typed = TYPING_MENTION.exec(el.value.slice(0, s))
    if (!typed) {
      setMention(null)
      return
    }
    const query = typed[2]!
    const start = s - query.length - 1
    const at = caretPoint(el, start)
    setMention((m) => (m && m.query === query && m.start === start ? m : { start, query, top: at.top, left: Math.min(at.left, Math.max(0, el.clientWidth - 240)) }))
  }, [people, styles])

  const query = mention?.query ?? null
  useEffect(() => {
    if (query === null || !people) {
      setFound([])
      return
    }
    let live = true
    const t = setTimeout(() => {
      void Promise.resolve(people(query))
        .then((list) => {
          if (!live) return
          setFound(list.slice(0, 8))
          setActive(0)
        })
        .catch(() => {})
    }, 100)
    return () => {
      live = false
      clearTimeout(t)
    }
  }, [query, people])

  function put(next: string, a: number, b = a) {
    if (maxLength !== undefined && next.length > Number(maxLength)) return
    onValueChange(next)
    requestAnimationFrame(() => {
      const el = ref.current
      if (!el) return
      el.focus()
      el.setSelectionRange(a, b)
      sync()
    })
  }

  const codeQuery = code?.query ?? null
  useEffect(() => {
    if (codeQuery === null) {
      setHits([])
      return
    }
    let live = true
    setHits(codeHits(codeQuery, rank, shortcodeTable))
    setActive(0)
    if (!shortcodeTable) void loadShortcodes().then((t) => live && setHits(codeHits(codeQuery, rank, t)))
    return () => {
      live = false
    }
  }, [codeQuery, rank])

  function pickCode(hit: CodeHit) {
    if (!code) return
    if (hit.img) noteEmojiUse(hit.key)
    const end = code.start + 1 + code.query.length
    setCode(null)
    setHits([])
    put(value.slice(0, code.start) + hit.insert + value.slice(end).replace(/^ /, ''), code.start + hit.insert.length)
  }

  /** A standard `:shortcode:` typed out in full becomes its emoji; ours stay their `:name:`. */
  function closeShortcode(next: string, caret: number): { next: string; caret: number } {
    if (!shortcodeTable || !next.includes(':')) return { next, caret }
    const closed = CLOSED_SHORTCODE.exec(next.slice(0, caret))
    if (!closed || isEmojiName(closed[3]!)) return { next, caret }
    const char = shortcodeTable.shortcodeChar(closed[3]!.toLowerCase())
    if (!char) return { next, caret }
    const from = caret - closed[2]!.length
    return { next: next.slice(0, from) + char + next.slice(caret), caret: from + char.length }
  }

  function pick(p: MentionCandidate) {
    if (!mention) return
    const end = mention.start + 1 + mention.query.length
    const insert = `@${p.username} `
    setMention(null)
    setFound([])
    put(value.slice(0, mention.start) + insert + value.slice(end).replace(/^ /, ''), mention.start + insert.length)
  }

  function wrap(open: string, close: string) {
    if (!selection) return
    // Spaces at either edge stay outside the marks: `** bold**` would not read as bold.
    let { start, end } = selection
    while (start < end && /\s/.test(value[start]!)) start++
    while (end > start && /\s/.test(value[end - 1]!)) end--
    if (start === end) return
    if (value.slice(start - open.length, start) === open && value.slice(end, end + close.length) === close) {
      put(value.slice(0, start - open.length) + value.slice(start, end) + value.slice(end + close.length), start - open.length, end - open.length)
      return
    }
    const words = value.slice(start, end)
    put(value.slice(0, start) + open + words + close + value.slice(end), start + open.length, start + open.length + words.length)
  }

  function keys(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (code && hits.length) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        setActive((a) => (a + (e.key === 'ArrowDown' ? 1 : hits.length - 1)) % hits.length)
        return
      }
      if ((e.key === 'Enter' && !e.shiftKey) || e.key === 'Tab') {
        e.preventDefault()
        pickCode(hits[active] ?? hits[0]!)
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        setCode(null)
        return
      }
    }
    if (mention && found.length) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        setActive((a) => (a + (e.key === 'ArrowDown' ? 1 : found.length - 1)) % found.length)
        return
      }
      if ((e.key === 'Enter' && !e.shiftKey) || e.key === 'Tab') {
        e.preventDefault()
        pick(found[active] ?? found[0]!)
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        setMention(null)
        return
      }
    }
    if (selection && (e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey) {
      const k = e.key.toLowerCase()
      if (k === 'b' || k === 'i') {
        e.preventDefault()
        wrap(k === 'b' ? '**' : '*', k === 'b' ? '**' : '*')
        return
      }
    }
    onKeyDown?.(e)
  }

  const listId = useRef(`mentions-${Math.random().toString(36).slice(2)}`).current
  /** A point in the box, in the window: what the lists above the words hang from. */
  const caretBox = (at: { top: number; left: number }) => {
    const box = ref.current?.getBoundingClientRect()
    if (!box) return null
    const left = box.left + at.left
    const top = box.top + at.top
    return { left, right: left, top, bottom: top, width: 0 }
  }
  const codesOpen = Boolean(code && hits.length)
  const listOpen = !codesOpen && Boolean(mention && found.length)
  const anyList = codesOpen || listOpen

  return (
    <div className={cn('relative', wrapperClassName)}>
      <textarea
        {...rest}
        ref={ref}
        value={value}
        maxLength={maxLength}
        aria-autocomplete="list"
        aria-controls={anyList ? listId : undefined}
        aria-expanded={anyList}
        aria-activedescendant={anyList ? `${listId}-${active}` : undefined}
        onChange={(e) => {
          const done = closeShortcode(e.target.value, e.target.selectionStart ?? e.target.value.length)
          if (done.next !== e.target.value) {
            put(done.next, done.caret)
            return
          }
          onValueChange(e.target.value)
          requestAnimationFrame(sync)
        }}
        onKeyDown={keys}
        onSelect={(e) => {
          sync()
          onSelect?.(e)
        }}
        onKeyUp={(e) => {
          sync()
          onKeyUp?.(e)
        }}
        onMouseUp={(e) => {
          sync()
          onMouseUp?.(e)
        }}
        onBlur={(e) => {
          setMention(null)
          setSelection(null)
          onBlur?.(e)
        }}
      />
      {listOpen ? (
        // Opens upward: the chat box sits at the bottom of the window.
        <Floating
          id={listId}
          role="listbox"
          aria-label="People to mention"
          anchor={() => caretBox(mention!)}
          side="top"
          gap={4}
          className="kryo-radius w-60 overflow-y-auto overflow-x-hidden border border-border bg-popover py-1 shadow-lg"
        >
          {found.map((p, i) => (
            <div
              key={p.id}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === active}
              onPointerDown={(e) => e.preventDefault()}
              onClick={() => pick(p)}
              onMouseEnter={() => setActive(i)}
              className={cn('flex cursor-pointer items-center gap-2 px-2 py-1.5 text-sm', i === active ? 'bg-secondary text-foreground' : 'text-muted-foreground')}
            >
              <span className="grid size-5 shrink-0 place-items-center overflow-hidden rounded-full border border-border bg-secondary text-[9px] font-bold uppercase">
                {p.avatarUrl ? <img src={p.avatarUrl} alt="" className="size-full object-cover" /> : (p.displayName || p.username).slice(0, 1)}
              </span>
              <span className="min-w-0 flex-1 truncate">
                <span className="font-bold text-foreground">{p.displayName || p.username}</span>
                {p.displayName ? <span className="ml-1.5 text-xs text-muted-foreground">@{p.username}</span> : null}
              </span>
            </div>
          ))}
        </Floating>
      ) : null}
      {codesOpen ? (
        <Floating
          id={listId}
          role="listbox"
          aria-label="Emoji"
          anchor={() => caretBox(code!)}
          side="top"
          gap={4}
          className="kryo-radius w-60 overflow-y-auto overflow-x-hidden border border-border bg-popover py-1 shadow-lg"
        >
          {hits.map((h, i) => (
            <div
              key={h.key}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === active}
              onPointerDown={(e) => e.preventDefault()}
              onClick={() => pickCode(h)}
              onMouseEnter={() => setActive(i)}
              className={cn('flex cursor-pointer items-center gap-2 px-2 py-1 text-sm', i === active ? 'bg-secondary text-foreground' : 'text-muted-foreground')}
            >
              {h.img ? (
                <img src={`${base}${h.img}`} alt="" width={22} height={22} className="size-[22px]" />
              ) : (
                <span className="inline-flex size-[22px] items-center justify-center text-lg leading-none" aria-hidden>
                  {h.char}
                </span>
              )}
              <span className="min-w-0 flex-1 truncate font-mono text-xs">{h.label}</span>
            </div>
          ))}
        </Floating>
      ) : null}
      {selection && !anyList ? <StylePicker at={() => caretBox(selection)} owned={effects} onWrap={wrap} /> : null}
      {/* kryo.to holds the limit whatever this says (lib/mentions-db.ts); this only warns. */}
      {people && mentionCount(value) > MENTION_LIMITS.member.perMessage ? (
        <p role="status" className="mt-1 text-[11px] leading-relaxed text-warning">
          Members can mention up to {MENTION_LIMITS.member.perMessage} people in one message. Anyone past that is not told.
        </p>
      ) : null}
    </div>
  )
})

const MARKS = [
  { label: 'Bold', open: '**', close: '**', Icon: Bold, keys: 'Ctrl+B' },
  { label: 'Italic', open: '*', close: '*', Icon: Italic, keys: 'Ctrl+I' },
  { label: 'Strikethrough', open: '~~', close: '~~', Icon: Strikethrough },
  { label: 'Spoiler', open: '||', close: '||', Icon: EyeOff },
  { label: 'Code', open: '`', close: '`', Icon: Code },
] as const

const PICKER_BUTTON =
  'inline-flex size-7 items-center justify-center text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring'

function StylePicker({ at, owned, onWrap }: { at: () => { left: number; right: number; top: number; bottom: number; width: number } | null; owned: boolean; onWrap: (open: string, close: string) => void }) {
  const [teaser, setTeaser] = useState(false)
  return (
    <Floating
      role="toolbar"
      aria-label="Style the selected words"
      onPointerDown={(e) => e.preventDefault()}
      anchor={at}
      side="top"
      className="kryo-radius w-64 overflow-y-auto border border-border bg-popover p-1 shadow-lg"
    >
      <div className="flex items-center">
        {MARKS.map(({ label, open, close, Icon, ...m }) => (
          <button key={label} type="button" title={'keys' in m ? `${label} (${m.keys})` : label} onClick={() => onWrap(open, close)} className={PICKER_BUTTON}>
            <Icon className="size-3.5" aria-hidden />
            <span className="sr-only">{label}</span>
          </button>
        ))}
      </div>
      <div className="mt-1 grid grid-cols-3 gap-px border-t border-border pt-1">
        {STANDARD_TEXT_EFFECTS.map((fx) => (
          <button
            key={fx}
            type="button"
            aria-disabled={!owned}
            onClick={() => (owned ? onWrap(`{${fx}}`, `{/${fx}}`) : setTeaser(true))}
            className={cn('flex items-center gap-1 px-1.5 py-1 text-left text-xs font-bold transition-colors hover:bg-secondary focus-visible:bg-secondary focus-visible:outline-none', !owned && 'opacity-70')}
          >
            {!owned ? <Lock className="size-2.5 shrink-0 text-muted-foreground" aria-hidden /> : null}
            <FxSpan fx={fx}>{TEXT_EFFECT_LABEL[fx]}</FxSpan>
          </button>
        ))}
      </div>
      {!owned && teaser ? (
        <p className="border-t border-border px-1.5 pb-1 pt-1.5 text-[11px] leading-snug text-muted-foreground">
          Text effects come with the Text effects perk, for Backers and up or bought with Kryos.{' '}
          <button type="button" onClick={() => openStorePage('/perks')} className="font-bold text-foreground underline underline-offset-2 hover:text-primary">
            See perks
          </button>
        </p>
      ) : null}
    </Floating>
  )
}

function caretPoint(el: HTMLTextAreaElement, pos: number): { top: number; left: number; height: number } {
  const style = getComputedStyle(el)
  const copy = document.createElement('div')
  for (const prop of MIRRORED) copy.style.setProperty(prop, style.getPropertyValue(prop))
  copy.style.position = 'absolute'
  copy.style.visibility = 'hidden'
  copy.style.top = '0'
  copy.style.left = '-9999px'
  copy.style.whiteSpace = 'pre-wrap'
  copy.style.overflowWrap = 'break-word'
  copy.style.width = `${el.offsetWidth}px`
  copy.textContent = el.value.slice(0, pos)
  const mark = document.createElement('span')
  mark.textContent = el.value.slice(pos, pos + 1) || '.'
  copy.appendChild(mark)
  document.body.appendChild(copy)
  const height = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.4
  const point = { top: mark.offsetTop - el.scrollTop, left: mark.offsetLeft - el.scrollLeft, height }
  copy.remove()
  return point
}

const MIRRORED = [
  'box-sizing',
  'border-top-width',
  'border-right-width',
  'border-bottom-width',
  'border-left-width',
  'border-style',
  'padding-top',
  'padding-right',
  'padding-bottom',
  'padding-left',
  'font-family',
  'font-size',
  'font-weight',
  'font-style',
  'letter-spacing',
  'line-height',
  'text-transform',
  'word-spacing',
  'tab-size',
]

/**
 * `@username` of people the chat knows as stored mentions (`<@id>`), and who
 * they are: what end-to-end chat sends, since no server reads it to do so.
 */
export function storedMentions(text: string, known: MentionCandidate[]): { text: string; ids: string[] } {
  const byName = new Map(known.map((p) => [p.username.toLowerCase(), p.id]))
  let out = text
  const ids: string[] = []
  for (const m of typedMentions(text).reverse()) {
    const id = byName.get(m.username.toLowerCase())
    if (!id) continue
    out = out.slice(0, m.at) + rawMention(id) + out.slice(m.at + m.length)
    if (!ids.includes(id)) ids.unshift(id)
  }
  return { text: out, ids }
}

/** Stored mentions back to `@username` for editing, for people the chat knows. */
export function editableMentions(text: string, known: MentionCandidate[]): string {
  const byId = new Map(known.map((p) => [p.id, p.username]))
  return text.replace(/<@(\d{1,20})>/g, (whole, id: string) => {
    const name = byId.get(id)
    return name ? `@${name}` : whole
  })
}

/** The emoji and sticker picker beside a text box. */
export function EmojiButton({
  target,
  value,
  onValueChange,
  stickers = true,
  onPick,
  className,
  align = 'right',
  rank = null,
}: {
  target?: RefObject<HTMLTextAreaElement | HTMLInputElement | null>
  value?: string
  onValueChange?: (next: string) => void
  stickers?: boolean
  onPick?: (token: string, kind: 'emoji' | 'sticker') => void
  className?: string
  align?: 'left' | 'right'
  /** The writer's supporter rank: the rank sets it covers are offered too. */
  rank?: string | null
}) {
  const [open, setOpen] = useState(false)
  const [tab, setTab] = useState<'emoji' | 'sticker'>('emoji')
  const [q, setQ] = useState('')
  const box = useRef<HTMLDivElement>(null)
  const base = useAssetBase()

  useEffect(() => {
    if (!open) return
    const close = (e: PointerEvent) => {
      if (!box.current?.contains(e.target as Node) && !(e.target as Element).closest?.('[data-kryo-float]')) setOpen(false)
    }
    const esc = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('pointerdown', close)
    document.addEventListener('keydown', esc)
    return () => {
      document.removeEventListener('pointerdown', close)
      document.removeEventListener('keydown', esc)
    }
  }, [open])

  function insert(token: string, kind: 'emoji' | 'sticker') {
    if (kind === 'emoji') noteEmojiUse(token.slice(1, -1))
    if (onPick && (kind === 'sticker' || value === undefined)) {
      onPick(token, kind)
      if (kind === 'sticker') setOpen(false)
      return
    }
    if (value === undefined || !onValueChange) return
    const el = target?.current
    const start = el?.selectionStart ?? value.length
    const end = el?.selectionEnd ?? value.length
    const before = value.slice(0, start)
    const piece = `${before && !/\s$/.test(before) ? ' ' : ''}${token} `
    onValueChange(before + piece + value.slice(end).replace(/^ /, ''))
    requestAnimationFrame(() => {
      if (!el) return
      el.focus()
      el.setSelectionRange(start + piece.length, start + piece.length)
    })
  }

  const list = searchEmoji(q).filter((e) => canUseRank(e.rank, rank))
  // Every rank's set shows; the ones above yours are locked, with where to get them.
  const stickerSets: (RankId | null)[] = [null, ...RANKS]
  // The ones picked most on this device, read each time the panel opens (./emoji-usage.ts).
  const frequent = useMemo(
    () => (open ? frequentEmoji().map((n) => emojiFor(n)).filter((e): e is EmojiEntry => !!e && canUseRank(e.rank, rank)) : []),
    [open, rank],
  )
  const groups: { id: string; label: string }[] = [...(frequent.length ? [{ id: 'frequent', label: 'Frequently used' }] : []), ...EMOJI_GROUPS, ...EMOJI_RANK_GROUPS]
  return (
    <div ref={box} className={cn('relative', className)}>
      <button
        type="button"
        aria-expanded={open}
        aria-haspopup="dialog"
        title={stickers ? 'Emoji and stickers' : 'Emoji'}
        onClick={() => setOpen((o) => !o)}
        className="inline-flex size-9 items-center justify-center text-muted-foreground transition-colors hover:text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
      >
        <Smile className="size-4" aria-hidden />
        <span className="sr-only">{stickers ? 'Emoji and stickers' : 'Emoji'}</span>
      </button>
      {open ? (
        <Floating
          role="dialog"
          aria-label={stickers ? 'Emoji and stickers' : 'Emoji'}
          anchor={() => box.current?.getBoundingClientRect()}
          side="top"
          align={align}
          gap={4}
          className="kryo-radius w-72 overflow-y-auto border border-border bg-popover shadow-lg"
        >
          {stickers ? (
            <div role="tablist" className="flex border-b border-border text-xs font-bold uppercase tracking-wider">
              {(['emoji', 'sticker'] as const).map((t) => (
                <button
                  key={t}
                  type="button"
                  role="tab"
                  aria-selected={tab === t}
                  onClick={() => setTab(t)}
                  className={cn('flex-1 px-3 py-2 transition-colors', tab === t ? 'bg-secondary text-foreground' : 'text-muted-foreground hover:text-foreground')}
                >
                  {t === 'emoji' ? 'Emoji' : 'Stickers'}
                </button>
              ))}
            </div>
          ) : null}
          {tab === 'emoji' || !stickers ? (
            <div className="p-2">
              <label className="kryo-radius mb-2 flex items-center gap-1.5 border border-border bg-background px-2 py-1">
                <Search className="size-3 text-muted-foreground" aria-hidden />
                <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Find an emoji" aria-label="Find an emoji" className="min-w-0 flex-1 bg-transparent text-xs text-foreground outline-none" />
              </label>
              <div className="max-h-56 overflow-y-auto">
                {(q ? [{ id: 'found', label: 'Found' }] : groups).map((g) => {
                  const setRank = isRank(g.id) ? g.id : null
                  const locked = !q && !canUseRank(setRank, rank)
                  const items = q ? list : g.id === 'frequent' ? frequent : locked ? EMOJI.filter((e) => e.group === g.id) : list.filter((e) => e.group === g.id)
                  if (!items.length) return null
                  return (
                    <div key={g.id} className="mb-1.5">
                      <RankHeading label={g.label} rank={setRank} locked={locked} />
                      <div className="grid grid-cols-8 gap-0.5">
                        {items.map((e) => (
                          <button
                            key={e.name}
                            type="button"
                            title={locked && setRank ? `For ${RANK_LABEL[setRank]}s and up` : `:${e.name}:`}
                            aria-disabled={locked || undefined}
                            onPointerDown={(ev) => ev.preventDefault()}
                            onClick={() => (locked ? undefined : insert(emojiToken(e.name), 'emoji'))}
                            className={cn(
                              'kryo-radius flex aspect-square items-center justify-center focus-visible:bg-secondary focus-visible:outline-none',
                              locked ? 'cursor-not-allowed opacity-40 grayscale' : 'hover:bg-secondary',
                            )}
                          >
                            <img src={`${base}${emojiPath(e.name)}`} alt={`:${e.name}:`} width={26} height={26} loading="lazy" decoding="async" className="size-[26px]" />
                          </button>
                        ))}
                      </div>
                    </div>
                  )
                })}
                {q && !list.length ? <p className="py-4 text-center text-xs text-muted-foreground">No emoji called that.</p> : null}
              </div>
            </div>
          ) : (
            <div className="max-h-64 overflow-y-auto p-2">
              {stickerSets.map((set) => {
                const locked = !canUseRank(set, rank)
                return (
                  <div key={set ?? 'base'} className="mb-1.5">
                    {set ? <RankHeading label={`${RANK_LABEL[set]} set`} rank={set} locked={locked} /> : null}
                    <div className="grid grid-cols-3 gap-1">
                      {STICKERS.filter((s) => (s.rank ?? null) === set).map((s) => (
                        <button
                          key={s.id}
                          type="button"
                          title={locked && set ? `For ${RANK_LABEL[set]}s and up` : s.label}
                          aria-disabled={locked || undefined}
                          onPointerDown={(ev) => ev.preventDefault()}
                          onClick={() => (locked ? undefined : insert(stickerToken(s.id), 'sticker'))}
                          className={cn(
                            'kryo-radius flex aspect-square items-center justify-center focus-visible:bg-secondary focus-visible:outline-none',
                            locked ? 'cursor-not-allowed opacity-40 grayscale' : 'hover:bg-secondary',
                          )}
                        >
                          <img src={`${base}${stickerPath(s.id) ?? ''}`} alt={s.label} width={76} height={76} loading="lazy" decoding="async" className="size-[76px]" />
                        </button>
                      ))}
                    </div>
                  </div>
                )
              })}
            </div>
          )}
        </Floating>
      ) : null}
    </div>
  )
}
