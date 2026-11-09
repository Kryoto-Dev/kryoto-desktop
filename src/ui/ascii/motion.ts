/**
 * Small moving pictures for what is new in Kryoto Desktop. Most are pixel art
 * on a 64 by 24 grid of square pixels, drawn by `PixelMotion` at a whole number of
 * device pixels per pixel, so every pixel is the same size on any screen.
 *
 * Each scene is a function of `t` in 0..1 that paints one frame. Pixels have
 * three tones: off, dim and full, all in the current text colour. They move in
 * hard steps, as the matrix's glyphs do; nothing glows or fades. Each scene
 * tells its story in the first part of its period and holds the finished
 * picture for the rest, so the loop reads as "it happened", not as a flicker.
 */

export const SCENE_W = 64
export const SCENE_H = 24

export type Scene = {
  period: number
  /** Paint the frame at `t` (0..1) onto a blank canvas. */
  draw: (c: Canvas, t: number) => void
}

const FULL = 2
const DIM = 1

/** A tiny canvas of tones: 0 off, 1 dim, 2 full. */
export class Canvas {
  readonly w = SCENE_W
  readonly h = SCENE_H
  readonly px = new Uint8Array(SCENE_W * SCENE_H)

  set(x: number, y: number, tone = FULL) {
    x = Math.round(x)
    y = Math.round(y)
    if (x >= 0 && x < this.w && y >= 0 && y < this.h) this.px[y * this.w + x] = tone
  }
  get(x: number, y: number) {
    return x >= 0 && x < this.w && y >= 0 && y < this.h ? this.px[y * this.w + x]! : 0
  }
  rect(x: number, y: number, w: number, h: number, tone = FULL) {
    for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) this.set(x + i, y + j, tone)
  }
  hline(x: number, y: number, w: number, tone = FULL) {
    this.rect(x, y, w, 1, tone)
  }
  vline(x: number, y: number, h: number, tone = FULL) {
    this.rect(x, y, 1, h, tone)
  }
  /** An outline with its four corner pixels left out: a pixel-art rounded box. */
  box(x: number, y: number, w: number, h: number, tone = FULL) {
    this.hline(x + 1, y, w - 2, tone)
    this.hline(x + 1, y + h - 1, w - 2, tone)
    this.vline(x, y + 1, h - 2, tone)
    this.vline(x + w - 1, y + 1, h - 2, tone)
  }
  /** A filled box with corners cut by `r` pixels (a stepped rounding). */
  rounded(x: number, y: number, w: number, h: number, r: number, tone = FULL) {
    for (let j = 0; j < h; j++)
      for (let i = 0; i < w; i++) {
        const dx = Math.max(0, r - i, i - (w - 1 - r))
        const dy = Math.max(0, r - j, j - (h - 1 - r))
        if (dx * dx + dy * dy <= r * r) this.set(x + i, y + j, tone)
      }
  }
  /** Rows of `#` (full), `+` (dim), `.` (off: punched out) and ` ` (left alone). */
  stamp(x: number, y: number, rows: readonly string[]) {
    rows.forEach((row, j) =>
      Array.from(row).forEach((ch, i) => {
        if (ch === '#') this.set(x + i, y + j, FULL)
        else if (ch === '+') this.set(x + i, y + j, DIM)
        else if (ch === '.') this.set(x + i, y + j, 0)
      }),
    )
  }
}

/** The story's progress: 0..1 over the first `share` of the period, then held. */
const story = (t: number, share = 0.8) => Math.min(1, Math.max(0, t / share))
/** Slow in and out, so a move starts and lands gently even in steps. */
const ease = (k: number) => (k < 0.5 ? 2 * k * k : 1 - 2 * (1 - k) * (1 - k))

// --- HatchZip --------------------------------------------------------------------

/**
 * A scene in the logo's lettering instead of pixels: block and double-line
 * cells (`cells.ts`), drawn by `SceneMotion` as sharp vectors. `caption` says
 * in words what the picture is showing at `t`.
 */
export type CellScene = {
  kind: 'cells'
  period: number
  frame: (t: number) => readonly string[]
  caption?: (t: number) => string
}

const pad = (s: string, w: number) => (s.length >= w ? s.slice(0, w) : s + ' '.repeat(w - s.length))

/**
 * HatchZip, off and then on, on the same small drive. The game's folder (left)
 * fills one file at a time while a block crosses over from the archive (right);
 * the bar underneath is the drive: the game, the archive, then free space.
 *
 * - Off: the archive stays whole while the game unpacks beside it. The drive
 *   fills up halfway through and the next file has nowhere to go.
 * - On: the archive is read from its end and cut short as each file lands,
 *   so the drive never holds both, and the whole game fits.
 *
 * After hatchzip.dev's own picture of how it works, redrawn in our cells.
 */
export const HATCH: CellScene = {
  kind: 'cells',
  period: 12000,
  caption: (t) => (t < 0.5 ? 'Off: the archive stays, the drive runs out' : 'On: the archive shrinks as the game appears'),
  frame: (t) => {
    const W = 26
    const FILES = 8
    const ARCHIVE = 8
    const DRIVE = 12
    const on = t >= 0.5
    const k = story((t % 0.5) / 0.5)
    // Off, the drive holds the archive and the game at once: it is full after
    // DRIVE - ARCHIVE files, and the next one never lands.
    const room = on ? FILES : DRIVE - ARCHIVE
    const progress = Math.min(k * FILES, room + 0.6)
    const landed = Math.min(FILES, Math.floor(progress))
    const stuck = !on && landed >= room
    const done = landed >= FILES
    const moving = !done && !stuck && k > 0
    const f = progress - Math.floor(progress)
    const blink = Math.floor(t * 24) % 2 === 0
    // On, one cell of archive is cut from its end for each file that lands.
    const inner = on ? ARCHIVE - landed : ARCHIVE
    const slots = (ch: string) => {
      const cells = ch.repeat(landed) + (stuck && blink ? '▒' : '')
      return `║${pad(cells, FILES)}║`
    }
    // The block in transit, right to left across the gap, on the upper row.
    const gap = (row: number) => {
      // Off, the next file waits halfway across with nowhere to land.
      if (stuck && row === 1) return blink ? ' ▪  ' : '    '
      if (!moving || row !== 1) return '    '
      const at = 3 - Math.min(3, Math.floor(f * 4))
      return Array.from({ length: 4 }, (_, i) => (i === at ? '▪' : ' ')).join('')
    }
    // The cell being read is lighter: on, it is the one about to be cut off.
    const archive = (shade: string) => (inner > 0 ? `║${moving && on ? shade.repeat(inner - 1) + '▒' : shade.repeat(inner)}║` : '')
    const edge = (l: string, r: string) => (inner > 0 ? `${l}${'═'.repeat(inner)}${r}` : '')
    // The drive, to scale: the game, then the archive, then what is free.
    const used = landed + inner
    const free = Math.max(0, DRIVE - used)
    const drive = '█'.repeat(landed) + '▓'.repeat(Math.min(inner, DRIVE - landed)) + '░'.repeat(free)
    const full = stuck && blink ? drive.slice(0, -1) + '▒' : drive
    return [
      pad(`╔════════╗    ${edge('╔', '╗')}`, W),
      pad(`${slots('█')}${gap(1)}${archive('▓')}`, W),
      pad(`${slots('▓')}${gap(2)}${archive('▓')}`, W),
      pad(`╚════════╝    ${edge('╚', '╝')}`, W),
      pad('', W),
      pad(`║${full}║`, W),
    ]
  },
}

// --- Bridge ---------------------------------------------------------------------

/**
 * Bringing a library over: the other launcher's shelf (left) empties one game
 * at a time as each crosses the gap to Kryoto's shelf (right). A game Kryoto has lands
 * solid, in Kryoto's own art; one it does not list (the lighter spines) keeps
 * the look it had. The bar underneath is the play time arriving with them.
 * Same cells and pacing as HatchZip.
 */
export const SHELF_MOVE: CellScene = {
  kind: 'cells',
  period: 9000,
  caption: (t) => (story(t, 0.85) < 1 ? 'Moving your games over' : 'Matched games take Kryoto’s art'),
  frame: (t) => {
    const W = 24
    // ▓ a game Kryoto has, ▒ one of the player's own.
    const BOOKS = ['▓', '▒', '▓', '▓', '▒', '▓']
    const N = BOOKS.length
    const k = story(t, 0.85) * N
    const moved = Math.min(N, Math.floor(k))
    const f = k - Math.floor(k)
    const moving = moved < N && k > 0
    // The one in flight has left one shelf and not reached the other.
    const left = BOOKS.slice(0, N - moved - (moving ? 1 : 0)).join('')
    const right = BOOKS.slice(N - moved)
      .reverse()
      .map((b) => (b === '▓' ? '█' : '▒'))
      .join('')
    const gap = (row: number) => {
      if (!moving || row !== 1) return '    '
      const at = Math.min(3, Math.floor(f * 4))
      return Array.from({ length: 4 }, (_, i) => (i === at ? '▪' : ' ')).join('')
    }
    const shelf = (books: string) => `║${pad(books, 8)}║`
    const hours = Math.round((moved / N) * (W - 2))
    return [
      pad('╔════════╗    ╔════════╗', W),
      pad(`${shelf(left)}${gap(1)}${shelf(right)}`, W),
      pad(`${shelf(left)}${gap(2)}${shelf(right)}`, W),
      pad('╚════════╝    ╚════════╝', W),
      pad('', W),
      pad(`║${'█'.repeat(hours)}${'░'.repeat(W - 2 - hours)}║`, W),
    ]
  },
}

/** The same move for games that stay what they are: no art changes hands. */
export const SHELF_SEND: CellScene = { ...SHELF_MOVE, caption: () => 'Moving your games over' }

// --- VR --------------------------------------------------------------------------

/** The hills seen in the lenses: a height per column, repeating as the view pans. */
const HILLS = [1, 2, 3, 4, 5, 5, 4, 3, 2, 2, 3, 4, 4, 3, 2, 1, 1, 2, 3, 3, 2, 1, 1, 1, 2, 2]

/**
 * VR: a headset from the front - the visor, the strap over the top, the
 * straps at the sides, the nose cut-out. Its lenses are on: a range of hills
 * under a night sky, the same world in both eyes, sliding past as your head
 * turns.
 */
export const HEADSET: Scene = {
  period: 5200,
  draw: (c, t) => {
    const vx = 14
    const vy = 4
    const vw = 36
    const vh = 16
    // Straps first, the visor over them.
    c.rect(0, 10, vx + 1, 3, DIM)
    c.rect(vx + vw - 1, 10, c.w - (vx + vw - 1), 3, DIM)
    c.hline(vx + 9, 1, vw - 18, DIM)
    c.hline(vx + 6, 2, 3, DIM)
    c.hline(vx + vw - 9, 2, 3, DIM)
    c.hline(vx + 4, 3, 2, DIM)
    c.hline(vx + vw - 6, 3, 2, DIM)
    // The visor: a dim body with a bright outline, the lenses and the nose
    // cut out of it, so what the lenses show stands out from the shell.
    const lw = 13
    const lh = 9
    const ly = vy + 3
    const lenses = [vx + 3, vx + vw - 3 - lw]
    const roundIn = (x: number, y: number, bx: number, by: number, bw: number, bh: number, r: number) => {
      const i = x - bx
      const j = y - by
      if (i < 0 || j < 0 || i >= bw || j >= bh) return false
      const dx = Math.max(0, r - i, i - (bw - 1 - r))
      const dy = Math.max(0, r - j, j - (bh - 1 - r))
      return dx * dx + dy * dy <= r * r
    }
    const nose = (x: number, y: number) => {
      const depth = y - (vy + vh - 4)
      return depth >= 0 && Math.abs(x - (vx + vw / 2 - 0.5)) <= 1.5 + depth
    }
    const body = (x: number, y: number) =>
      roundIn(x, y, vx, vy, vw, vh, 4) && !lenses.some((lx) => roundIn(x, y, lx, ly, lw, lh, 3)) && !nose(x, y)
    for (let y = vy; y < vy + vh; y++)
      for (let x = vx; x < vx + vw; x++) {
        if (!body(x, y)) continue
        const edge = !body(x - 1, y) || !body(x + 1, y) || !body(x, y - 1) || !body(x, y + 1)
        c.set(x, y, edge ? FULL : DIM)
      }
    // The world in each lens: the lenses switch on top to bottom, then the
    // hills pan past under the sun.
    const on = Math.min(lh, Math.floor((t / 0.12) * lh))
    const pan = Math.floor(t * HILLS.length * 2)
    const inLens = (i: number, j: number) => {
      const dx = Math.max(0, 3 - i, i - (lw - 4))
      const dy = Math.max(0, 3 - j, j - (lh - 4))
      return dx * dx + dy * dy <= 9
    }
    for (const lx of lenses) {
      const draw = (i: number, j: number, tone = FULL) => {
        if (j < on && inLens(i, j)) c.set(lx + i, ly + j, tone)
      }
      // Two stars in the sky.
      draw(3, 1, DIM)
      draw(9, 2, DIM)
      for (let i = 0; i < lw; i++) {
        const h = HILLS[(i + pan) % HILLS.length]!
        // The ground, then the hills on it.
        draw(i, lh - 1, DIM)
        for (let j = 0; j < h; j++) draw(i, lh - 2 - j)
      }
    }
  },
}

// --- Patchbay --------------------------------------------------------------------

const K_MARK = ['#..#..#..#', '#.#..#..#.', '##..#..#..', '#.#.#..#..', '#..#..#...']

/**
 * Patchbay: Kryoto (its K// on a screen) on the left, a server of your own
 * on the right. The plug runs down the cable into the rack, then saves and
 * achievements travel along it both ways and the rack's lights come on, one
 * unit at a time.
 */
export const PATCH: Scene = {
  period: 5600,
  draw: (c, t) => {
    // Kryoto: a screen with K// on it and a stand.
    c.box(2, 3, 19, 14)
    c.hline(3, 5, 17, DIM)
    c.stamp(6, 8, K_MARK)
    c.rect(9, 17, 5, 2)
    c.hline(6, 19, 11)
    // The rack: three units, each with vents and a light.
    const rx = 46
    c.box(rx, 1, 16, 21)
    for (let u = 0; u < 3; u++) {
      const uy = 2 + u * 7
      if (u > 0) c.hline(rx + 1, uy - 1, 14)
      for (let v = 0; v < 3; v++) c.hline(rx + 3, uy + 2 + v, 6, v === 1 ? FULL : DIM)
    }
    // The cable from the screen's side to the rack's middle unit.
    const y = 11
    const from = 21
    const to = rx - 1
    const plug = ease(Math.min(1, t / 0.3))
    const head = Math.round(from + plug * (to - 3 - from))
    c.hline(from, y, head - from)
    c.rect(head, y - 1, 3, 3)
    c.set(head + 3, y)
    if (plug < 1) return
    c.set(to, y)
    const k = (t - 0.3) / 0.7
    const span = to - 4 - from
    // Saves out (full) above the cable, answers back (dim) below it.
    for (let p = 0; p < 3; p++) {
      const out = from + 1 + Math.floor((k * 2.2 * span + p * (span / 3)) % span)
      c.rect(out, y - 2, 2, 1)
      const back = to - 4 - Math.floor((k * 2.2 * span + p * (span / 3) + span / 6) % span)
      c.rect(back, y + 2, 2, 1, DIM)
    }
    // The rack's lights, unit by unit as data reaches them; the screen's cursor blinks.
    const lit = Math.min(3, Math.floor(k * 7))
    for (let u = 0; u < lit; u++) c.rect(rx + 11, 2 + u * 7 + 2, 2, 3)
    if (Math.floor(t * 12) % 2 === 0) c.hline(6, 14, 3)
  },
}

// --- Settings sections -----------------------------------------------------------

/**
 * The built-in ad blocker: a sweep crosses a web page and takes the ad blocks
 * (`▓`) with it, leaving a little dust; the page's own lines stay.
 */
export const ADSWEEP: CellScene = {
  kind: 'cells',
  period: 4200,
  frame: (t) => {
    const PAGE = ['──────  ▓▓▓▓▓▓▓▓', '▓▓▓▓▓▓  ────────', '──────  ────────', '▓▓▓▓▓▓  ▓▓▓▓▓▓▓▓']
    const W = 16
    // From just off the left edge to just past the right: -1 to W + 1.
    const at = Math.floor(ease(story(t, 0.7)) * (W + 2)) - 1
    const rows = PAGE.map((line) =>
      Array.from(line)
        .map((ch, x) => (x === at ? '▐' : ch !== '▓' || x > at ? ch : x === at - 1 ? '░' : ' '))
        .join(''),
    )
    return [`╔${'═'.repeat(W + 2)}╗`, ...rows.map((r) => `║ ${r} ║`), `╚${'═'.repeat(W + 2)}╝`]
  },
}

/**
 * Windows Security exclusions: files on their way into the game's folder.
 * Scanned, each one waits at the gate before it goes in; excluded, they go
 * straight through and the folder fills three times as fast.
 */
export const SCAN_SKIP: CellScene = {
  kind: 'cells',
  period: 6000,
  caption: (t) => (t < 0.5 ? 'Scanned: each file waits its turn' : 'Excluded: files go straight in'),
  frame: (t) => {
    const scanned = t < 0.5
    const k = (t % 0.5) / 0.5
    const SPAN = 10
    const GATE = 5
    // Scanned, a file spends the middle of its trip held at the gate.
    const at = scanned ? (k < 0.3 ? (k / 0.3) * GATE : k < 0.7 ? GATE : GATE + ((k - 0.7) / 0.3) * GATE) : (k * 3 * SPAN) % SPAN
    const x = Math.min(SPAN - 1, Math.floor(at))
    const held = scanned && x === GATE && Math.floor(t * 20) % 2 === 0
    const lane = Array.from({ length: SPAN }, (_, i) => (i === x ? (held ? '▓' : '▪') : scanned && i === GATE ? '│' : '─')).join('')
    const filled = scanned ? 1 : 1 + Math.min(3, Math.floor(k * 3.5))
    return [`${lane} ╔════╗`, `${' '.repeat(SPAN)} ║${'▓'.repeat(filled)}${' '.repeat(4 - filled)}║`, `${' '.repeat(SPAN)} ╚════╝`]
  },
}

/** Sounds: a speaker, its waves stepping out one ring at a time. */
export const SOUND: CellScene = {
  kind: 'cells',
  period: 2400,
  frame: (t) => {
    const n = Math.min(3, Math.floor(story(t, 0.7) * 4))
    const ring = (ch: string) => [0, 1, 2].map((i) => (i < n ? ch : ' ')).join('  ')
    return ['  ▄█ ' + ring('┐'), '▐███ ' + ring('│'), '▐███ ' + ring('│'), '  ▀█ ' + ring('┘')].map((r) => pad(r, 12))
  },
}

/** Downloads: blocks drop one by one into the tray, which fills from the left. */
export const TRAY: CellScene = {
  kind: 'cells',
  period: 4000,
  frame: (t) => {
    const DROPS = 4
    const k = story(t, 0.85) * DROPS
    const landed = Math.min(DROPS, Math.floor(k))
    const fall = landed < DROPS ? Math.min(3, Math.floor((k - landed) * 4)) : -1
    const rows = Array.from({ length: 4 }, (_, row) => pad(row === fall ? '     ██' : '', 12))
    const fill = '▒'.repeat(landed * 2) + ' '.repeat(10 - landed * 2)
    return [...rows, '║          ║', `║${fill}║`, '╚══════════╝']
  },
}

/** Cloud saves: a save leaves the PC along the wire and lands in the cloud. */
export const BACKUP: CellScene = {
  kind: 'cells',
  period: 3200,
  frame: (t) => {
    const span = 8
    const k = story(t, 0.75)
    const at = Math.min(span, Math.floor(k * (span + 1)))
    const arrived = at >= span
    const wire = Array.from({ length: span }, (_, i) => (i === at ? '▪' : '─')).join('')
    return [
      '╔════╗           ▄██▄  ',
      `║▓▓▓▓║ ${wire} ${arrived ? '▄██▓▓██▄' : '▄██████▄'}`,
      '╚════╝          ▀▀▀▀▀▀▀▀',
      ' ▀▀▀▀                   ',
    ].map((r) => pad(r, 24))
  },
}

/** Your phone: a signal goes out to it and comes back to the PC. */
export const PHONE: CellScene = {
  kind: 'cells',
  period: 3000,
  frame: (t) => {
    const span = 8
    const step = Math.floor(t * span * 2) % (span * 2)
    // Out to the phone on the upper wire, back on the lower one.
    const out = step < span ? step : -1
    const back = step >= span ? span - 1 - (step - span) : -1
    const wire = (at: number) => Array.from({ length: span }, (_, i) => (i === at ? '▪' : '─')).join('')
    return [
      '╔══════╗          ╔══╗',
      `║▓▓▓▓▓▓║ ${wire(out)} ║▓▓║`,
      `╚══════╝ ${wire(back)} ║▓▓║`,
      '  ▀▀▀▀            ╚══╝',
    ].map((r) => pad(r, 22))
  },
}
