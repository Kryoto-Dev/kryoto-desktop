import { useEffect, useRef, useState } from 'react'
import { cn } from '@/lib/utils'
import { BRAND_GLYPHS } from '@/ui/ascii/brand-glyphs'

/**
 * kryo.to's ASCII logo, ported from the site's `components/kryo-ascii.tsx` so
 * it plays the same: the same lettering set as text in Geist Mono, the same
 * per-character scramble, the same slots opening and the dot folding away.
 *
 * - `animate`: K// scrambles out to Kryo.to, the dot folds into "Kryoto",
 *   holds, and scrambles back to K//; on a loop unless `loop` is off.
 * - otherwise: K// at rest, and hovering plays the fast version.
 *
 * The site's flag colours are left out on purpose: they are its easter egg.
 */

const SCRAMBLE = '░▒▓█╗╔╝╚═║/\\.:_-=<>+*'

const FROM_SLOTS = ['K', '/', '/', ' ', ' .', ' ', ' ']
const TO_SLOTS = ['K', 'r', 'y', 'o', '.', 't', 'o']

const glyph = (ch: string, row: number) => BRAND_GLYPHS[ch]?.[row] ?? ''

function reducedMotion() {
  return typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches
}

type Cell = { from: string; to: string; start: number; end: number; char: string }

export function KryoAscii({
  className,
  animate = false,
  loop = true,
}: {
  className?: string
  animate?: boolean
  loop?: boolean
}) {
  const [showDot, setShowDot] = useState(false)
  const [hovered, setHovered] = useState(false)
  const [expanded, setExpanded] = useState(false)
  const rowRefs = useRef<(HTMLDivElement | null)[][]>([])
  const hasMounted = useRef(false)

  useEffect(() => {
    let raf = 0
    let timer: number | undefined
    let cancelled = false
    const wait = (ms: number) => new Promise<void>((r) => { timer = window.setTimeout(r, ms) })

    function animateText(source: string[], target: string[], direction: 'forward' | 'backward', fast = false) {
      return new Promise<void>((resolve) => {
        const queues: Cell[][][] = []
        for (let s = 0; s < 7; s++) {
          queues[s] = []
          for (let r = 0; r < 6; r++) {
            const row: Cell[] = []
            queues[s]![r] = row
            // Start from what is on screen, so an interrupted animation carries on smoothly.
            const dom = rowRefs.current[s]?.[r]?.textContent
            const toStr = glyph(target[s]!, r)
            const fromStr = dom && dom.length === toStr.length ? dom : glyph(source[s]!, r)
            for (let c = 0; c < toStr.length; c++) {
              const sweep = (direction === 'forward' ? s : 6 - s) * (fast ? 4 : 10)
              const start = Math.floor(Math.random() * (fast ? 10 : 20)) + sweep
              const end = start + (fast ? 10 : 30) + Math.floor(Math.random() * (fast ? 15 : 30))
              row.push({ from: fromStr[c] ?? ' ', to: toStr[c] ?? ' ', start, end, char: '' })
            }
          }
        }

        let frame = 0
        const run = () => {
          if (cancelled) return resolve()
          let done = true
          for (let s = 0; s < 7; s++) {
            for (let r = 0; r < 6; r++) {
              let out = ''
              for (const q of queues[s]![r]!) {
                if (frame >= q.end) out += q.to
                else if (frame >= q.start) {
                  done = false
                  if (q.from === ' ' && q.to === ' ') out += ' '
                  else {
                    if (!q.char || Math.random() < (fast ? 0.3 : 0.15)) {
                      q.char = SCRAMBLE[Math.floor(Math.random() * SCRAMBLE.length)]!
                    }
                    out += q.char
                  }
                } else {
                  done = false
                  out += q.from
                }
              }
              const el = rowRefs.current[s]?.[r]
              if (el) el.textContent = out
            }
          }
          if (done) resolve()
          else {
            frame++
            raf = requestAnimationFrame(run)
          }
        }
        run()
      })
    }

    const paint = (slots: string[]) =>
      slots.forEach((ch, s) => {
        for (let r = 0; r < 6; r++) {
          const el = rowRefs.current[s]?.[r]
          if (el) el.textContent = glyph(ch, r)
        }
      })

    if (animate && reducedMotion()) {
      // Still: land on the word the animation would rest on.
      setExpanded(true)
      paint(TO_SLOTS)
    } else if (animate) {
      void (async () => {
        await wait(500)
        while (!cancelled) {
          setExpanded(true)
          setShowDot(true)
          await animateText(FROM_SLOTS, TO_SLOTS, 'forward')
          if (cancelled) break
          setShowDot(false)
          if (!loop) break
          await wait(4000)
          if (cancelled) break
          setShowDot(true)
          await wait(400)
          if (cancelled) break
          await animateText(TO_SLOTS, FROM_SLOTS, 'backward')
          if (cancelled) break
          setExpanded(false)
          setShowDot(false)
          await wait(2000)
        }
      })()
    } else if (!hasMounted.current) {
      hasMounted.current = true
      paint(FROM_SLOTS)
    } else {
      void (async () => {
        if (hovered) {
          setExpanded(true)
          setShowDot(true)
          await animateText(FROM_SLOTS, TO_SLOTS, 'forward', true)
          if (!cancelled) setShowDot(false)
        } else {
          setShowDot(true)
          await animateText(TO_SLOTS, FROM_SLOTS, 'backward', true)
          if (!cancelled) {
            setExpanded(false)
            setShowDot(false)
          }
        }
      })()
    }

    return () => {
      cancelled = true
      window.clearTimeout(timer)
      cancelAnimationFrame(raf)
    }
  }, [animate, hovered, loop])

  return (
    <div
      role="img"
      aria-label="Kryoto"
      onMouseEnter={animate ? undefined : () => setHovered(true)}
      onMouseLeave={animate ? undefined : () => setHovered(false)}
      className={cn('kryo-ascii-art relative flex w-fit select-none items-start font-mono leading-none text-foreground', className)}
    >
      {TO_SLOTS.map((_, s) => (
        <div
          key={s}
          className={cn('flex select-none flex-col transition-all duration-500 ease-in-out', s >= 3 && 'overflow-hidden')}
          style={
            s === 4
              ? { width: expanded && showDot ? '3ch' : '0px', opacity: expanded && showDot ? 1 : 0 }
              : s >= 3
                ? { width: expanded ? '8ch' : '0px', opacity: expanded ? 1 : 0 }
                : { width: '8ch' }
          }
        >
          {Array.from({ length: 6 }, (_, r) => (
            <div
              key={r}
              className="kryo-ascii-art flex whitespace-pre font-mono leading-none"
              style={{ minHeight: '1em' }}
              ref={(el) => {
                ;(rowRefs.current[s] ??= [])[r] = el
              }}
            >
              {glyph(FROM_SLOTS[s]!, r)}
            </div>
          ))}
        </div>
      ))}
    </div>
  )
}
