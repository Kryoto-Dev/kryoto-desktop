import {
  ANIMS, FPS, H, W, animsFor, makeColors, render, toRGBA,
  type Grid, type MascotId, type Mode, type Pose, type RGBA, type ThemeId,
} from '@/pet/engine'

/**
 * A live mascot on a canvas: plays animations, keeps itself busy when nobody
 * is touching it, and reacts to the pointer. The page decides where it goes;
 * this only draws it and runs its moods.
 *
 * Every stage on the page shares one animation frame loop, which skips stages
 * that are off screen or in a hidden tab. With reduced motion (the site's
 * switch or the system's) a stage holds the first frame of whatever it is
 * doing (or the frame the animation names as its `still`) instead of
 * animating it.
 */

export type StageTheme = { theme: ThemeId; mode: Mode }

/** The page's own palette and mode, as stamped on <html>. */
export function pageTheme(): StageTheme {
  const root = document.documentElement
  const p = root.getAttribute('data-palette') as ThemeId | null
  return { theme: p ?? 'monochrome', mode: root.classList.contains('dark') ? 'dark' : 'light' }
}

function reducedMotion() {
  return (
    document.documentElement.getAttribute('data-motion') === 'reduced' ||
    (typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches)
  )
}

const colorCache = new Map<string, Record<string, RGBA>>()
function colors(t: StageTheme) {
  const key = t.theme + t.mode
  let c = colorCache.get(key)
  if (!c) colorCache.set(key, (c = makeColors(t.theme, t.mode)))
  return c
}

const stages = new Set<Stage>()
let looping = false
function loop(now: number) {
  if (!stages.size) {
    looping = false
    return
  }
  if (!document.hidden) for (const s of stages) if (s.visible) s.tick(now / 1000)
  requestAnimationFrame(loop)
}

// Things to do when nobody is touching it.
const IDLE_POOL = ['blink', 'look', 'bored', 'stretch', 'yawn', 'dance', 'sing', 'wave', 'happy', 'think', 'spin', 'nod', 'meditate', 'hiccup', 'sneeze', 'wink']
// Animations calm enough for the eyes to follow the pointer.
const LOOKABLE = new Set(['idle', 'blink', 'bored', 'shy', 'love', 'happy', 'wave', 'sing', 'nod'])
const POKES = ['poke', 'laugh', 'surprised', 'happy', 'wink', 'hiccup', 'tickle', 'jump']
// The sponsor's golden bits (taco break, juggling, shimmer, nap, dance) share
// one rare slot, so the egg is no more common than the taco alone was. A nap
// is no answer to a click.
const GOLDEN = Object.values(ANIMS).filter((a) => a.sponsor).map((a) => a.id)
const GOLDEN_POKES = GOLDEN.filter((id) => id !== 'taconap')
const any = (list: string[]) => list[Math.floor(Math.random() * list.length)]!

/**
 * A pinned routine instead of the idle director (a profile pet the owner set):
 * plays these in turn, never the same twice running when there is a choice,
 * with `rest` seconds of idle between. `hold` stays on through all of it, so
 * the sponsor's pet keeps its gold between tricks.
 */
export type Pin = { anims: string[]; rest: number; hold?: Pose }

type PlayOptions = {
  /** Repeat until something else plays. */
  loop?: boolean
  /** Seconds to keep it up; the animation's own length otherwise. */
  for?: number
  /** What comes after: an animation id, or a function. Idle when absent. */
  then?: string | (() => void)
  progress?: number | (() => number)
  /** Pose fields laid over every frame of this animation, read as it draws. */
  pose?: () => Pose
}

export class Stage {
  readonly canvas: HTMLCanvasElement
  mascot: MascotId
  visible = true
  override: Pose = {}
  /** Plays animations by itself after a while idle, and falls asleep. */
  autoIdle: boolean
  /** Fixed theme; follows the page when unset. */
  theme: StageTheme | null
  onState?: (anim: string) => void
  anim = 'idle'
  /** While true the idle director keeps its hands off (dragging, a transfer). */
  busy = false
  private pin: Pin | null = null
  private ctx: CanvasRenderingContext2D
  private scratch = document.createElement('canvas')
  private t0 = 0
  private opts: PlayOptions = {}
  private lastFrame = -1
  private grid: Grid | null = null
  private look: [number, number] | null = null
  private lastInput = performance.now() / 1000
  private nextIdle = this.lastInput + 6
  private clicks: number[] = []
  private calmPokes = 0
  private last = ''
  private io: IntersectionObserver | null = null
  private cleanup: (() => void)[] = []

  constructor(canvas: HTMLCanvasElement, o: { mascot: MascotId; scale: number; autoIdle?: boolean; theme?: StageTheme; onState?: (anim: string) => void; pin?: Pin | null }) {
    this.canvas = canvas
    this.mascot = o.mascot
    this.autoIdle = o.autoIdle ?? true
    this.theme = o.theme ?? null
    this.onState = o.onState
    canvas.width = W * o.scale
    canvas.height = H * o.scale
    this.ctx = canvas.getContext('2d')!
    this.scratch.width = W
    this.scratch.height = H
    this.play('idle', { loop: true })
    this.setPin(o.pin ?? null)
    if (typeof IntersectionObserver !== 'undefined') {
      this.io = new IntersectionObserver((entries) => {
        for (const e of entries) {
          this.visible = e.isIntersecting
          if (this.visible) this.redraw()
        }
      })
      this.io.observe(canvas)
    }
    // A palette or mode change repaints at once rather than on the next frame.
    const mo = new MutationObserver(() => this.redraw())
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'data-palette', 'data-motion'] })
    this.cleanup.push(() => mo.disconnect())
    stages.add(this)
    if (!looping) {
      looping = true
      requestAnimationFrame(loop)
    }
  }

  destroy() {
    stages.delete(this)
    this.io?.disconnect()
    this.cleanup.forEach((f) => f())
  }

  redraw() {
    this.lastFrame = -1
  }

  setMascot(id: MascotId) {
    this.mascot = id
    this.back()
  }

  /** Pin a routine, or null to hand it back to the idle director. Starts at once. */
  setPin(pin: Pin | null) {
    const now = performance.now() / 1000
    this.pin = pin?.anims.length ? pin : null
    this.back()
    this.nextIdle = this.pin ? 0 : now + 6
    if (this.pin) this.pinned(now)
  }

  play(anim: string, opts: PlayOptions = {}) {
    const a = ANIMS[anim]
    if (!a || (a.only && a.only !== this.mascot)) return
    this.anim = anim
    this.t0 = performance.now() / 1000
    this.opts = opts
    this.redraw()
    this.onState?.(anim)
  }

  back() {
    this.override = {}
    this.busy = false
    this.play('idle', { loop: true })
  }

  /** Mark a touch, so the idle director waits and a sleeper wakes up. */
  touched() {
    const now = performance.now() / 1000
    this.lastInput = now
    this.nextIdle = now + 6 + Math.random() * 6
  }

  tick(now: number) {
    const a = ANIMS[this.anim]!
    // The frame's clock starts before the input that began this animation, so
    // its first frame can fall a moment before it, and the engine's frame
    // tables have nothing at a negative time.
    const t = Math.max(0, now - this.t0)
    const total = this.opts.for ?? (this.opts.loop ? Infinity : a.dur)
    if (t >= total) {
      const then = this.opts.then
      if (typeof then === 'function') then()
      else if (then) this.play(then, { loop: true })
      else this.back()
      return
    }
    const still = reducedMotion()
    if (this.autoIdle && !still) this.idle(now)
    const frame = still ? 0 : Math.floor(t * FPS)
    if (frame === this.lastFrame) return
    this.lastFrame = frame
    const raw = this.opts.progress
    const progress = typeof raw === 'function' ? raw() : raw
    const override: Pose = { ...this.pin?.hold, ...this.opts.pose?.(), ...this.override }
    if (this.look && LOOKABLE.has(this.anim) && !override.look) override.look = this.look
    const at = still ? (a.still ?? 0) : a.once ? Math.min(t, a.dur) : t
    const theme = this.theme ?? pageTheme()
    const grid = render(this.mascot, this.anim, at, { ...theme, progress, override })
    this.grid = grid
    this.scratch.getContext('2d')!.putImageData(new ImageData(toRGBA(grid, colors(theme)), W, H), 0, 0)
    this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height)
    this.ctx.imageSmoothingEnabled = false
    this.ctx.drawImage(this.scratch, 0, 0, this.canvas.width, this.canvas.height)
  }

  private idle(now: number) {
    if (this.busy || this.anim !== 'idle') return
    if (this.pin) return this.pinned(now)
    if (now - this.lastInput > 45) {
      this.play('sleep', { loop: true })
      return
    }
    if (now < this.nextIdle) return
    if (Math.random() < 1 / 40) {
      const gold = any(GOLDEN)
      this.play(gold)
      this.nextIdle = now + ANIMS[gold]!.dur + 8 + Math.random() * 10
      return
    }
    // Kubi's refreeze only follows a melt, which never ends on a puddle.
    const own = animsFor(this.mascot).filter((a) => a.cat === 'own' && a.id !== 'refreeze').map((a) => a.id)
    const pool = [...IDLE_POOL, ...own]
    const pick = pool[Math.floor(Math.random() * pool.length)]!
    const a = ANIMS[pick]!
    if (pick === 'melt') this.play('melt', { then: () => this.play('refreeze') })
    else this.play(pick, { for: a.once ? a.dur : Math.max(a.dur, 2.5) })
    this.nextIdle = now + a.dur + 8 + Math.random() * 10
  }

  private pinned(now: number) {
    const pin = this.pin!
    if (now < this.nextIdle) return
    const own = pin.anims.filter((id) => {
      const a = ANIMS[id]
      return a && (!a.only || a.only === this.mascot)
    })
    const choice = own.length > 1 ? own.filter((id) => id !== this.last) : own
    if (!choice.length) return
    const pick = any(choice)
    this.last = pick
    // Reduced motion never runs the director again, so it holds this pose.
    if (reducedMotion()) return this.play(pick, { loop: true })
    this.play(pick, { for: ANIMS[pick]!.dur })
    // Back to back when there is no rest; the next one starts as this ends.
    this.nextIdle = now + ANIMS[pick]!.dur + (pin.rest ? pin.rest * (0.75 + Math.random() * 0.5) : 0)
  }

  /** Is there a drawn pixel under this point of the page? */
  opaqueAt(clientX: number, clientY: number) {
    if (!this.grid) return false
    const r = this.canvas.getBoundingClientRect()
    const x = Math.floor(((clientX - r.left) / r.width) * W)
    const y = Math.floor(((clientY - r.top) / r.height) * H)
    // A pixel of slack around the body, so the edge is easy to hit.
    for (let dy = -1; dy <= 1; dy++)
      for (let dx = -1; dx <= 1; dx++) {
        const k = this.grid.get(x + dx, y + dy)
        if (k && k !== 'x') return true
      }
    return false
  }

  /** Eyes follow the pointer while it is calm. */
  lookAt(clientX: number, clientY: number) {
    const r = this.canvas.getBoundingClientRect()
    const dx = clientX - (r.left + r.width / 2)
    const dy = clientY - (r.top + r.height * 0.55)
    const look: [number, number] = [Math.abs(dx) > r.width * 0.3 ? Math.sign(dx) : 0, Math.abs(dy) > r.height * 0.35 ? Math.sign(dy) : 0]
    if (String(look) !== String(this.look)) {
      this.look = look
      this.redraw()
    }
  }

  /** A click: a random reaction, getting crankier the more it is clicked. */
  poke() {
    const now = performance.now() / 1000
    const asleep = this.anim === 'sleep' || this.anim === 'curl'
    this.touched()
    if (asleep) return this.play('surprised', { for: 1.6 })
    if (this.busy) return
    this.clicks = this.clicks.filter((c) => now - c < 2.2)
    this.clicks.push(now)
    if (this.clicks.length >= 7) return this.play('dizzy', { for: 3 })
    if (this.clicks.length >= 5) return this.play('angry', { for: 1.8 })
    // The sponsor's gold: a small chance per calm click, certain by the 15th.
    if (++this.calmPokes >= 15 || Math.random() < 1 / 12) {
      this.calmPokes = 0
      return this.play(any(GOLDEN_POKES))
    }
    this.play(POKES[Math.floor(Math.random() * POKES.length)]!, { for: 1.5 })
  }
}

/** Petting: strokes back and forth over the head. Returns a pointermove handler. */
export function petDetector(stage: Stage) {
  let lastX: number | null = null
  let dir = 0
  let strokes: number[] = []
  let stop = 0
  return (clientX: number) => {
    if (lastX !== null) {
      const d = Math.sign(clientX - lastX)
      if (d && d !== dir) {
        dir = d
        const now = performance.now()
        strokes = strokes.filter((s) => now - s < 1400)
        strokes.push(now)
        if (strokes.length >= 4 && stage.anim !== 'pet' && !stage.busy) stage.play('pet', { loop: true })
      }
    }
    lastX = clientX
    stage.touched()
    clearTimeout(stop)
    stop = window.setTimeout(() => {
      if (stage.anim === 'pet') stage.play('love', { for: 1.8 })
      strokes = []
      lastX = null
    }, 700)
  }
}
