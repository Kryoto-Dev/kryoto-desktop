import type { CSSProperties } from 'react'
import { cn } from '@/lib/utils'

/**
 * Kryoto's two badges, drawn as pixel art on a cell grid (the same cells as
 * the ASCII motion everywhere else) so they read as ours and not as a stock
 * icon set's check.
 *
 * - The team gem: a cut crystal in the reader's accent, with a stepped glint
 *   that crosses it now and then. Team membership only.
 * - The verified shield: a check that draws itself in, cell by cell. A build
 *   somebody ran and found working.
 *
 * Both are still with reduced motion (the system's setting or the site's):
 * `.kryo-px` rules in the stylesheet turn the motion off, and what is left is the
 * finished picture.
 *
 * Art legend: `a` the badge, `h` a light facet, `w` the sparkle, `s` a shaded
 * facet, `d` the gem's girdle, `o` the mark cut into it, `.` empty.
 */

// prettier-ignore
const GEM = [
  '..hhaaass..',
  '.hhwhaasss.',
  'ddddddddddd',
  '.hhaaaasss.',
  '..haaasss..',
  '...aasss...',
  '....ass....',
  '.....s.....',
]

// prettier-ignore
const SHIELD = [
  'aaaaaaaaa',
  'ahhaaaaaa',
  'ahaaaaaoa',
  'aaaaaaoaa',
  'aoaaaoaas',
  'aaoaoaaas',
  '.aaoaaas.',
  '..aaass..',
  '....s....',
]

type Cell = { x: number; y: number; kind: string }

function cells(art: string[]): Cell[] {
  const out: Cell[] = []
  art.forEach((row, y) => [...row].forEach((kind, x) => kind !== '.' && out.push({ x, y, kind })))
  return out
}

/** Cells as one path: a board of a few hundred cards draws one element per layer, not one per cell. */
function cellPath(list: Cell[]): string {
  return list.map((c) => `M${c.x} ${c.y}h1v1h-1z`).join('')
}

const GEM_CELLS = cells(GEM)
const GEM_BODY = cellPath(GEM_CELLS)
const GEM_LIGHT = cellPath(GEM_CELLS.filter((c) => c.kind === 'h'))
const GEM_SHADE = cellPath(GEM_CELLS.filter((c) => c.kind === 's'))
const GEM_SPARK = cellPath(GEM_CELLS.filter((c) => c.kind === 'w'))
const GEM_GIRDLE = cellPath(GEM_CELLS.filter((c) => c.kind === 'd'))
// The glint's diagonals, top left to bottom right.
const GEM_DIAGONALS = [...new Set(GEM_CELLS.map((c) => c.x + c.y))].map((d) => ({ d, path: cellPath(GEM_CELLS.filter((c) => c.x + c.y === d)) }))

const SHIELD_CELLS = cells(SHIELD)
const SHIELD_BODY = cellPath(SHIELD_CELLS.filter((c) => c.kind !== 'o'))
const SHIELD_LIGHT = cellPath(SHIELD_CELLS.filter((c) => c.kind === 'h'))
const SHIELD_SHADE = cellPath(SHIELD_CELLS.filter((c) => c.kind === 's'))
// The check's cells in the order it is drawn: down the short stroke, up the long one.
const CHECK = SHIELD_CELLS.filter((c) => c.kind === 'o').sort((p, q) => p.x - q.x)

/**
 * The Kryoto team gem. `label` is what a screen reader hears and the tooltip
 * says; `decorative` drops both, for a gem inside a chip that names itself.
 */
export function TeamGem({ className, label = 'Kryoto team member', decorative = false }: { className?: string; label?: string; decorative?: boolean }) {
  return (
    <span title={decorative ? undefined : label} className={cn('kryo-px kryo-px-gem inline-flex shrink-0 text-primary', className)}>
      <svg viewBox="0 0 11 8" width="1.2em" height="0.88em" shapeRendering="crispEdges" aria-hidden focusable="false">
        <path d={GEM_BODY} fill="currentColor" />
        <path d={GEM_LIGHT} fill="#fff" opacity={0.5} />
        <path d={GEM_SHADE} fill="#000" opacity={0.3} />
        <path d={GEM_GIRDLE} fill="#000" opacity={0.42} />
        <path d={GEM_SPARK} fill="#fff" opacity={0.9} />
        {/* The glint: one diagonal of cells after another, then nothing for a while. */}
        <g fill="#fff">
          {GEM_DIAGONALS.map(({ d, path }) => (
            <path key={d} d={path} className="kryo-px-glint" style={{ '--d': d } as CSSProperties} />
          ))}
        </g>
      </svg>
      {decorative ? null : <span className="sr-only">{label}</span>}
    </span>
  )
}

/** The verified shield. Decorative beside the word "Verified"; pass `label` when it stands alone. */
export function VerifiedShield({ className, label }: { className?: string; label?: string }) {
  return (
    <span title={label} className={cn('kryo-px kryo-px-shield inline-flex shrink-0 text-success', className)}>
      <svg viewBox="0 0 9 9" width="1em" height="1em" shapeRendering="crispEdges" aria-hidden focusable="false">
        <path d={SHIELD_BODY} fill="currentColor" />
        <path d={SHIELD_LIGHT} fill="#fff" opacity={0.45} />
        <path d={SHIELD_SHADE} fill="#000" opacity={0.3} />
        <g fill="#fff">
          {CHECK.map((c, i) => (
            <rect key={`${c.x}-${c.y}`} x={c.x} y={c.y} width={1} height={1} className="kryo-px-draw" style={{ '--i': i } as CSSProperties} />
          ))}
        </g>
      </svg>
      {label ? <span className="sr-only">{label}</span> : null}
    </span>
  )
}
