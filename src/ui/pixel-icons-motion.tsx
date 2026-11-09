'use client'

/**
 * The motion for Kryoto's pixel icons (`pixel-icons.tsx`), and the Classic
 * style: it loads the classic shapes (`pixel-icons-classic.ts`) only on a page
 * in that style, fills them into the icons, and its CSS switches the pictures.
 *
 * `<PixelIconMotion />` is mounted once per app. From then on, hovering or
 * focusing a button, link, tab or menu item plays the icons inside it once:
 * each icon's own short story where it has one (`ANIM`), otherwise a rain of
 * bright cells down its columns that flips the picture's cells as it passes,
 * the way the matrix's glyphs fall. Every animation ends on the still
 * picture: the frames are written into the icon's two paths and the still
 * picture put back, so the rendered markup is never left changed.
 *
 * An icon with `animate-spin` (or a spinner) turns in quarter steps while it
 * is on screen. Nothing plays with reduced motion (the system's setting, or
 * `data-motion="reduced"`) or in the Classic style (`data-icons="classic"`).
 *
 * `<PixelSplash icon="Download" />` is the big variant: the same picture in a
 * mosaic of finer cells, decoded by falling rain and then kept alive by it,
 * for empty states, setups and splash screens. It stays pixel art, and keeps
 * moving, in the Classic style too.
 *
 * THE SAME FILE LIVES IN EVERY KRYOTO FRONTEND; see `pixel-icons.tsx`.
 */

import { useEffect, useRef, useState, type CSSProperties, type ReactElement } from 'react'
import { ANIM, GRID, PIXEL_ICONS, artOffset, gridPaths, parseArt, type IconAnim, type LucideIcon, type PixelIconName } from './pixel-icons'

type Grid = number[][]
type Frame = [string, string]
type Sequence = { frames: Frame[]; step: number }
type Rect = [number, number, number, number]

const N = GRID
const blank = (): Grid => Array.from({ length: N }, () => Array<number>(N).fill(0))
const copy = (g: Grid): Grid => g.map((row) => row.slice())
/** A cell's tone, 0 outside the grid. */
const at = (g: Grid, x: number, y: number): number => g[y]?.[x] ?? 0
const setAt = (g: Grid, x: number, y: number, v: number) => {
  const row = g[y]
  if (row && x >= 0 && x < row.length) row[x] = v
}
const inRects = (rects: Rect[] | undefined, x: number, y: number) =>
  Boolean(rects?.some(([rx, ry, w, h]) => x >= rx && x < rx + w && y >= ry && y < ry + h))

/** A small seeded random, so an icon rains the same way every time. */
function seeded(text: string): () => number {
  let h = 2166136261
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 16777619)
  return () => {
    h = Math.imul(h ^ (h >>> 15), 2246822507)
    h = Math.imul(h ^ (h >>> 13), 3266489909)
    return ((h ^= h >>> 16) >>> 0) / 4294967296
  }
}

function shift(g: Grid, dx: number, dy: number, fix?: Rect[]): Grid {
  const out = blank()
  for (let y = 0; y < N; y++)
    for (let x = 0; x < N; x++) {
      const v = at(g, x, y)
      if (!v || inRects(fix, x, y)) continue
      setAt(out, x + dx, y + dy, Math.max(at(out, x + dx, y + dy), v))
    }
  for (let y = 0; y < N; y++) for (let x = 0; x < N; x++) if (inRects(fix, x, y) && at(g, x, y)) setAt(out, x, y, at(g, x, y))
  return out
}

const turnCw = (g: Grid): Grid => g.map((_, y) => g.map((_, x) => at(g, y, N - 1 - x)))
const turnCcw = (g: Grid): Grid => g.map((_, y) => g.map((_, x) => at(g, N - 1 - y, x)))

/** The default: rain down each column, a few steps apart, flipping cells. */
function rain(name: string, g: Grid): Grid[] {
  const rnd = seeded(name)
  const offsets = Array.from({ length: N }, () => Math.floor(rnd() * 6))
  const length = N + Math.max(...offsets) + 2
  const frames: Grid[] = []
  for (let f = 0; f < length; f++) {
    const out = copy(g)
    for (let x = 0; x < N; x++) {
      const head = f - (offsets[x] ?? 0)
      if (head >= 0 && head < N) setAt(out, x, head, at(g, x, head) === 2 ? 0 : 2)
      for (const tail of [head - 1, head - 2]) if (tail >= 0 && tail < N && at(g, x, tail) === 0) setAt(out, x, tail, 1)
    }
    frames.push(out)
  }
  return frames
}

function draw(g: Grid, order: 'ltr' | 'rtl' | 'ttb' | 'btt' | 'out'): Grid[] {
  const c = (N - 1) / 2
  const key = (x: number, y: number) =>
    order === 'ltr' ? x : order === 'rtl' ? N - 1 - x : order === 'ttb' ? y : order === 'btt' ? N - 1 - y : Math.round(Math.max(Math.abs(x - c), Math.abs(y - c)))
  const steps = [...new Set(g.flatMap((row, y) => row.map((v, x) => (v ? key(x, y) : -1))).filter((k) => k >= 0))].sort((a, b) => a - b)
  return [blank(), ...steps.map((upTo) => g.map((row, y) => row.map((v, x) => (key(x, y) <= upTo ? v : 0))))]
}

function framesFor(name: PixelIconName, still: Grid, filled: boolean): { frames: Grid[]; step: number } {
  const anim: IconAnim | undefined = filled ? undefined : ANIM[name]
  if (!anim) return { frames: rain(name, still), step: 38 }
  switch (anim.k) {
    case 'nudge': {
      const moved = shift(still, anim.x, anim.y, anim.fix)
      return { frames: [moved, moved, still, moved, moved], step: 80 }
    }
    case 'shake': {
      const a = shift(still, anim.x, anim.y, anim.fix)
      const b = shift(still, -anim.x, -anim.y, anim.fix)
      return { frames: [a, b, a, b, still], step: 60 }
    }
    case 'draw':
      return { frames: draw(still, anim.o), step: 45 }
    case 'spin': {
      const turn = anim.ccw ? turnCcw : turnCw
      const one = turn(still)
      const two = turn(one)
      return { frames: [one, two, turn(two)], step: 90 }
    }
    case 'scan':
      return {
        frames: Array.from({ length: N }, (_, r) => still.map((row, y) => (y === r ? row.map((v) => (v === 2 ? 0 : 2)) : row))),
        step: 40,
      }
    case 'fill': {
      const art = PIXEL_ICONS.FILLED[name]
      const solid = art ? parseArt(art) : still
      return { frames: [solid, still, solid], step: 120 }
    }
    case 'pulse': {
      const dimmed = still.map((row) => row.map((v) => (v ? 1 : 0)))
      return { frames: [dimmed, still, dimmed], step: 110 }
    }
    case 'shut': {
      const ys = still.flatMap((row, y) => (row.some(Boolean) ? [y] : []))
      const mid = Math.round((Math.min(...ys) + Math.max(...ys)) / 2)
      const xs = still.flatMap((row) => row.flatMap((v, x) => (v ? [x] : [])))
      const shut = blank()
      for (let x = Math.min(...xs); x <= Math.max(...xs); x++) setAt(shut, x, mid, 2)
      const half = still.map((row, y) => (Math.abs(y - mid) <= 2 ? row : row.map(() => 0)))
      return { frames: [half, shut, half], step: 70 }
    }
    case 'blink': {
      const out = still.map((row, y) => row.map((v, x) => (inRects(anim.off, x, y) ? 0 : v)))
      return { frames: [out, still, out], step: 120 }
    }
    case 'flash': {
      const out = still.map((row, y) => row.map((v, x) => (inRects(anim.on, x, y) ? 2 : v)))
      return { frames: [out, still, out], step: 90 }
    }
    case 'frames':
      return { frames: anim.f.map(parseArt), step: anim.step ?? 90 }
  }
}

const sequences = new Map<string, Sequence>()

function artOf(name: string, filled: boolean): string | undefined {
  return (filled && PIXEL_ICONS.FILLED[name as PixelIconName]) || PIXEL_ICONS.ART[name as PixelIconName]
}

/** The frames an icon plays (not counting the still picture it ends on). */
export function sequenceFor(name: string, filled = false): Sequence | null {
  const key = `${name}${filled ? ':f' : ''}`
  const hit = sequences.get(key)
  if (hit) return hit
  const art = artOf(name, filled)
  if (!art) return null
  const { frames, step } = framesFor(name as PixelIconName, parseArt(art), filled)
  const seq = { frames: frames.map((g) => gridPaths(g)), step }
  sequences.set(key, seq)
  return seq
}

/** Quarter turns of the still picture, for `animate-spin` and spinners. */
function spinFor(name: string, filled: boolean): Frame[] | null {
  const key = `${name}${filled ? ':f' : ''}:spin`
  const hit = sequences.get(key)
  if (hit) return hit.frames
  const art = artOf(name, filled)
  if (!art) return null
  const still = parseArt(art)
  const one = turnCw(still)
  const two = turnCw(one)
  const frames = [still, one, two, turnCw(two)].map((g) => gridPaths(g))
  sequences.set(key, { frames, step: 110 })
  return frames
}

// --- playing -------------------------------------------------------------------

/**
 * Whether `el` holds still. The big splash pictures are not the small icons,
 * which the Classic style swaps for line icons: they stay, and keep moving, in
 * either style (`splash`), and hold still only with reduced motion.
 */
function still(el: Element, splash = false): boolean {
  if (!splash && el.closest('[data-icons="classic"]')) return true
  if (el.closest('[data-motion="reduced"]')) return true
  if (el.closest('[data-motion="full"]')) return false
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches
}

const playing = new WeakSet<Element>()

function pathsOf(svg: SVGSVGElement): [SVGPathElement, SVGPathElement] | null {
  const paths = svg.querySelectorAll(':scope > svg.kp-p > path')
  return paths.length >= 2 ? [paths[0] as SVGPathElement, paths[1] as SVGPathElement] : null
}

/** Play one icon's frames once, ending on its still picture. */
export function playIcon(svg: SVGSVGElement, force = false): void {
  if (playing.has(svg) || svg.hasAttribute('data-kp-loop')) return
  if (!force && still(svg)) return
  const name = svg.getAttribute('data-kp')
  const seq = name ? sequenceFor(name, svg.hasAttribute('data-kp-filled')) : null
  const paths = pathsOf(svg)
  if (!seq || !paths) return
  // Measured again where it stands now: a pop-in may have been mid-scale before.
  snapIcon(svg)
  const [full, dim] = paths
  const rest: Frame = [full.getAttribute('d') ?? '', dim.getAttribute('d') ?? '']
  playing.add(svg)
  let i = 0
  let written: Frame = rest
  const tick = () => {
    // React drew a different picture meanwhile (its props changed): it wins.
    if (full.getAttribute('d') !== written[0] || !svg.isConnected) {
      playing.delete(svg)
      return
    }
    if (i >= seq.frames.length) {
      full.setAttribute('d', rest[0])
      dim.setAttribute('d', rest[1])
      playing.delete(svg)
      return
    }
    written = seq.frames[i++] ?? rest
    full.setAttribute('d', written[0])
    dim.setAttribute('d', written[1])
    setTimeout(tick, seq.step)
  }
  tick()
}

/** Play every icon under `root` (the gallery and the recordings use this). */
export function playIcons(root: ParentNode, force = false, limit = Infinity): void {
  const icons =
    root instanceof SVGSVGElement && root.matches('.kp-icon') ? [root] : Array.from(root.querySelectorAll<SVGSVGElement>('svg.kp-icon'))
  icons.slice(0, limit).forEach((svg) => playIcon(svg, force))
}

const HOST =
  'a[href], button, summary, label, [role="button"], [role="link"], [role="menuitem"], [role="menuitemradio"], [role="menuitemcheckbox"], [role="tab"], [role="option"], [role="switch"], [data-kp-host]'

let installed = 0
let current: Element | null = null
let loopTimer: ReturnType<typeof setInterval> | null = null
const looping = new Map<SVGSVGElement, Frame>()

function onOver(e: PointerEvent) {
  const target = e.target as Element | null
  if (!target?.closest) return
  const host = target.closest(HOST) ?? target.closest('svg.kp-icon')
  if (!host || host === current) return
  current = host
  playIcons(host, false, 4)
}

function onOut(e: PointerEvent) {
  const next = e.relatedTarget as Node | null
  if (current && (!next || !current.contains(next))) current = null
}

function onFocus(e: FocusEvent) {
  const target = e.target as Element | null
  if (!target?.matches) return
  let visible: boolean
  try {
    visible = target.matches(':focus-visible')
  } catch {
    visible = true
  }
  if (visible) playIcons(target, false, 4)
}

/** Spinners and `animate-spin` icons: quarter turns while they are on screen. */
function loopTick() {
  if (typeof document === 'undefined' || document.hidden) return
  const now = performance.now()
  const seen = new Set<SVGSVGElement>()
  document.querySelectorAll<SVGSVGElement>('svg.kp-icon[data-kp-loop]').forEach((svg) => {
    const name = svg.getAttribute('data-kp')
    const paths = pathsOf(svg)
    if (!name || !paths) return
    seen.add(svg)
    if (!looping.has(svg)) looping.set(svg, [paths[0].getAttribute('d') ?? '', paths[1].getAttribute('d') ?? ''])
    if (still(svg)) return
    const frames = spinFor(name, svg.hasAttribute('data-kp-filled'))
    if (!frames) return
    const frame = frames[Math.floor(now / 110) % frames.length]
    if (frame && paths[0].getAttribute('d') !== frame[0]) {
      paths[0].setAttribute('d', frame[0])
      paths[1].setAttribute('d', frame[1])
    }
  })
  // An icon that stopped spinning (its class changed): put its picture back.
  for (const [svg, rest] of looping) {
    if (seen.has(svg)) continue
    looping.delete(svg)
    const paths = svg.isConnected ? pathsOf(svg) : null
    if (paths) {
      paths[0].setAttribute('d', rest[0])
      paths[1].setAttribute('d', rest[1])
    }
  }
}

// --- whole device pixels ---------------------------------------------------------

/**
 * Square cells, every one the same size. The server picks a viewBox from the
 * icon's CSS size, but the screen's scale (125%, 150%) and where the icon
 * lands (half a pixel in, beside text) can still put cell edges between
 * device pixels, and then some cells draw a pixel wider or taller than the
 * rest. On the page, each icon's pixel picture is re-fitted to the device
 * pixels it actually covers: whole pixels per cell, the picture centred, its
 * edges on the pixel grid.
 */
function snapIcon(svg: SVGSVGElement) {
  const inner = svg.querySelector<SVGSVGElement>(':scope > svg.kp-p')
  if (!inner) return
  const box = svg.getBoundingClientRect()
  if (!box.width || !box.height) return
  const dpr = window.devicePixelRatio || 1
  const across = box.width * dpr
  // Device pixels per cell: whole numbers, never under one (a cell under a
  // pixel loses whole lines to crisp edges). Rounded to the nearest, not
  // down: at 125% or 150% a 14 px icon covers 17.5 or 21 device pixels, and
  // flooring drew it with one-pixel cells, 8-10 CSS pixels across, far
  // smaller than the text beside it. Most pictures leave a cell free on each
  // side, so a slightly bigger grid spills only its empty margin past the box
  // (the CSS lets it).
  const cell = Math.max(1, Math.round(across / 11.5))
  const span = across / cell
  // Centre the picture on what it draws (`data-kp-o`, half a cell for an odd
  // width), then nudge it so its edges sit on a device pixel wherever the
  // icon itself starts.
  const [dx = 0, dy = 0] = (inner.dataset.kpO ?? '').split(' ').map(Number)
  const edge = (start: number, off: number) => {
    const at = start * dpr + ((span - GRID) / 2 - off) * cell
    return -(Math.round(at) - start * dpr) / cell
  }
  const view = `${edge(box.left, dx).toFixed(4)} ${edge(box.top, dy).toFixed(4)} ${span.toFixed(4)} ${span.toFixed(4)}`
  if (inner.getAttribute('viewBox') !== view) inner.setAttribute('viewBox', view)
}

/**
 * The 3x3 matrix (components/ui/matrix.tsx) gets the same care. It lays its
 * nine dots out in whole CSS pixels, which are whole device pixels only at
 * 100%, 200% or 300%; at 125% or 150% a 3 px dot is 3.75 device pixels and
 * the dots draw unequal again. There, dots, gaps and edges are counted in
 * device pixels instead.
 */
function snapMatrix(el: HTMLElement) {
  const across = el.getBoundingClientRect().width
  if (!across) return
  const dpr = window.devicePixelRatio || 1
  const unit = Number.isInteger(dpr) ? 1 : dpr
  const box = across * unit
  const dot = Math.max(2, Math.floor(box * 0.29))
  const gap = Math.max(1, Math.floor((box - 3 * dot) / 2.6))
  const edge = Math.max(0, Math.floor((box - 3 * dot - 2 * gap) / 2))
  const px = (n: number) => `${+(n / unit).toFixed(4)}px`
  Object.assign(el.style, {
    gridTemplateColumns: `repeat(3, ${px(dot)})`,
    gridTemplateRows: `repeat(3, ${px(dot)})`,
    gap: px(gap),
    placeContent: 'start',
    padding: `${px(edge)} 0 0 ${px(edge)}`,
    boxSizing: 'border-box',
  })
  // Rounding a dot this small only blurs its corners.
  el.style.setProperty('--kryo-m-r', dot < 6 ? '0' : '')
}

const SNAPPED = 'svg.kp-icon, .kryo-matrix'
let snapQueued = false
const snapPending = new Set<Element>()
function queueSnap(els: Iterable<Element>) {
  for (const el of els) snapPending.add(el)
  if (snapQueued) return
  snapQueued = true
  // A timer, not requestAnimationFrame: a tab opened in the background never
  // gets an animation frame, and its icons would stay unfitted until shown.
  setTimeout(() => {
    snapQueued = false
    const batch = [...snapPending]
    snapPending.clear()
    for (const el of batch) {
      if (!el.isConnected) continue
      if (el instanceof SVGSVGElement) snapIcon(el)
      else if (el instanceof HTMLElement) snapMatrix(el)
    }
  }, 0)
}
const snapAll = () => queueSnap(document.querySelectorAll(SNAPPED))

// --- the Classic style -------------------------------------------------------
// The classic shapes are a module of their own (`pixel-icons-classic.ts`),
// loaded only on a page in the Classic style, so nobody on Pixel downloads
// them. Every icon already holds a `<use>` of its shape; the shapes go into one
// sprite appended to <body>, outside anything React draws, and
// `data-kp-classic` on <html> tells the CSS the pictures are there to show.

let classic: Record<string, string> | null = null
let classicLoading: Promise<void> | null = null
const inClassic = () => document.documentElement.getAttribute('data-icons') === 'classic'

function loadClassic(): Promise<void> {
  classicLoading ??= import('./pixel-icons-classic').then(
    (m) => {
      classic = m.CLASSIC
    },
    () => {
      // Offline or a failed chunk: the pixel pictures stay, and the next switch tries again.
      classicLoading = null
    },
  )
  return classicLoading
}

function addClassicSprite() {
  if (!classic || document.getElementById('kp-classic')) return
  const sprite = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
  sprite.id = 'kp-classic'
  sprite.setAttribute('aria-hidden', 'true')
  sprite.setAttribute('width', '0')
  sprite.setAttribute('height', '0')
  sprite.setAttribute('style', 'position:absolute;width:0;height:0;overflow:hidden')
  sprite.innerHTML = Object.entries(classic)
    .map(([name, shape]) => `<symbol id="kp-c-${name}">${shape}</symbol>`)
    .join('')
  document.body.appendChild(sprite)
  document.documentElement.setAttribute('data-kp-classic', '')
}

/** The style is Classic now (at start, from Appearance, or Kryoto Desktop's own setting): make sure the shapes are there. */
function onStyle() {
  if (!inClassic()) return
  if (classic) addClassicSprite()
  else void loadClassic().then(addClassicSprite)
}

// Started as soon as this module runs, while the page is still hydrating, so
// the classic pictures are usually there by the time the icons are.
if (typeof document !== 'undefined' && inClassic()) void loadClassic()

function onAdded(records: MutationRecord[]) {
  const found: Element[] = []
  for (const r of records)
    r.addedNodes.forEach((n) => {
      if (!(n instanceof Element)) return
      if (n.matches(SNAPPED)) found.push(n)
      else n.querySelectorAll(SNAPPED).forEach((s) => found.push(s))
    })
  if (found.length) queueSnap(found)
}

let observer: MutationObserver | null = null
let styleObserver: MutationObserver | null = null
let dprQuery: MediaQueryList | null = null
function watchScale() {
  dprQuery?.removeEventListener('change', onScale)
  dprQuery = matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`)
  dprQuery.addEventListener('change', onScale)
}
function onScale() {
  watchScale()
  snapAll()
}
let resizeTimer: ReturnType<typeof setTimeout> | null = null
function onResize() {
  if (resizeTimer) clearTimeout(resizeTimer)
  resizeTimer = setTimeout(snapAll, 150)
}
// Fitting depends on where an icon sits, and that moves after the first fit:
// web fonts arrive and re-flow the text an icon sits beside, images load,
// a panel opens. Fit again once each of those settles.
function refitWhenSettled() {
  document.fonts?.ready.then(snapAll, () => {})
  if (document.readyState !== 'complete') window.addEventListener('load', snapAll, { once: true })
  setTimeout(snapAll, 1000)
}
// A transition that ends can have moved icons outside the element itself (a
// bar that grows re-centres its neighbours), so it re-checks them all, once.
function onTransitionEnd() {
  onResize()
}
// Anything else that moves an icon by a fraction of a pixel (a bar whose
// contents load late and re-centre it) is caught by a slow re-check while the
// page is visible; an icon that has not moved is left untouched.
let refitTimer: ReturnType<typeof setInterval> | null = null
function refitTick() {
  if (document.visibilityState === 'visible') snapAll()
}

function install(): () => void {
  installed += 1
  if (installed === 1) {
    document.addEventListener('pointerover', onOver, { passive: true })
    document.addEventListener('pointerout', onOut, { passive: true })
    document.addEventListener('focusin', onFocus)
    loopTimer = setInterval(loopTick, 55)
    observer = new MutationObserver(onAdded)
    observer.observe(document.body, { childList: true, subtree: true })
    styleObserver = new MutationObserver(onStyle)
    styleObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['data-icons'] })
    onStyle()
    window.addEventListener('resize', onResize, { passive: true })
    document.addEventListener('transitionend', onTransitionEnd, { passive: true })
    watchScale()
    snapAll()
    refitWhenSettled()
    refitTimer = setInterval(refitTick, 3000)
  }
  return () => {
    installed -= 1
    if (installed > 0) return
    document.removeEventListener('pointerover', onOver)
    document.removeEventListener('pointerout', onOut)
    document.removeEventListener('focusin', onFocus)
    if (loopTimer) clearInterval(loopTimer)
    loopTimer = null
    if (refitTimer) clearInterval(refitTimer)
    refitTimer = null
    observer?.disconnect()
    observer = null
    styleObserver?.disconnect()
    styleObserver = null
    window.removeEventListener('resize', onResize)
    document.removeEventListener('transitionend', onTransitionEnd)
    window.removeEventListener('load', snapAll)
    dprQuery?.removeEventListener('change', onScale)
    dprQuery = null
  }
}

/** The Pixel/Classic switch and the classic spinner, for every icon. */
export const PIXEL_ICON_CSS = `
svg.kp-icon, svg.kp-icon > svg.kp-p { overflow: visible }
html[data-icons="classic"][data-kp-classic] svg.kp-icon > svg.kp-p { display: none }
html[data-icons="classic"][data-kp-classic] svg.kp-icon > use.kp-l { display: inline }
html[data-icons="classic"] svg.kp-icon.kp-spin { animation: kp-turn 1s linear infinite; transform-origin: center }
@keyframes kp-turn { to { transform: rotate(360deg) } }
svg.kp-splash { display: block }
`

/** Where an app without kryo.to's own setting keeps the Pixel/Classic choice (the same key kryo.to uses). */
export const ICON_STYLE_KEY = 'kryo-icons'

/** The choice saved on this device: Kryoto's pixel icons unless Classic was picked. */
export function storedIconStyle(): 'pixel' | 'classic' {
  try {
    return localStorage.getItem(ICON_STYLE_KEY) === 'classic' ? 'classic' : 'pixel'
  } catch {
    return 'pixel'
  }
}

/** Show pixel or classic icons from now on, and remember it on this device. */
export function setIconStyle(style: 'pixel' | 'classic') {
  if (style === 'classic') document.documentElement.setAttribute('data-icons', 'classic')
  else document.documentElement.removeAttribute('data-icons')
  try {
    localStorage.setItem(ICON_STYLE_KEY, style)
  } catch {
    /* storage blocked: the choice lasts until the app closes */
  }
}

/**
 * Mount once near the root of an app: the style switch and the motion.
 * `stored`: the app has no other source for the choice (Forge, the
 * Minecraft launcher), so apply what this device saved.
 */
export function PixelIconMotion({ stored = false }: { stored?: boolean }): ReactElement {
  useEffect(() => {
    if (stored && storedIconStyle() === 'classic') document.documentElement.setAttribute('data-icons', 'classic')
  }, [stored])
  useEffect(() => install(), [])
  return (
    <style href="kryoto-pixel-icons" precedence="default">
      {PIXEL_ICON_CSS}
    </style>
  )
}

// --- the big variant -------------------------------------------------------------

const SUB = 2
const BIG = N * SUB

type Drop = { x: number; y: number; speed: number; trail: number }

function newDrop(x: number, rnd: () => number, start?: number): Drop {
  return { x, y: start ?? -Math.floor(rnd() * BIG), speed: 0.45 + rnd() * 0.7, trail: 3 + Math.floor(rnd() * 6) }
}

function cellsPath(cells: Array<[number, number]>): string {
  // Each mosaic cell a little smaller than its slot, so the grain shows.
  return cells.map(([x, y]) => `M${x + 0.07} ${y + 0.07}h0.86v0.86h-0.86z`).join('')
}

/**
 * The big variant of an icon: its picture in a mosaic of 24 by 24 finer
 * cells. Rain falls down every column; the picture's cells appear as the
 * first drop passes them, then the rain keeps falling, faint, around and
 * through it. Still with reduced motion or the Classic style.
 */
export function PixelSplash({
  icon,
  className,
  style,
  label,
}: {
  /** The icon itself (`icon={Download}`) or its picture's name. */
  icon: PixelIconName | LucideIcon
  className?: string
  style?: CSSProperties
  /** Read out instead of the picture; without it the picture is decorative. */
  label?: string
}) {
  const name = (typeof icon === 'string' ? icon : icon.displayName) as PixelIconName
  const art = PIXEL_ICONS.ART[name] ?? PIXEL_ICONS.ART.Square
  const offset = artOffset(art)
  const ref = useRef<SVGSVGElement>(null)
  const [layers, setLayers] = useState<[string, string, string, string]>(() => {
    const g = parseArt(art)
    const full: Array<[number, number]> = []
    const dim: Array<[number, number]> = []
    for (let y = 0; y < BIG; y++)
      for (let x = 0; x < BIG; x++) {
        const v = at(g, Math.floor(x / SUB), Math.floor(y / SUB))
        if (v === 2) full.push([x, y])
        else if (v === 1) dim.push([x, y])
      }
    return [cellsPath(full), cellsPath(dim), '', '']
  })

  useEffect(() => {
    const svg = ref.current
    if (!svg || still(svg, true)) return
    const g = parseArt(art)
    const tone = (x: number, y: number) => at(g, Math.floor(x / SUB), Math.floor(y / SUB))
    const rnd = seeded(`${name}:splash`)
    const drops = Array.from({ length: BIG }, (_, x) => newDrop(x, rnd, -Math.floor(rnd() * 12)))
    const shown = Array.from({ length: BIG }, () => Array<boolean>(BIG).fill(false))
    let visible = true
    const io = typeof IntersectionObserver === 'function' ? new IntersectionObserver(([e]) => (visible = e?.isIntersecting ?? true)) : null
    io?.observe(svg)
    const timer = setInterval(() => {
      if (!visible || document.hidden) return
      const full: Array<[number, number]> = []
      const dim: Array<[number, number]> = []
      const trail: Array<[number, number]> = []
      const heads: Array<[number, number]> = []
      const headAt = new Set<string>()
      for (const d of drops) {
        d.y += d.speed
        const head = Math.floor(d.y)
        for (let y = 0; y <= Math.min(head, BIG - 1); y++) {
          const row = shown[y]
          if (row) row[d.x] = true
        }
        if (head - d.trail >= BIG) Object.assign(d, newDrop(d.x, rnd))
      }
      for (const d of drops) {
        const head = Math.floor(d.y)
        for (let k = 0; k <= d.trail; k++) {
          const y = head - k
          if (y < 0 || y >= BIG) continue
          if (k === 0) {
            heads.push([d.x, y])
            headAt.add(`${d.x},${y}`)
          } else if (!tone(d.x, y)) trail.push([d.x, y])
        }
      }
      for (let y = 0; y < BIG; y++)
        for (let x = 0; x < BIG; x++) {
          if (!shown[y]?.[x]) continue
          const v = tone(x, y)
          // The head flips the cell it is on, as the small icons' rain does.
          if (v && headAt.has(`${x},${y}`)) continue
          if (v === 2) full.push([x, y])
          else if (v === 1) dim.push([x, y])
        }
      setLayers([cellsPath(full), cellsPath(dim), cellsPath(trail), cellsPath(heads.filter(([x, y]) => !tone(x, y)))])
    }, 70)
    return () => {
      clearInterval(timer)
      io?.disconnect()
    }
  }, [art, name])

  return (
    <svg
      ref={ref}
      viewBox={`${offset[0] * SUB} ${offset[1] * SUB} ${BIG} ${BIG}`}
      className={`kp-splash ${className ?? ''}`.trim()}
      style={style}
      fill="currentColor"
      shapeRendering="crispEdges"
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      data-kp-splash={name}
    >
      <path d={layers[2]} opacity={0.14} />
      <path d={layers[1]} opacity={0.45} />
      <path d={layers[0]} />
      <path d={layers[3]} opacity={0.55} />
    </svg>
  )
}
