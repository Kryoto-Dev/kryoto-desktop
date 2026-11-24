/**
 * The cell engine every screensaver draws through.
 *
 * The screen is one grid of cells, and a cell is either a dot of Kryoto's 3x3
 * matrix (ui/Matrix.tsx) or one ASCII glyph, at one of the matrix's four inks
 * (full, half, low, a trace) or not drawn at all. A screensaver only says what
 * each cell shows; the engine draws it, and keeps it cheap enough to leave on
 * a TV for hours:
 *
 * - One 2D canvas. Each dot and glyph is drawn once into a sprite sheet, and a
 *   frame only copies the cells that changed since the last one. Dots switch
 *   in hard steps, so a frame is usually a few dozen copies.
 * - At most `fps` frames a second (20 full screen), and none at all while the
 *   window is hidden.
 * - The whole picture wanders slowly as one piece: the canvas is a little
 *   larger than the screen and is moved by a transform in whole pixels, which
 *   the compositor does without redrawing anything. With the screensavers'
 *   own moves, nothing stays lit on the same pixels for long.
 * - With motion reduced, one still frame, changed every few minutes.
 *
 * Kept free of imports from the app (and of the DOM at load time) so the pure
 * parts run under `pnpm test`.
 */

/* ── Cells ─────────────────────────────────────────────────── */

/** The matrix's four inks, and nothing. */
export const OFF = 0
export const TRACE = 1
export const LOW = 2
export const HALF = 3
export const FULL = 4
/**
 * Half a trace: a field of dots far from anything lit (Rain's sky), so a whole
 * screen of grid reads as a grid and not as grey. Never used for a dot that
 * belongs to a shape.
 */
export const FAINT = 5
/** components/ui/matrix.tsx: full, half, low and the trace that keeps the grid legible; then FAINT. */
export const INK_ALPHA = [0, 0.14, 0.3, 0.55, 1, 0.065] as const

const LEVEL_MASK = 7
const ACCENT = 8

/** A dot of the matrix. */
export const dot = (level: number, accent = false): number => (level ? level | (accent ? ACCENT : 0) : 0)

/** One character in a cell; a space is the grid's trace dot. */
export function glyph(ch: string, level: number, accent = false): number {
  if (!level) return 0
  if (ch === ' ') return dot(TRACE)
  return (ch.charCodeAt(0) << 4) | level | (accent ? ACCENT : 0)
}

export const levelOf = (cell: number) => cell & LEVEL_MASK
export const isGlyph = (cell: number) => cell >>> 4 !== 0

/* ── Pure helpers ──────────────────────────────────────────── */

/** A stable pseudo-random number in [0, 1) for a pair of numbers. */
export function hash(a: number, b = 0): number {
  const n = Math.sin(a * 127.1 + b * 311.7 + 7.3) * 43758.5453
  return n - Math.floor(n)
}

/** A small seeded generator (mulberry32), so a run can be replayed in tests. */
export function seeded(seed: number): () => number {
  let s = seed >>> 0
  return () => {
    s = (s + 0x6d2b79f5) >>> 0
    let t = s
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/** The display's own frame lands within this of when it was asked for. */
const VSYNC_MS = 16

/**
 * How long to wait before asking for the next frame, so frames come no more
 * often than `fps` a second. Asked a display frame early: the frame itself
 * then waits for `capReady`.
 */
export function capDelay(last: number, now: number, fps: number): number {
  return Math.max(0, last + 1000 / fps - VSYNC_MS - now)
}

/** Whether a display frame at `now` may be drawn (a millisecond of slack for timer jitter). */
export function capReady(last: number, now: number, fps: number): boolean {
  return now - last >= 1000 / fps - 1
}

/**
 * A cell's width in device pixels: 80 across a 16:9 screen at `scale` 1, so
 * the picture is the same at 1080p and 4K, just sharper. Whole pixels, so
 * every dot is the same size.
 */
export function pitchFor(width: number, height: number, scale = 1): number {
  return Math.max(2, Math.round((Math.min(width, (height * 16) / 9) / 80) * scale))
}

/* ── What a screensaver gets ───────────────────────────────── */

export type Grid = {
  cols: number
  rows: number
  /** One value per cell, row by row (`dot`, `glyph`): what the cell shows this frame. */
  cells: Uint32Array
  /** The cells on screen before any wandering: what is kept inside stays in sight. */
  view: { c0: number; r0: number; c1: number; r1: number }
  /** A row's height as a share of a column's width. */
  rowPitch: number
}

export type Scene = {
  grid: Grid
  /** Game titles a screensaver may show, played last first: none hidden, none adult. */
  titles: () => readonly string[]
  /** The game played last, if any. */
  lastPlayed: () => string | null
  /** Motion reduced: draw a still composition for the time given. */
  reduced: boolean
  /** A thumbnail in Settings: same picture, nothing that needs reading. */
  thumb: boolean
  rand: () => number
  /** Ask for the start screen's own scene underneath (Field), or none. */
  underlay: (kind: 'splash' | null) => void
  /** Lay the grid out again for another screensaver's cells (Shuffle, between two). */
  setScale: (scale: number, rowPitch?: number) => Grid
}

export type Runner = {
  /** Fill `scene.grid.cells` for `t` ms after the start. */
  frame: (t: number) => void
}

export type VariantSpec = {
  id: string
  name: string
  /** Cell size against the default (Clock's numerals are drawn larger). */
  scale?: number
  /** Rows closer than columns (the mark's true shape). Default: the mark's. */
  rowPitch?: number
  /** With motion reduced, how long until the still frame changes. */
  stillFor?: (now: Date) => number
  make: (scene: Scene) => Runner
}

/** With motion reduced, a new still this often. */
export const STILL_MS = 180_000
/** How far the whole picture wanders, in cells, and how slowly (seconds per sway). */
const DRIFT_CELLS = 1.6
const DRIFT_X_S = 263
const DRIFT_Y_S = 197
/** Ink at full is this bright: a TV left on shows as little lit as it can. */
const BRIGHTNESS = 0.9
/** Rows a little closer than columns, as the mark's dots are (mark-data.ts). */
export const ROW_PITCH = 0.885
/** A dot's side as a share of a column (the matrix's 14% gaps: 0.62). */
const DOT = 0.62

/** Where the picture has wandered to after `t` ms, in CSS pixels. */
export function driftAt(t: number, amplitude: number): { x: number; y: number } {
  const s = t / 1000
  return {
    x: amplitude * Math.sin((2 * Math.PI * s) / DRIFT_X_S + 0.7),
    y: amplitude * Math.sin((2 * Math.PI * s) / DRIFT_Y_S),
  }
}

/* ── The engine ────────────────────────────────────────────── */

export type MountOptions = {
  fps: number
  titles: () => readonly string[]
  lastPlayed: () => string | null
  /** Wander as a whole (full screen); a thumbnail stays put. */
  drift: boolean
  thumb?: boolean
  onUnderlay?: (kind: 'splash' | null) => void
  /** Measured for performance checks: the work of each frame, in ms. */
  onFrame?: (ms: number) => void
}

function reducedMotion() {
  return typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches
}

/**
 * Draw `spec` into `canvas`, which must sit inside a positioned box: the
 * canvas is sized to that box (a little past it, to wander). Returns the stop.
 */
export function mountSaver(canvas: HTMLCanvasElement, spec: VariantSpec, opts: MountOptions): () => void {
  const ctx = canvas.getContext('2d', { alpha: true })
  const host = canvas.parentElement
  if (!ctx || !host) return () => {}

  const reduced = reducedMotion()
  let scale = spec.scale ?? 1
  let rowPitch = spec.rowPitch ?? ROW_PITCH

  // Geometry, in device pixels unless named CSS.
  let dpr = 1
  let pw = 2
  let ph = 2
  let ox = 0
  let oy = 0
  let margin = 0 // CSS px the canvas reaches past the box on every side
  let grid: Grid = { cols: 0, rows: 0, cells: new Uint32Array(0), view: { c0: 0, r0: 0, c1: 0, r1: 0 }, rowPitch }
  let shown = new Uint32Array(0)

  // Colours: the ink is the foreground, the accent the reader's palette.
  let ink = '#ededed'
  let accent = ink
  let font = 'ui-monospace, monospace'
  const readPalette = () => {
    const css = getComputedStyle(canvas)
    ink = css.color || ink
    accent = css.getPropertyValue('--primary').trim() || ink
    font = css.getPropertyValue('--font-ascii').trim() || font
  }

  // The sprite sheet: every look a cell can have, drawn once.
  const sheet = document.createElement('canvas')
  const sctx = sheet.getContext('2d')!
  const SLOTS_X = 32
  const SLOTS_Y = 24
  let slots = new Map<number, number>()
  const clearSprites = () => {
    slots = new Map()
    sheet.width = pw * SLOTS_X
    sheet.height = ph * SLOTS_Y
    shown.fill(0xffffffff)
  }
  const sprite = (cell: number): number => {
    let slot = slots.get(cell)
    if (slot !== undefined) return slot
    if (slots.size >= SLOTS_X * SLOTS_Y) clearSprites()
    slot = slots.size
    slots.set(cell, slot)
    const x = (slot % SLOTS_X) * pw
    const y = Math.floor(slot / SLOTS_X) * ph
    sctx.clearRect(x, y, pw, ph)
    sctx.globalAlpha = INK_ALPHA[levelOf(cell)]! * BRIGHTNESS
    sctx.fillStyle = cell & ACCENT ? accent : ink
    const code = cell >>> 4
    if (!code) {
      const side = Math.max(1, Math.min(Math.round(pw * DOT), ph - 1))
      const dx = x + Math.floor((pw - side) / 2)
      const dy = y + Math.floor((ph - side) / 2)
      sctx.beginPath()
      if (side >= 6 && sctx.roundRect) sctx.roundRect(dx, dy, side, side, side * 0.2)
      else sctx.rect(dx, dy, side, side)
      sctx.fill()
    } else {
      sctx.font = `700 ${Math.max(4, Math.round(ph * 0.92))}px ${font}`
      sctx.textAlign = 'center'
      sctx.textBaseline = 'middle'
      sctx.fillText(String.fromCharCode(code), x + pw / 2, y + ph / 2 + ph * 0.04)
    }
    sctx.globalAlpha = 1
    return slot
  }

  const scene: Scene = {
    get grid() {
      return grid
    },
    titles: opts.titles,
    lastPlayed: opts.lastPlayed,
    reduced,
    thumb: !!opts.thumb,
    rand: Math.random,
    underlay: (kind) => opts.onUnderlay?.(kind),
    setScale: (next, rows) => {
      scale = next
      rowPitch = rows ?? ROW_PITCH
      layout()
      return grid
    },
  }

  const layout = () => {
    const box = host.getBoundingClientRect()
    const w = Math.max(1, box.width)
    const h = Math.max(1, box.height)
    dpr = window.devicePixelRatio || 1
    pw = pitchFor(w * dpr, h * dpr, scale)
    ph = Math.max(2, Math.round(pw * rowPitch))
    margin = opts.drift && !reduced ? Math.ceil(((DRIFT_CELLS + 0.5) * pw) / dpr) : 0
    const cw = w + margin * 2
    const chh = h + margin * 2
    Object.assign(canvas.style, { position: 'absolute', left: `${-margin}px`, top: `${-margin}px`, width: `${cw}px`, height: `${chh}px` })
    canvas.width = Math.round(cw * dpr)
    canvas.height = Math.round(chh * dpr)
    const cols = Math.floor(canvas.width / pw)
    const rows = Math.floor(canvas.height / ph)
    ox = Math.floor((canvas.width - cols * pw) / 2)
    oy = Math.floor((canvas.height - rows * ph) / 2)
    const m = margin * dpr
    grid = {
      cols,
      rows,
      cells: new Uint32Array(cols * rows),
      view: {
        c0: Math.ceil((m - ox) / pw),
        r0: Math.ceil((m - oy) / ph),
        c1: Math.floor((m + w * dpr - ox) / pw),
        r1: Math.floor((m + h * dpr - oy) / ph),
      },
      rowPitch,
    }
    shown = new Uint32Array(cols * rows)
    ctx.clearRect(0, 0, canvas.width, canvas.height)
    readPalette()
    clearSprites()
  }

  const paint = () => {
    const { cells, cols } = grid
    for (let i = 0; i < cells.length; i++) {
      const v = cells[i]!
      if (v === shown[i]) continue
      shown[i] = v
      const x = ox + (i % cols) * pw
      const y = oy + Math.floor(i / cols) * ph
      ctx.clearRect(x, y, pw, ph)
      if (!levelOf(v)) continue
      const slot = sprite(v)
      ctx.drawImage(sheet, (slot % SLOTS_X) * pw, Math.floor(slot / SLOTS_X) * ph, pw, ph, x, y, pw, ph)
    }
  }

  let lastDrift = ''
  const wander = (t: number) => {
    if (!margin) return
    const d = driftAt(t, (DRIFT_CELLS * pw) / dpr)
    const x = Math.round(d.x * dpr) / dpr
    const y = Math.round(d.y * dpr) / dpr
    const key = `${x},${y}`
    if (key === lastDrift) return
    lastDrift = key
    canvas.style.transform = `translate3d(${x}px, ${y}px, 0)`
  }

  layout()
  let runner = spec.make(scene)

  const t0 = performance.now()
  let last = -Infinity
  let timer = 0
  let raf = 0
  let stopped = false

  const frame = (now: number) => {
    raf = 0
    if (stopped || document.hidden) return
    if (!reduced && !capReady(last, now, opts.fps)) {
      raf = requestAnimationFrame(frame)
      return
    }
    last = now
    const began = performance.now()
    // A display frame's time is when the frame began, which can be a moment before the start.
    const t = Math.max(0, now - t0)
    runner.frame(t)
    paint()
    wander(t)
    opts.onFrame?.(performance.now() - began)
    schedule()
  }
  const schedule = () => {
    if (stopped || document.hidden || timer || raf) return
    const delay = reduced ? (spec.stillFor?.(new Date()) ?? STILL_MS) : capDelay(last, performance.now(), opts.fps)
    timer = window.setTimeout(() => {
      timer = 0
      raf = requestAnimationFrame(frame)
    }, delay)
  }
  raf = requestAnimationFrame(frame)

  // Hidden: nothing scheduled at all. Back: straight on.
  const visibility = () => {
    if (document.hidden) {
      window.clearTimeout(timer)
      cancelAnimationFrame(raf)
      timer = raf = 0
    } else if (!timer && !raf) raf = requestAnimationFrame(frame)
  }
  document.addEventListener('visibilitychange', visibility)

  // A new size lays the grid out again and starts the picture over.
  let size = `${host.clientWidth}x${host.clientHeight}x${window.devicePixelRatio}`
  const resized = () => {
    const next = `${host.clientWidth}x${host.clientHeight}x${window.devicePixelRatio}`
    if (next === size) return
    size = next
    scale = spec.scale ?? 1
    rowPitch = spec.rowPitch ?? ROW_PITCH
    layout()
    runner = spec.make(scene)
    if (reduced) {
      window.clearTimeout(timer)
      timer = 0
      if (!raf) raf = requestAnimationFrame(frame)
    }
  }
  const watch = new ResizeObserver(resized)
  watch.observe(host)
  window.addEventListener('resize', resized)

  // The palette or the font changed (Settings, a font arriving): redraw every cell.
  const repaint = () => {
    readPalette()
    clearSprites()
  }
  const themes = new MutationObserver(repaint)
  themes.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'data-palette', 'style'] })
  document.fonts?.addEventListener?.('loadingdone', repaint)

  return () => {
    stopped = true
    window.clearTimeout(timer)
    cancelAnimationFrame(raf)
    document.removeEventListener('visibilitychange', visibility)
    watch.disconnect()
    window.removeEventListener('resize', resized)
    themes.disconnect()
    document.fonts?.removeEventListener?.('loadingdone', repaint)
    opts.onUnderlay?.(null)
  }
}

/* ── Small drawing helpers for the screensavers ────────────── */

/** Fill every cell with `value` (the trace grid, or nothing). */
export function fill(grid: Grid, value: number) {
  grid.cells.fill(value)
}

/** Put one cell, ignoring anything off the grid. */
export function put(grid: Grid, c: number, r: number, value: number) {
  if (c < 0 || r < 0 || c >= grid.cols || r >= grid.rows) return
  grid.cells[r * grid.cols + c] = value
}

/** Write a line of text from (c, r), one character per cell. */
export function text(grid: Grid, c: number, r: number, line: string, level: number, accent = false) {
  for (let i = 0; i < line.length; i++) put(grid, c + i, r, glyph(line[i]!, level, accent))
}
