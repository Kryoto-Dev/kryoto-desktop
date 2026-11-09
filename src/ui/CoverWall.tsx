import { useEffect, useState } from 'react'
import { artSrc } from '@/lib/art'
import { catalogApiUrl } from '@/lib/endpoint'

const KEY = 'kryoto.wall'

/**
 * Columns and covers in one pass of a column: fewer than the home page's
 * fourteen, because this is a 960 by 640 window and not a 4K screen, and every
 * tile is decoded memory.
 */
const COLUMNS = 9
const ROWS = 5
/** Fewer distinct covers than this and there is no wall, as on the site. */
const MIN_COVERS = 12

function remembered(): string[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(KEY) ?? '[]')
    return Array.isArray(value) ? value.filter((c): c is string => typeof c === 'string') : []
  } catch {
    return []
  }
}

/**
 * kryo.to's home page wall (its components/home/landing/backdrop.tsx): the same
 * tilted columns of real covers, neighbours drifting in opposite directions,
 * from the same pool, which holds no adult art (`/api/desktop/wall`).
 *
 * The last covers seen are kept, so it is there at once and offline, and art
 * comes through the client's disk cache. Each column is one CSS animation on
 * the compositor: no JavaScript runs once it has started. Reduced motion holds
 * it still, and with nothing to show the screen is simply plain.
 */
export function CoverWall() {
  const [covers, setCovers] = useState(remembered)

  useEffect(() => {
    let live = true
    void catalogApiUrl('/api/desktop/wall')
      .then((url) => fetch(url, { credentials: 'omit' }))
      .then((res) => (res.ok ? (res.json() as Promise<{ covers?: string[] }>) : null))
      .then((wall) => {
        if (!wall?.covers?.length) return
        try {
          // For the next start: this wall stays as it is, nothing swaps under the reader.
          localStorage.setItem(KEY, JSON.stringify(wall.covers.slice(0, COLUMNS * ROWS)))
        } catch {
          /* not important */
        }
        if (live) setCovers((now) => (now.length ? now : wall.covers!))
      })
      .catch(() => {})
    return () => {
      live = false
    }
  }, [])

  // A first start with no covers yet, or kryo.to out of reach: the wall as a still picture.
  if (covers.length < MIN_COVERS) {
    return <img aria-hidden alt="" src="/brand/wall-web.jpg" className="pointer-events-none absolute inset-0 size-full object-cover" />
  }
  const columns = Array.from({ length: COLUMNS }, (_, col) => Array.from({ length: ROWS }, (_, row) => covers[(col * 5 + row * 3) % covers.length]!))
  return (
    <div aria-hidden className="pointer-events-none absolute inset-0 overflow-hidden">
      <div className="kryo-wall absolute left-1/2 top-1/2 flex gap-3">
        {columns.map((tiles, col) => (
          <div key={col} className="w-48 shrink-0" style={{ marginTop: col % 2 ? '2.5rem' : 0 }}>
            <div className="kryo-wall-track" style={{ animationDuration: `${110 + ((col * 37) % 60)}s`, animationDirection: col % 2 ? 'reverse' : 'normal' }}>
              {[0, 1].map((pass) => (
                <div key={pass} className="flex flex-col gap-3 pb-3">
                  {tiles.map((cover, row) => (
                    <img key={row} src={artSrc(cover) ?? undefined} alt="" decoding="async" draggable={false} width={460} height={215} className="kryo-radius block h-auto w-full border border-border bg-card object-cover" />
                  ))}
                </div>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
