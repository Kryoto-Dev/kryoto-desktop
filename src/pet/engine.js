// Kryoto's mascots (Telly, Kubi, Yuki, Blip): pixel art drawn entirely by code.
//
// Every frame is a 48x48 grid of palette keys ('o' outline, 'b' body, ...).
// A theme turns the keys into colours, so one drawing serves every palette
// and both modes. No DOM in here, and plain JavaScript on purpose: the same
// file is copied verbatim into Kryoto Desktop (src/pet/engine.js) and the
// preview renderer (design-previews/mascot), so change all three together.
// Types are in engine.d.ts.

export const W = 48
export const H = 48
export const GROUND = 42 // the row feet stand on
export const FPS = 12 // pixel art moves in steps, not tweens

/* ------------------------------------------------------------------------ */
/* Colour                                                                   */
/* ------------------------------------------------------------------------ */

function oklch(L, C, h) {
  const hr = (h * Math.PI) / 180
  const a = C * Math.cos(hr)
  const b = C * Math.sin(hr)
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ].map((v) => {
    const c = v <= 0.0031308 ? 12.92 * v : 1.055 * Math.pow(v, 1 / 2.4) - 0.055
    return Math.round(Math.min(1, Math.max(0, c)) * 255)
  })
}

// The site's own tokens (kryo.to/app/globals.css), so a preview tile sits on
// exactly the background the mascot would live on.
const MONO_LIGHT = { bg: [0.98, 0, 0], fg: [0.12, 0, 0], muted: [0.48, 0, 0], card: [0.96, 0, 0], border: [0.84, 0, 0] }
export const THEMES = {
  monochrome: {
    label: 'Monochrome', hue: 0, chroma: 0,
    light: MONO_LIGHT,
    dark: { bg: [0.15, 0, 0], fg: [0.96, 0, 0], muted: [0.64, 0, 0], card: [0.19, 0, 0], border: [0.28, 0, 0] },
  },
  oled: {
    label: 'Black', hue: 0, chroma: 0, neon: true,
    light: MONO_LIGHT,
    dark: { bg: [0.02, 0, 0], fg: [0.98, 0, 0], muted: [0.6, 0, 0], card: [0.06, 0, 0], border: [0.18, 0, 0] },
  },
  amber: {
    label: 'Amber', hue: 68, chroma: 1.15,
    light: { bg: [0.97, 0.02, 75], fg: [0.22, 0.08, 65], muted: [0.45, 0.06, 65], card: [0.94, 0.03, 75], border: [0.82, 0.05, 75] },
    dark: { bg: [0.14, 0.03, 65], fg: [0.92, 0.12, 75], muted: [0.7, 0.08, 75], card: [0.18, 0.04, 65], border: [0.28, 0.06, 65] },
  },
  emerald: {
    label: 'Green', hue: 150, chroma: 1,
    light: { bg: [0.97, 0.02, 150], fg: [0.2, 0.06, 150], muted: [0.42, 0.05, 150], card: [0.94, 0.03, 150], border: [0.82, 0.04, 150] },
    dark: { bg: [0.13, 0.03, 150], fg: [0.92, 0.1, 150], muted: [0.68, 0.07, 150], card: [0.17, 0.04, 150], border: [0.26, 0.05, 150] },
  },
  nord: {
    label: 'Slate', hue: 240, chroma: 0.75,
    light: { bg: [0.97, 0.01, 240], fg: [0.18, 0.04, 240], muted: [0.44, 0.04, 240], card: [0.94, 0.02, 240], border: [0.82, 0.03, 240] },
    dark: { bg: [0.16, 0.03, 240], fg: [0.93, 0.03, 240], muted: [0.65, 0.04, 240], card: [0.2, 0.04, 240], border: [0.28, 0.05, 240] },
  },
  sepia: {
    label: 'Sepia', hue: 50, chroma: 0.6,
    light: { bg: [0.95, 0.03, 60], fg: [0.22, 0.05, 50], muted: [0.45, 0.04, 50], card: [0.91, 0.04, 60], border: [0.8, 0.05, 60] },
    dark: { bg: [0.16, 0.02, 50], fg: [0.92, 0.05, 50], muted: [0.66, 0.04, 50], card: [0.2, 0.03, 50], border: [0.28, 0.04, 50] },
  },
  blossom: {
    label: 'Pink', hue: 350, chroma: 1,
    light: { bg: [0.97, 0.02, 350], fg: [0.22, 0.07, 350], muted: [0.46, 0.06, 350], card: [0.94, 0.03, 350], border: [0.83, 0.045, 350] },
    dark: { bg: [0.14, 0.03, 350], fg: [0.93, 0.08, 350], muted: [0.7, 0.06, 350], card: [0.18, 0.04, 350], border: [0.27, 0.05, 350] },
  },
}

export function pageColors(theme, mode) {
  const t = THEMES[theme][mode]
  const css = (v) => `rgb(${oklch(...v).join(' ')})`
  return { bg: css(t.bg), fg: css(t.fg), muted: css(t.muted), card: css(t.card), border: css(t.border) }
}

/** Palette key -> [r, g, b, a] for one theme and mode. */
export function makeColors(theme, mode) {
  const t = THEMES[theme]
  const site = t[mode]
  const hue = t.hue
  const ch = t.chroma
  const c = (L, C = 0, h = hue) => [...oklch(L, C * ch, h), 255]
  let k
  if (t.neon && mode === 'dark') {
    // Black (OLED): the mascot turns into a glowing line drawing.
    k = {
      o: c(0.97), b: c(0.11), B: c(0.05), h: c(0.2), e: c(0.97), w: c(1),
      a: c(0.82), A: c(0.55), k: c(0.95), c: c(0.42), M: c(0.3),
      p: c(0.14), P: c(0.08), s: c(0.03), S: c(0.07), g: c(0.98), G: c(0.45),
      J: c(0.12), j: c(0.06), l: c(0.97),
    }
  } else if (mode === 'light') {
    k = {
      o: c(0.24, 0.05), b: c(0.985, 0.012), B: c(0.87, 0.04), h: c(1), e: c(0.2, 0.05), w: c(1),
      a: c(0.66, 0.14), A: c(0.5, 0.13), k: c(0.85, 0.1), c: c(0.84, 0.09), M: c(0.45, 0.13),
      p: c(0.995), P: c(0.86, 0.02), s: c(0.26, 0.04), S: c(0.3, 0.045), g: c(0.93, 0.1), G: c(0.58, 0.08),
    }
    k.J = k.k
    k.j = k.a
    k.l = k.o
  } else {
    k = {
      o: c(0.09, 0.02), b: c(0.94, 0.025), B: c(0.77, 0.05), h: c(0.995, 0.005), e: c(0.13, 0.03), w: c(1),
      a: c(0.74, 0.14), A: c(0.57, 0.13), k: c(0.89, 0.09), c: c(0.78, 0.1), M: c(0.42, 0.12),
      p: c(0.96), P: c(0.78, 0.02), s: c(0.13, 0.03), S: c(0.165, 0.035), g: c(0.93, 0.11), G: c(0.55, 0.08),
    }
    k.J = k.k
    k.j = k.a
    k.l = k.B
  }
  k.F = [...oklch(...site.fg), 255]
  k.D = [...oklch(...site.muted), 255]
  k.x = [...oklch(...site.fg), mode === 'dark' ? 46 : 34]
  return k
}

/* ------------------------------------------------------------------------ */
/* Pixels                                                                   */
/* ------------------------------------------------------------------------ */

export class Grid {
  constructor(w = W, h = H) {
    this.w = w
    this.h = h
    this.px = new Array(w * h).fill(null)
  }
  set(x, y, k) {
    x = Math.round(x)
    y = Math.round(y)
    if (k && x >= 0 && y >= 0 && x < this.w && y < this.h) this.px[y * this.w + x] = k
  }
  get(x, y) {
    if (x < 0 || y < 0 || x >= this.w || y >= this.h) return null
    return this.px[y * this.w + x]
  }
}

// Shapes are rasterised by testing pixel centres, so a shape centred on
// x = 24 is mirror-symmetric (columns 23 and 24 pair up).
class Mask {
  constructor(w = W, h = H) {
    this.w = w
    this.h = h
    this.m = new Uint8Array(w * h)
  }
  has(x, y) {
    return x >= 0 && y >= 0 && x < this.w && y < this.h && this.m[y * this.w + x] === 1
  }
  put(x, y, v = 1) {
    if (x >= 0 && y >= 0 && x < this.w && y < this.h) this.m[y * this.w + x] = v
  }
  ellipse(cx, cy, rx, ry, v = 1) {
    if (rx <= 0 || ry <= 0) return this
    for (let y = Math.floor(cy - ry); y <= Math.ceil(cy + ry); y++)
      for (let x = Math.floor(cx - rx); x <= Math.ceil(cx + rx); x++) {
        const dx = (x + 0.5 - cx) / rx
        const dy = (y + 0.5 - cy) / ry
        if (dx * dx + dy * dy <= 1) this.put(x, y, v)
      }
    return this
  }
  circle(cx, cy, r, v = 1) {
    return this.ellipse(cx, cy, r, r, v)
  }
  rect(x, y, w, h, v = 1) {
    for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) this.put(Math.round(x) + i, Math.round(y) + j, v)
    return this
  }
  rrect(x, y, w, h, r, v = 1) {
    x = Math.round(x)
    y = Math.round(y)
    for (let j = 0; j < h; j++)
      for (let i = 0; i < w; i++) {
        const px = x + i + 0.5
        const py = y + j + 0.5
        const dx = Math.max(x + r - px, 0, px - (x + w - r))
        const dy = Math.max(y + r - py, 0, py - (y + h - r))
        if (dx * dx + dy * dy <= r * r + 0.3) this.put(x + i, y + j, v)
      }
    return this
  }
  line(x0, y0, x1, y1, r, v = 1) {
    const n = Math.max(1, Math.ceil(Math.hypot(x1 - x0, y1 - y0) * 3))
    for (let i = 0; i <= n; i++) this.circle(x0 + ((x1 - x0) * i) / n, y0 + ((y1 - y0) * i) / n, r, v)
    return this
  }
  poly(pts, v = 1) {
    const xs = pts.map((p) => p[0])
    const ys = pts.map((p) => p[1])
    for (let y = Math.floor(Math.min(...ys)); y <= Math.ceil(Math.max(...ys)); y++)
      for (let x = Math.floor(Math.min(...xs)); x <= Math.ceil(Math.max(...xs)); x++) {
        const px = x + 0.5
        const py = y + 0.5
        let inside = false
        for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
          const [xi, yi] = pts[i]
          const [xj, yj] = pts[j]
          if (yi > py !== yj > py && px < ((xj - xi) * (py - yi)) / (yj - yi) + xi) inside = !inside
        }
        if (inside) this.put(x, y, v)
      }
    return this
  }
  clipBelow(row) {
    for (let y = row + 1; y < this.h; y++) for (let x = 0; x < this.w; x++) this.put(x, y, 0)
    return this
  }
}

/** Paint a mask: a 1px outline around it, then the fill with a shaded rim. */
function fill(g, mask, key, { outline = 'o', shade = null, light = null } = {}) {
  const { w, h } = mask
  if (outline)
    for (let y = 0; y < h; y++)
      for (let x = 0; x < w; x++)
        if (!mask.has(x, y) && (mask.has(x - 1, y) || mask.has(x + 1, y) || mask.has(x, y - 1) || mask.has(x, y + 1)))
          g.set(x, y, outline)
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      if (!mask.has(x, y)) continue
      let k = key
      if (light && !mask.has(x, y - 1)) k = light
      if (shade && (!mask.has(x, y + 2) || (!mask.has(x + 1, y) && !mask.has(x + 1, y + 1)))) k = shade
      g.set(x, y, k)
    }
}

/** Stamp a little bitmap; '.' is clear. `outline` traces it like a mask. */
function sprite(g, x, y, rows, { outline = null, flip = false, map = null } = {}) {
  x = Math.round(x)
  y = Math.round(y)
  const h = rows.length
  const w = rows[0].length
  const at = (i, j) => {
    if (i < 0 || j < 0 || i >= w || j >= h) return null
    const ch = rows[j][flip ? w - 1 - i : i]
    if (ch === '.' || ch === ' ' || ch === undefined) return null
    return (map && map[ch]) || ch
  }
  if (outline)
    for (let j = -1; j <= h; j++)
      for (let i = -1; i <= w; i++)
        if (!at(i, j) && (at(i - 1, j) || at(i + 1, j) || at(i, j - 1) || at(i, j + 1))) g.set(x + i, y + j, outline)
  for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) g.set(x + i, y + j, at(i, j))
}

// 3x5 type for screens, progress numbers and GIF labels.
const FONT_ROWS = {
  A: '.#. #.# ### #.# #.#', B: '##. #.# ##. #.# ##.', C: '.## #.. #.. #.. .##', D: '##. #.# #.# #.# ##.',
  E: '### #.. ##. #.. ###', F: '### #.. ##. #.. #..', G: '.## #.. #.# #.# .##', H: '#.# #.# ### #.# #.#',
  I: '### .#. .#. .#. ###', J: '..# ..# ..# #.# .#.', K: '#.# #.# ##. #.# #.#', L: '#.. #.. #.. #.. ###',
  M: '#.# ### ### #.# #.#', N: '##. #.# #.# #.# #.#', O: '.#. #.# #.# #.# .#.', P: '##. #.# ##. #.. #..',
  Q: '.#. #.# #.# ##. .##', R: '##. #.# ##. #.# #.#', S: '.## #.. .#. ..# ##.', T: '### .#. .#. .#. .#.',
  U: '#.# #.# #.# #.# ###', V: '#.# #.# #.# #.# .#.', W: '#.# #.# ### ### #.#', X: '#.# #.# .#. #.# #.#',
  Y: '#.# #.# .#. .#. .#.', Z: '### ..# .#. #.. ###', 0: '### #.# #.# #.# ###', 1: '.#. ##. .#. .#. ###',
  2: '##. ..# .#. #.. ###', 3: '##. ..# .#. ..# ##.', 4: '#.# #.# ### ..# ..#', 5: '### #.. ##. ..# ##.',
  6: '.## #.. ### #.# ###', 7: '### ..# .#. .#. .#.', 8: '### #.# ### #.# ###', 9: '### #.# ### ..# ##.',
  '%': '#.# ..# .#. #.. #.#', '/': '..# ..# .#. #.. #..', '!': '.#. .#. .#. ... .#.', '?': '##. ..# .#. ... .#.',
  '-': '... ... ### ... ...', '.': '... ... ... ... .#.', ':': '... .#. ... .#. ...', '+': '... .#. ### .#. ...',
  ' ': '... ... ... ... ...', "'": '.#. .#. ... ... ...', '(': '..# .#. .#. .#. ..#', ')': '#.. .#. .#. .#. #..',
}
const FONT = {}
for (const ch in FONT_ROWS) FONT[ch] = FONT_ROWS[ch].replace(/ /g, '')

export function text(g, str, x, y, key) {
  for (const ch of String(str).toUpperCase()) {
    const f = FONT[ch] || FONT['?']
    for (let j = 0; j < 5; j++) for (let i = 0; i < 3; i++) if (f[j * 3 + i] === '#') g.set(x + i, y + j, key)
    x += 4
  }
}
export const textWidth = (str) => String(str).length * 4 - 1

/* ------------------------------------------------------------------------ */
/* Faces                                                                    */
/* ------------------------------------------------------------------------ */

// Eye glyphs are drawn for the left eye; the right eye mirrors them. 'K' is
// the feature colour, 'W' a shine, 'A'/'L' accent, 'C' cheek.
const EYES = {
  open: { rows: ['WK', 'KK', 'KK'] },
  open4: { rows: ['WK', 'KK', 'KK', 'KK'] },
  closed: { rows: ['...', '...', 'K..', '.KK'], off: -1, dy: -1 },
  happy: { rows: ['...', '.K.', 'K.K'], off: -1 },
  squeeze: { rows: ['K.', '.K', 'K.'] },
  wide: { rows: ['WKK', 'KKK', 'KKK', 'KKK'], off: -1, dy: -1 },
  dot: { rows: ['..', '.K', '.K'] },
  x: { rows: ['K.K', '.K.', 'K.K'], off: -1 },
  heart: { rows: ['.A.A.', 'AAAAA', '.AAA.', '..A..'], off: -2, dy: -1 },
  star: { rows: ['.L.', 'LWL', '.L.'], off: -1 },
  spiral: { rows: ['KKK', '..K', 'K.K', 'KKK'], off: -1, dy: -1 },
  sleepy: { rows: ['...', 'KKK', '.KK'], off: -1 },
  line: { rows: ['...', '...', 'KKK'], off: -1 },
  teary: { rows: ['WK', 'KK', 'KK', 'LL'] },
  none: { rows: ['..'] },
}
const BROWS = {
  sad: ['..K', 'KK.'],
  angry: ['KK.', '..K'],
  flat: ['KKK'],
}
const MOUTHS = {
  smile: ['K..K', '.KK.'],
  small: ['KK'],
  flat: ['KKKK'],
  open: ['KKKK', 'KMMK', '.KK.'],
  grin: ['KKKKKK', 'KMMMMK', '.KMMK.', '..KK..'],
  big: ['KKKKKK', 'KMMMMK', 'KMMMMK', '.KKKK.'],
  o: ['.KK.', 'KMMK', '.KK.'],
  frown: ['.KK.', 'K..K'],
  wavy: ['.K.K.K', 'K.K.K.'],
  cat: ['K.KK.K', '.K..K.'],
  tongue: ['KKKK', '.CC.'],
  teeth: ['KKKKKK', 'KWWWWK', 'KKKKKK'],
  yawn: ['.KK.', 'KMMK', 'KMMK', '.KK.'],
  smirk: ['...K', 'KKK.'],
  kiss: ['.K', 'K.', '.K'],
  none: ['..'],
}

function glyph(g, rows, x, y, map, mirror) {
  const w = rows[0].length
  sprite(g, x, y, rows, { flip: mirror, map })
  return w
}

/**
 * Draw both eyes, brows, mouth and cheeks. The face is symmetric about the
 * line `fx`; eyes sit `d` columns either side of it.
 */
function face(g, P, f) {
  const { fx, ey, d, my } = f
  const K = f.K || 'e'
  const map = { K, W: f.W || 'w', A: 'a', L: 'k', C: 'c', M: f.M || 'M' }
  const [lx, ly] = P.look
  const drawEye = (style, side) => {
    const e = EYES[style === 'open' && f.eh === 4 ? 'open4' : style] || EYES.open
    const w = e.rows[0].length
    const off = e.off || 0
    const x0 = side < 0 ? fx - d - 2 : fx + d
    const sx = side < 0 ? x0 + off : x0 + 2 - w - off
    glyph(g, e.rows, sx + lx, ey + (e.dy || 0) + ly, map, side > 0)
  }
  drawEye(P.eyes, -1)
  drawEye(P.eyesR || P.eyes, 1)
  if (P.brow && BROWS[P.brow]) {
    const b = BROWS[P.brow]
    sprite(g, fx - d - 3 + lx, ey - 2 - b.length + 1 + ly, b, { map })
    sprite(g, fx + d + lx, ey - 2 - b.length + 1 + ly, b, { map, flip: true })
  }
  if (f.cheeks !== false && P.eyes !== 'none') {
    const cy = ey + (f.eh || 3) + ly
    const ck = f.cheekKey || 'c'
    const n = P.blush ? 3 : 2
    for (let i = 0; i < n; i++) {
      g.set(fx - d - 3 - i + lx + (n === 3 ? 1 : 0), cy, ck)
      g.set(fx + d + 2 + i + lx - (n === 3 ? 1 : 0), cy, ck)
    }
    if (P.blush) {
      g.set(fx - d - 3 + lx, cy - 1, ck)
      g.set(fx + d + 2 + lx, cy - 1, ck)
    }
  }
  const m = MOUTHS[P.mouth] || MOUTHS.smile
  const mw = m[0].length
  sprite(g, fx - mw / 2 + lx, my + ly, m, { map })
}

/** A stubby limb: `ang` 0 hangs down, 90 points out, 180 points up. */
function arm(g, x, y, ang, len, side, key, r = 1) {
  const a = (ang * Math.PI) / 180
  const hx = x + side * Math.sin(a) * len
  const hy = y + Math.cos(a) * len
  const m = new Mask().line(x, y, hx, hy, r).circle(hx, hy, r + 0.45)
  fill(g, m, key, { shade: null })
  return [hx, hy]
}

const even = (v) => Math.max(2, Math.round(v / 2) * 2)

/* ------------------------------------------------------------------------ */
/* Theme extras: each palette dresses the mascot a little differently      */
/* ------------------------------------------------------------------------ */

export const EXTRAS = {
  monochrome: 'Plain',
  oled: 'Neon line art',
  amber: 'Autumn leaf',
  emerald: 'Sprout',
  nord: 'Winter beanie',
  sepia: 'Top hat',
  blossom: 'Sakura flower',
}

function accessory(g, theme, A) {
  if (!A.hat) return
  const [hx, hy] = A.hat
  const hw = A.hatW || 12
  if (theme === 'emerald') {
    sprite(g, hx - 3, hy - 4, ['aa..aa', 'aaAAaa', '..AA..', '..AA..'].map((r) => r), { outline: 'o', map: { A: 'A' } })
  } else if (theme === 'blossom') {
    const [sx, sy] = A.side
    sprite(g, sx - 2, sy - 2, ['.c.c.', 'ccacc', '.cwc.', 'ccacc', '.c.c.'].map((r, i) => (i === 2 ? '.cwc.' : r)), {
      outline: 'o',
      map: { w: 'k' },
    })
  } else if (theme === 'amber') {
    const [sx, sy] = A.side
    sprite(g, sx - 2, sy - 3, ['..a..', '.aaa.', 'aaAaa', '.aAa.', '..A..', '...A.'], { outline: 'o' })
  } else if (theme === 'nord') {
    const m = new Mask()
    m.ellipse(hx, hy + 1, hw / 2, 5).clipBelow(Math.round(hy))
    fill(g, m, 'A', { shade: null })
    for (let x = Math.round(hx - hw / 2) + 1; x < hx + hw / 2 - 1; x += 3) g.set(x, hy - 3, 'a')
    const cuff = new Mask().rrect(hx - hw / 2 - 1, hy - 1, hw + 2, 3, 1)
    fill(g, cuff, 'h', { shade: null })
    for (let x = Math.round(hx - hw / 2); x < hx + hw / 2; x += 2) g.set(x, hy, 'B')
    const pom = new Mask().circle(hx, hy - 5.5, 2)
    fill(g, pom, 'h', { shade: 'B' })
  } else if (theme === 'sepia') {
    const crown = new Mask().rect(hx - 3, hy - 7, 6, 6)
    fill(g, crown, 'e', { shade: null })
    const brim = new Mask().rect(hx - 5, hy - 1, 10, 1)
    fill(g, brim, 'e', { shade: null })
    for (let x = hx - 3; x < hx + 3; x++) g.set(x, hy - 3, 'a')
    g.set(hx - 2, hy - 6, 'G')
  }
}

/* ------------------------------------------------------------------------ */
/* The four candidates                                                      */
/* ------------------------------------------------------------------------ */

function feet(g, P, xs, bot, w, h, key) {
  xs.forEach((x, i) => {
    const side = i === 0 ? -1 : 1
    const lift = (P.step === side ? 1 : 0) + (P.kick ? (i === 0 ? P.kick : -P.kick) : 0)
    const m = new Mask().rrect(x - w / 2, bot - h + 1 - Math.max(0, lift), w, h, 1)
    fill(g, m, key, { shade: null })
  })
}

// Kubi: an ice cube with a frost shard on top. Square, like the sharp corner
// setting; melts when things go wrong and refreezes when they're fixed.
const kubi = {
  id: 'kubi',
  name: 'Kubi',
  tagline: 'A tiny ice cube. Kryo means cold.',
  pitch:
    'An ice cube with a frost shard on its head. Square, so it fits the sharp corners. It melts into a puddle on an error and freezes back when things work again.',
  shadow: 11,
  draw(g, P) {
    const melt = P.melt || 0
    const bw = even(20 * P.sx * (1 + 0.15 * melt))
    const bh = Math.max(5, Math.round(17 * P.sy * (1 - 0.62 * melt)))
    const legs = melt > 0.2 ? 0 : 2
    const bot = GROUND - legs
    const top = bot - bh + 1
    const L = 24 - bw / 2
    const R = 24 + bw / 2
    if (legs) feet(g, P, [24 - 4.5, 24 + 4.5], GROUND, 4, 3, 'B')
    const ay = top + Math.round(bh * 0.55)
    let hL = [L - 2, ay + 3]
    let hR = [R + 2, ay + 3]
    if (melt < 0.55) {
      hL = arm(g, L + 1.5, ay, P.armL, 4.5, -1, 'b')
      hR = arm(g, R - 1.5, ay, P.armR, 4.5, 1, 'b')
    }
    const m = new Mask().rrect(L, top, bw, bh, 4)
    if (melt) m.ellipse(24, GROUND - 0.5, bw / 2 + 8 * melt, 1 + 2.2 * melt)
    fill(g, m, 'b', { shade: 'B' })
    // ice: a lighter top face and a glint
    for (let x = L + 3; x < R - 3; x++) g.set(x, top + 1, 'h')
    if (bh > 8) {
      g.set(L + 2, top + 3, 'w'); g.set(L + 2, top + 4, 'w'); g.set(L + 3, top + 3, 'w')
      g.set(R - 3, bot - 3, 'h')
      g.set(R - 4, bot - 2, 'h')
    }
    // frost shard
    const shard = ['..k..', '.kwk.', '.kkk.', 'kkAkk']
    if (melt < 0.5) sprite(g, 22, top - 4, shard, { outline: 'o', map: { A: 'a' } })
    const ey = top + Math.max(2, Math.round(bh * 0.37))
    const my = ey + (bh > 10 ? 4 : 3)
    if (P.sx > 0.5) face(g, P, { fx: 24, ey, d: 3, my })
    return {
      top: [24, top - 4], hat: [24, top], hatW: bw - 6, side: [R - 3, top + 1],
      handL: hL, handR: hR, eyeL: [20.5, ey + 3], eyeR: [27.5, ey + 3], mouth: [24, my + 1],
      center: [24, top + bh / 2], w: bw / 2, screen: null,
    }
  },
}

// Telly: a little Teletext TV. Its face is lit pixels on the screen, so the
// screen can turn into a progress bar, an icon or the K// mark.
const telly = {
  id: 'telly',
  name: 'Telly',
  tagline: 'A Teletext TV with a face on its screen.',
  pitch:
    'A small CRT in the same Teletext style as the site font. The face is pixels on its screen, so the screen can turn into a progress bar, a percentage, an icon or static. It has the most room for showing what is happening.',
  shadow: 12,
  draw(g, P, _ctx, t) {
    const bw = even(24 * P.sx)
    const bh = Math.round(18 * P.sy)
    const legH = 3
    const bot = GROUND - legH
    const top = bot - bh + 1
    const L = 24 - bw / 2
    const R = 24 + bw / 2
    const ear = P.ear || 0
    // antennas
    if (P.sx > 0.45) {
      for (const side of [-1, 1]) {
        const tip = [24 + side * (6.5 + (side < 0 ? ear : -ear)), top - 6 + Math.abs(ear) * 0.5]
        const am = new Mask().line(24 + side * 1.5, top, tip[0], tip[1], 0.5)
        fill(g, am, 'l', { outline: null })
        fill(g, new Mask().circle(tip[0], tip[1], 1.5), 'a', {})
      }
    }
    feet(g, P, [24 - 5, 24 + 5], GROUND, 3, legH + 1, 'B')
    const ay = top + Math.round(bh * 0.55)
    const hL = arm(g, L + 1, ay, P.armL, 4.5, -1, 'b', 0.7)
    const hR = arm(g, R - 1, ay, P.armR, 4.5, 1, 'b', 0.7)
    fill(g, new Mask().rrect(L, top, bw, bh, 3), 'b', { shade: 'B' })
    for (let x = L + 3; x < R - 3; x++) g.set(x, top + 1, 'h')
    // screen
    const sL = L + 2
    const sT = top + 2
    const sW = bw - 4
    const sH = bh - 7
    const sm = new Mask().rrect(sL, sT, sW, sH, 2)
    fill(g, sm, 's', { shade: null })
    for (let y = sT + 1; y < sT + sH; y += 2) for (let x = sL; x < sL + sW; x++) if (sm.has(x, y) && sm.has(x, y - 1)) g.set(x, y, 'S')
    // knobs and grille on the bottom strip
    const strip = sT + sH + 1
    if (bw > 12) {
      g.set(R - 4, strip, 'a'); g.set(R - 4, strip + 1, 'A')
      g.set(R - 6, strip, 'A'); g.set(R - 6, strip + 1, 'A')
      for (let x = L + 3; x < L + 9; x += 2) {
        g.set(x, strip, 'B')
        g.set(x, strip + 1, 'B')
      }
    }
    const scr = { x: sL + 1, y: sT + 1, w: sW - 2, h: sH - 2 }
    if (P.sx > 0.5) screenContent(g, P, scr, t)
    g.set(sL + sW - 3, sT + 1, 'G')
    g.set(sL + sW - 2, sT + 2, 'G')
    return {
      top: [24, top - 6], hat: [24, top], hatW: 10, side: [R - 3, top],
      handL: hL, handR: hR, eyeL: [20.5, sT + 6], eyeR: [27.5, sT + 6], mouth: [24, sT + 8],
      center: [24, top + bh / 2], w: bw / 2, screen: true,
    }
  },
}

const ICONS = {
  heart: ['.gg.gg.', 'ggggggg', 'ggggggg', '.ggggg.', '..ggg..', '...g...'],
  up: ['..g..', '.ggg.', 'ggggg', '..g..', '..g..'],
  down: ['..g..', '..g..', 'ggggg', '.ggg.', '..g..'],
  check: ['......g', '.....gg', 'g...gg.', 'gg.gg..', '.ggg...', '..g....'],
  cross: ['g...g', 'gg.gg', '.ggg.', 'gg.gg', 'g...g'],
  lock: ['.ggg.', 'g...g', 'g...g', 'ggggg', 'ggsgg', 'ggsgg', 'ggggg'],
  search: ['.ggg..', 'g...g.', 'g...g.', 'g...g.', '.gggg.', '....gg'],
  cloud: ['..ggg...', '.ggggggg', 'gggggggg', '.gggggg.'],
  note: ['..gg', '..g.', '..g.', 'ggg.', 'gg..'],
  star: ['...g...', '..ggg..', 'ggggggg', '.ggggg.', '.gg.gg.'],
  gamepad: ['.ggggggg.', 'ggsggggsg', 'gsssggsgs', 'ggsggggsg', 'gg.....gg'],
  wrench: ['g.g...', 'ggg...', '.ggg..', '..ggg.', '...ggg', '....gg'],
  bell: ['..g..', '.ggg.', '.ggg.', '.ggg.', 'ggggg', '..g..'],
  envelope: ['ggggggg', 'gg...gg', 'g.g.g.g', 'g..g..g', 'ggggggg'],
  question: ['.gg.', 'g..g', '..g.', '.g..', '....', '.g..'],
  hourglass: ['ggggg', '.ggg.', '..g..', '.g.g.', 'ggggg'],
  trophy: ['ggggggg', 'g.ggg.g', '.ggggg.', '..ggg..', '...g...', '.ggggg.'],
  bolt: ['...gg', '..gg.', '.gggg', '..gg.', '.gg..', 'gg...'],
}
for (const k in ICONS) {
  const w = Math.max(...ICONS[k].map((r) => r.length))
  ICONS[k] = ICONS[k].map((r) => r.replace(/ /g, '.').padEnd(w, '.'))
}

function screenContent(g, P, s, t) {
  const S = P.screen
  const cx = s.x + s.w / 2
  if (!S) {
    face(g, P, { fx: 24, ey: s.y + 2, d: 3, my: s.y + 6, K: 'g', W: 'w', M: 'G', cheekKey: 'G' })
    return
  }
  if (S.mode === 'static') {
    const f = Math.floor(t * FPS)
    for (let y = s.y - 1; y < s.y + s.h + 1; y++)
      for (let x = s.x - 1; x < s.x + s.w + 1; x++) {
        const r = hash(x * 31 + y, f)
        if (g.get(x, y) === 's' || g.get(x, y) === 'S') g.set(x, y, r > 0.66 ? 'g' : r > 0.4 ? 'G' : 's')
      }
    return
  }
  if (S.mode === 'bar') {
    const v = Math.max(0, Math.min(1, S.v))
    const label = `${Math.round(v * 100)}%`
    const icon = S.icon && textWidth(label) + 7 <= s.w
    if (icon) sprite(g, s.x + 1, s.y + 1, ICONS[S.icon])
    text(g, label, Math.round(cx - textWidth(label) / 2 + (icon ? 3 : 0)), s.y + 1, 'g')
    const bw = s.w - 2
    const by = s.y + s.h - 3
    for (let x = 0; x < bw; x++) {
      g.set(s.x + 1 + x, by, 'G')
      g.set(s.x + 1 + x, by + 2, 'G')
    }
    g.set(s.x, by + 1, 'G')
    g.set(s.x + bw + 1, by + 1, 'G')
    for (let x = 0; x < Math.round(v * bw); x++) g.set(s.x + 1 + x, by + 1, 'g')
    return
  }
  if (S.mode === 'icon') {
    const rows = ICONS[S.icon] || ICONS.question
    const map = S.color ? { g: S.color } : null
    sprite(g, Math.round(cx - rows[0].length / 2 + (S.ox || 0)), Math.round(s.y + s.h / 2 - rows.length / 2 + (S.oy || 0)), rows, { map })
    return
  }
  if (S.mode === 'text') {
    const str = S.text
    text(g, str, Math.round(cx - textWidth(str) / 2), Math.round(s.y + s.h / 2 - 2.5), 'g')
    if (S.cursor && Math.floor(t * 3) % 2 === 0) for (let j = 0; j < 5; j++) g.set(Math.round(cx + textWidth(str) / 2 + 2), Math.round(s.y + s.h / 2 - 2.5) + j, 'g')
    return
  }
  if (S.mode === 'wave') {
    for (let i = 0; i < s.w - 2; i++) {
      const hgt = Math.round(Math.abs(Math.sin(i * 0.9 + t * 9) * Math.sin(i * 0.37 + t * 3)) * (s.h / 2 - 1))
      for (let j = -hgt; j <= hgt; j++) g.set(s.x + 1 + i, Math.round(s.y + s.h / 2 - 0.5) + j, j === 0 ? 'g' : 'G')
    }
    return
  }
  if (S.mode === 'spinner') {
    const f = Math.floor(t * FPS)
    const c = [cx, s.y + s.h / 2]
    for (let i = 0; i < 8; i++) {
      const a = (i / 8) * Math.PI * 2
      const lit = (f - i + 80) % 8
      g.set(c[0] + Math.cos(a) * 3.2 - 0.5, c[1] + Math.sin(a) * 3.2 - 0.5, lit === 0 ? 'w' : lit < 3 ? 'g' : 'G')
    }
    return
  }
  if (S.mode === 'dots') {
    const f = Math.floor(t * 6)
    for (let i = 0; i < 3; i++) {
      const up = f % 3 === i ? 1 : 0
      const x = Math.round(cx - 5 + i * 4)
      const y = Math.round(s.y + s.h / 2 - 1 - up)
      g.set(x, y, 'g'); g.set(x + 1, y, 'g'); g.set(x, y + 1, 'g'); g.set(x + 1, y + 1, 'g')
    }
  }
}

// Yuki: an arctic fox kit. The cutest of the four; ears and a big tail do a
// lot of the acting.
const yuki = {
  id: 'yuki',
  name: 'Yuki',
  tagline: 'An arctic fox kit. Yuki is Japanese for snow.',
  pitch:
    'A white arctic fox kit with big ears and a fluffy tail. The cutest of the four. The ears and tail do a lot of the acting: they droop when sad, prick up when surprised, and the tail wags when it is happy.',
  shadow: 10,
  draw(g, P, _ctx, t) {
    const sx = P.sx
    const sy = P.sy
    const tail = P.tail || 0
    const ear = P.ear || 0
    const bodyCy = GROUND - 3.6 * sy
    const headCy = GROUND - 13.5 * sy
    // tail behind, curling up on the right
    const tw = Math.sin(t * 7) * 0.6 * (P.wag ?? 1)
    const tm = new Mask()
      .circle(28.5 * sx + 24 * (1 - sx), bodyCy + 1, 2.6)
      .circle(32 + tail + tw * 0.5, bodyCy - 2.2, 3.2)
      .circle(34 + tail * 1.5 + tw, bodyCy - 6.5, 3)
    const tipY = bodyCy - 7
    fill(g, tm, 'b', { shade: 'B' })
    for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) if (tm.has(x, y) && y < tipY) g.set(x, y, 'k')
    feet(g, P, [24 - 4, 24 + 4], GROUND, 4, 3, 'B')
    const ay = bodyCy - 1
    const hL = arm(g, 24 - 5 * sx, ay, P.armL, 4, -1, 'b', 0.9)
    const hR = arm(g, 24 + 5 * sx, ay, P.armR, 4, 1, 'b', 0.9)
    fill(g, new Mask().ellipse(24, bodyCy, 6.5 * sx, 4.6 * sy), 'b', { shade: 'B' })
    // chest fluff
    g.set(23, bodyCy - 3, 'h'); g.set(24, bodyCy - 3, 'h'); g.set(24, bodyCy - 2, 'h')
    // ears
    const rx = 9.5 * sx
    const ry = 7.6 * sy
    for (const side of [-1, 1]) {
      const base1 = [24 + side * 3, headCy - ry + 2]
      const base2 = [24 + side * (rx - 0.5), headCy - 2.5]
      const tip = [24 + side * (rx - 1 + Math.max(0, ear) * 3), headCy - ry - 5.5 + Math.max(0, ear) * 4]
      const em = new Mask().poly([base1, tip, base2])
      fill(g, em, 'b', { shade: null })
      const inner = new Mask().poly([
        [base1[0] + side * 1.5, base1[1] + 1],
        [tip[0] - side * 0.8, tip[1] + 2.5],
        [base2[0] - side * 1.6, base2[1] - 0.5],
      ])
      for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) if (inner.has(x, y)) g.set(x, y, 'k')
    }
    const hm = new Mask()
      .ellipse(24, headCy, rx, ry)
      .ellipse(24 - rx + 1.5, headCy + 2.5, 2.6, 2.2)
      .ellipse(24 + rx - 1.5, headCy + 2.5, 2.6, 2.2)
    fill(g, hm, 'b', { shade: 'B' })
    // muzzle and nose
    const mz = new Mask().ellipse(24, headCy + 3.6, 4.2, 2.6)
    for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) if (mz.has(x, y) && hm.has(x, y + 1)) g.set(x, y, 'h')
    const ny = Math.round(headCy + 2)
    const ey = Math.round(headCy - 2)
    if (sx > 0.5) {
      face(g, { ...P, mouth: 'none' }, { fx: 24, ey, d: 3, my: ny + 1, cheeks: true })
      const [lx, ly] = P.look
      g.set(23 + lx, ny + ly, 'e'); g.set(24 + lx, ny + ly, 'e')
      const m = MOUTHS[P.mouth === 'smile' ? 'cat' : P.mouth] || MOUTHS.cat
      sprite(g, 24 - m[0].length / 2 + lx, ny + 1 + ly, m, { map: { K: 'e', W: 'w', M: 'M', C: 'c' } })
    }
    return {
      top: [24, headCy - ry - 5], hat: [24, Math.round(headCy - ry + 1)], hatW: 12, side: [24 + rx - 3, headCy - ry + 1],
      handL: hL, handR: hR, eyeL: [20.5, ey + 3], eyeR: [27.5, ey + 3], mouth: [24, ny + 2],
      center: [24, headCy + 3], w: rx, screen: null,
    }
  },
}

// Blip: a frost slime. Squash and stretch is its whole body, so it has the
// widest range of motion.
const blip = {
  id: 'blip',
  name: 'Blip',
  tagline: 'A frost slime that squishes into shapes.',
  pitch:
    'A small slime with a frozen curl on top, in the accent colour of whichever palette is active. Its whole body squashes and stretches, so it can flatten, wobble, stretch tall or melt into a puddle. It has the most bounce.',
  shadow: 12,
  draw(g, P) {
    const rx = 11 * P.sx
    const ry = 9.5 * P.sy
    const base = GROUND
    const cy = base - ry + 2.5
    const top = Math.round(cy - ry)
    const m = new Mask().ellipse(24, cy, rx, ry).clipBelow(base)
    m.ellipse(24, base - 1, rx + 1.2, 2).clipBelow(base)
    // frozen curl on top
    m.circle(24.5, top - 0.2, 1.7).circle(26.6, top - 1.8, 1.15)
    // arms are part of the same goo, so they share one outline
    const ay = cy + 1
    const reach = (ang, side) => {
      const a = (ang * Math.PI) / 180
      return [24 + side * (rx - 1.5) + side * Math.sin(a) * 4.2, ay + Math.cos(a) * 3.5]
    }
    const hL = reach(P.armL, -1)
    const hR = reach(P.armR, 1)
    m.line(24 - rx + 2, ay, hL[0], hL[1], 1.3).line(24 + rx - 2, ay, hR[0], hR[1], 1.3)
    fill(g, m, 'J', { shade: 'j' })
    // shine and bubbles
    const sxl = Math.round(24 - rx * 0.6)
    g.set(sxl, top + 3, 'w'); g.set(sxl, top + 4, 'w'); g.set(sxl + 1, top + 2, 'w'); g.set(sxl + 2, top + 2, 'h')
    g.set(Math.round(24 + rx * 0.55), base - 4, 'h'); g.set(Math.round(24 + rx * 0.55) - 2, base - 3, 'h')
    const ey = Math.round(cy - ry * 0.18)
    if (P.sx > 0.5) face(g, P, { fx: 24, ey, d: 3, my: ey + 4 })
    return {
      top: [24, top - 3], hat: [24, top + 1], hatW: even(rx * 1.4), side: [24 + rx * 0.6, top + 2],
      handL: hL, handR: hR, eyeL: [20.5, ey + 3], eyeR: [27.5, ey + 3], mouth: [24, ey + 5],
      center: [24, cy], w: rx, screen: null,
    }
  },
}

export const MASCOTS = { kubi, telly, yuki, blip }
export const MASCOT_IDS = Object.keys(MASCOTS)

/* ------------------------------------------------------------------------ */
/* Props and particles                                                      */
/* ------------------------------------------------------------------------ */

const SPR = {
  file: ['pppp..', 'ppppP.', 'ppppPP', 'pppppp', 'paaaap', 'pppppp', 'paaapp', 'pppppp'],
  box: ['PPPaaPPP', 'PPPaaPPP', 'oooaaooo', 'pppaappp', 'pppaappp', 'pppppppP', 'PPPPPPPP'],
  up: ['..k..', '.kkk.', 'kkkkk', '.kkk.', '.kkk.'],
  down: ['.kkk.', '.kkk.', 'kkkkk', '.kkk.', '..k..'],
  heart: ['aa.aa', 'akaaa', 'aaaaa', '.aaa.', '..a..'],
  heartS: ['a.a', 'aaa', '.a.'],
  sparkB: ['..k..', '..k..', 'kkwkk', '..k..', '..k..'],
  sparkS: ['.k.', 'kwk', '.k.'],
  zB: ['FFFF', '..F.', '.F..', 'FFFF'],
  zS: ['FFF', '.F.', 'FFF'],
  note: ['..FF', '..F.', '..F.', 'FFF.', 'FF..'],
  q: ['.FF.', 'F..F', '..F.', '.F..', '....', '.F..'],
  ex: ['FF', 'FF', 'FF', 'FF', '..', 'FF'],
  drop: ['.k.', 'kkk', 'kkk', '.k.'],
  anger: ['.a.a.', 'aa.aa', '.....', 'aa.aa', '.a.a.'],
  bulb: ['.kkk.', 'kkwkk', 'kkkkk', 'kkkkk', '.kkk.', '.PPP.', '.PPP.'],
  check: ['..aaa..', '.aaaaw.', 'aaaawaa', 'awawaaa', 'aawaaaa', '.aaaaa.', '..aaa..'],
  cross: ['..aaa..', '.waaaw.', 'aawawaa', 'aaawaaa', 'aawawaa', '.waaaw.', '..aaa..'],
  star: ['...k...', '..kkk..', 'kkkkkkk', '.kkwkk.', '..kkk..', '.kk.kk.', '.k...k.'],
  starS: ['.k.', 'kkk', '.k.'],
  gamepad: ['.AAAAAAAAA.', 'AAeAAAAAkAA', 'AeeeAAAkAwA', 'AAeAAAAAkAA', 'AAA.....AAA'],
  mug: ['eeee..', 'ppppPP', 'pppp.P', 'ppppPP', 'PPPP..'],
  cookie: ['.AAA.', 'AeAAA', 'AAAeA', 'AeAAA', '.AAA.'],
  magnifier: ['.kk....', 'kwkk...', 'kkkk...', '.kkA...', '...AA..', '....AA.', '.....AA'],
  lock: ['..PPP..', '.P...P.', '.P...P.', 'aaaaaaa', 'aaaeaaa', 'aaaeaaa', 'AAAAAAA'],
  envelope: ['PpppppppP', 'pPpppppPp', 'ppPpppPpp', 'pppPaPppp', 'ppppppppp', 'PPPPPPPPP'],
  bubble: ['.ppppppppp.', 'ppppppppppp', 'ppppppppppp', 'ppppppppppp', '.ppppppppp.', '..pp.......', '.p.........'],
  trophy: ['kkkkkkk', 'kkwkkkk', 'kkkkkkk', '.kkkkk.', '..kkk..', '...A...', '..AAA..', '.AAAAA.'],
  wrench: ['P.P....', 'PPP....', '.PP....', '..PP...', '...PP..', '....PP.', '.....PP'],
  cloud: ['....ppp....', '..pppppp...', '.pppppppppp', 'ppppppppppp', 'PPPPPPPPPPP'],
  bell: ['...A...', '..kkk..', '.kkkkk.', '.kwkkk.', '.kkkkk.', 'kkkkkkk', '...A...'],
  hourglass: ['AAAAA', '.kkk.', '..k..', '.p.p.', '.ppp.', 'AAAAA'],
  pat: ['.pppp.', 'pppppp', 'pppppp', 'pppppp', '.p.p..'],
  sign: ['AAAAAAAAAAAAA', 'AAAAAAAAAAAAA', 'AAAAAAAAAAAAA', 'AAAAAAAAAAAAA', 'AAAAAAAAAAAAA', 'AAAAAAAAAAAAA', 'AAAAAAAAAAAAA', '......P......', '......P......'],
}

function hash(a, b = 0) {
  let h = (Math.imul(a | 0, 374761393) + Math.imul(b | 0, 668265263)) | 0
  h = Math.imul(h ^ (h >>> 13), 1274126177)
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296
}

function drawProps(g, P, A, t) {
  const [tx, ty] = A.top
  const f = Math.floor(t * FPS)
  for (const pr of P.props) {
    const pt = pr.t ?? t
    switch (pr.type) {
      case 'zzz':
        for (let i = 0; i < 3; i++) {
          const l = (pt * 0.5 + i / 3) % 1
          const s = l < 0.45 ? SPR.zS : SPR.zB
          sprite(g, tx + 5 + l * 7 + Math.sin(l * 6) * 1.5, ty - l * 11, s)
        }
        break
      case 'hearts':
        for (let i = 0; i < (pr.n || 3); i++) {
          const l = (pt * 0.6 + i / (pr.n || 3)) % 1
          const x = tx + (i % 2 ? 8 : -12) + Math.sin(l * 7 + i) * 2
          sprite(g, x, ty + 6 - l * 14, l < 0.3 ? SPR.heartS : SPR.heart, { outline: 'o' })
        }
        break
      case 'sparkles': {
        const spots = [[-13, 4], [12, 0], [-9, -6], [14, 12], [-15, 16], [9, -8]]
        spots.forEach(([sx, sy], i) => {
          const ph = (f + i * 3) % 8
          if (ph < 5) sprite(g, A.center[0] + sx - 2, A.center[1] + sy - 2 - (pr.rise ? ph : 0), ph === 2 ? SPR.sparkB : SPR.sparkS, { map: { k: pr.key || 'k' } })
        })
        break
      }
      case 'notes':
        for (let i = 0; i < 2; i++) {
          const l = (pt * 0.55 + i / 2) % 1
          sprite(g, tx + (i ? 9 : -14) + Math.sin(l * 9) * 1.5, ty + 8 - l * 14, SPR.note)
        }
        break
      case 'q':
        sprite(g, tx + 8, ty - 3 + (Math.floor(pt * 3) % 2), SPR.q)
        break
      case '!':
        sprite(g, tx + 8, ty - 4 - (Math.floor(pt * 6) % 2), SPR.ex)
        break
      case 'sweat': {
        const l = (pt * 0.8) % 1
        sprite(g, A.center[0] + A.w - 1, A.top[1] + 8 + l * 4, SPR.drop, { outline: 'o' })
        break
      }
      case 'tears':
        for (const [ex, ey] of [A.eyeL, A.eyeR]) {
          for (let i = 0; i < 2; i++) {
            const l = (pt * 1.6 + i * 0.5) % 1
            const side = ex < 24 + P.dx ? -1 : 1
            g.set(ex + side * (1 + l * 3), ey + l * 7, 'k')
            g.set(ex + side * (1 + l * 3), ey + l * 7 + 1, 'k')
          }
        }
        break
      case 'anger': {
        const big = Math.floor(pt * 4) % 2
        sprite(g, A.center[0] + A.w - 4 - big, A.top[1] + 3 - big, big ? SPR.anger : ['a.a', '...', 'a.a'])
        break
      }
      case 'steam':
        for (let i = 0; i < 3; i++) {
          const l = (pt * 0.9 + i / 3) % 1
          const x = (pr.x ?? (i % 2 ? A.center[0] + A.w : A.center[0] - A.w)) + Math.sin(l * 8 + i) * 1.5
          const y = (pr.y ?? A.top[1] + 4) - l * 8
          g.set(x, y, 'D'); if (l < 0.6) g.set(x + 1, y, 'D')
        }
        break
      case 'bulb': {
        sprite(g, tx - 2, ty - 9, SPR.bulb, { outline: 'o' })
        if (f % 4 < 2) for (const [rx, ry] of [[-5, -7], [5, -7], [-4, -11], [4, -11], [0, -13]]) g.set(tx + rx, ty + ry, 'k')
        break
      }
      case 'stars':
        for (let i = 0; i < 3; i++) {
          const a = pt * 4 + (i * Math.PI * 2) / 3
          sprite(g, tx + Math.cos(a) * 9 - 1, ty + 4 + Math.sin(a) * 2.5 - 1, SPR.starS, { outline: Math.sin(a) > 0 ? 'o' : null })
        }
        break
      case 'confetti':
        for (let i = 0; i < 16; i++) {
          const sp = 0.5 + hash(i, 7) * 0.6
          const l = (pt * sp * 0.6 + hash(i, 3)) % 1
          const x = 3 + hash(i, 1) * 42 + Math.sin(l * 10 + i) * 2
          const y = -2 + l * 46
          const key = ['a', 'k', 'c', 'F', 'A'][i % 5]
          g.set(x, y, key)
          if ((f + i) % 3) g.set(x + 1, y, key)
        }
        break
      case 'snow':
        for (let i = 0; i < 10; i++) {
          const l = (pt * (0.25 + hash(i, 2) * 0.2) + hash(i, 5)) % 1
          const x = 2 + hash(i, 9) * 44 + Math.sin(l * 6 + i) * 2
          const y = l * 44
          if (i % 3 === 0) sprite(g, x - 1, y - 1, ['.D.', 'DkD', '.D.'])
          else g.set(x, y, 'D')
        }
        break
      case 'dust': {
        const dir = pr.dir || 0
        for (let i = 0; i < 3; i++) {
          const l = (pt * 2 + i / 3) % 1
          const x = 24 + P.dx - dir * (6 + l * 6) + (dir === 0 ? (i - 1) * 8 : 0)
          if (l < 0.8) sprite(g, x - 1, GROUND - l * 3, l < 0.4 ? ['DD', 'DD'] : ['D.D', '.D.'])
        }
        break
      }
      case 'speed':
        for (let i = 0; i < 4; i++) {
          const y = 22 + i * 5
          const len = 3 + ((f + i * 2) % 4)
          const x = 24 + P.dx - (pr.dir || 1) * (14 + (i % 2) * 3)
          for (let j = 0; j < len; j++) g.set(x - (pr.dir || 1) * j, y, 'D')
        }
        break
      case 'file': {
        const [x, y] = pr.at === 'hand' ? A.handR : [(A.handL[0] + A.handR[0]) / 2, Math.min(A.handL[1], A.handR[1]) - 9]
        sprite(g, x - 3, y + (pr.bob ? Math.floor(pt * 6) % 2 : 0), SPR.file, { outline: 'o' })
        break
      }
      case 'box': {
        const [x, y] = pr.pos || [(A.handL[0] + A.handR[0]) / 2, Math.min(A.handL[1], A.handR[1]) - 3]
        sprite(g, x - 4, y, SPR.box, { outline: 'o' })
        break
      }
      case 'arrows':
        for (let i = 0; i < 2; i++) {
          const l = (pt * 1.2 + i / 2) % 1
          const up = pr.dir !== 'down'
          const x = tx + (i ? 10 : -14)
          const y = up ? ty + 14 - l * 14 : ty - 6 + l * 14
          sprite(g, x, y, up ? SPR.up : SPR.down, { outline: 'o' })
        }
        break
      case 'bar': {
        if (A.screen) break
        const v = Math.max(0, Math.min(1, pr.v))
        const bw = 22
        const x0 = Math.round(24 + P.dx - bw / 2)
        const y0 = Math.max(1, Math.round(pr.y ?? ty - 4))
        const m = new Mask().rect(x0, y0, bw, 3)
        fill(g, m, 'p', { shade: null })
        for (let x = 0; x < Math.round(v * bw); x++) for (let y = 0; y < 3; y++) g.set(x0 + x, y0 + y, y === 0 ? 'k' : 'a')
        if (pr.label !== false) {
          const label = `${Math.round(v * 100)}%`
          const fits = x0 + bw + 2 + textWidth(label) < W
          text(g, label, fits ? x0 + bw + 2 : Math.max(0, x0 - textWidth(label) - 2), y0 - 1, 'F')
        }
        break
      }
      case 'check':
      case 'cross': {
        const pop = Math.min(1, pt / 0.25)
        const [x, y] = [A.center[0] + A.w + 1, A.top[1] - 1]
        if (pop < 1) sprite(g, x + 1, y + 1, pr.type === 'check' ? ['.aaa.', 'aaaaw', 'awawa', 'aawaa', '.aaa.'] : ['.aaa.', 'wawaw', 'aawaa', 'wawaw', '.aaa.'], { outline: 'o' })
        else sprite(g, x, y, SPR[pr.type], { outline: 'o' })
        break
      }
      case 'gamepad': {
        const x = (A.handL[0] + A.handR[0]) / 2
        const y = Math.max(A.handL[1], A.handR[1]) - 3
        sprite(g, x - 5.5, y + (f % 4 === 0 ? 1 : 0), SPR.gamepad, { outline: 'o' })
        break
      }
      case 'mug': {
        const [x, y] = pr.pos || A.handR
        sprite(g, x - 2, y - 4, SPR.mug, { outline: 'o' })
        if (pr.steam !== false) for (let i = 0; i < 2; i++) { const l = (pt + i * 0.5) % 1; g.set(x - 1 + i * 2 + Math.sin(l * 9), y - 6 - l * 5, 'D') }
        break
      }
      case 'cookie': {
        const [x, y] = A.mouth
        const rows = SPR.cookie.map((r) => r.split('').map((c, i) => (i >= 5 - (pr.bites || 0) * 1.25 ? '.' : c)).join(''))
        if ((pr.bites || 0) < 4) sprite(g, x + 3, y - 2, rows, { outline: 'o' })
        if (pr.crumbs) for (let i = 0; i < 3; i++) { const l = (pt * 1.5 + i / 3) % 1; g.set(x + 3 + i * 2, y + 3 + l * 9, 'A') }
        break
      }
      case 'magnifier': {
        const [x, y] = A.handR
        sprite(g, x - 2, y - 6, SPR.magnifier, { outline: 'o', flip: !!pr.flip })
        break
      }
      case 'lock': {
        const x = (A.handL[0] + A.handR[0]) / 2
        const y = Math.min(A.handL[1], A.handR[1]) - 5
        sprite(g, x - 3.5, y, SPR.lock, { outline: 'o' })
        break
      }
      case 'envelope': {
        const drop = Math.min(1, pt / 0.4)
        const [x, y] = pr.held ? [(A.handL[0] + A.handR[0]) / 2, Math.min(A.handL[1], A.handR[1]) - 7] : [tx, ty - 9 + (1 - drop) * -14]
        sprite(g, x - 4.5, y, SPR.envelope, { outline: 'o' })
        break
      }
      case 'bubble':
      case 'heartBubble':
      case 'hiBubble': {
        const x = Math.round(Math.min(36, tx + 4))
        const y = Math.max(0, Math.round(ty - 6))
        sprite(g, x, y, SPR.bubble, { outline: 'o', flip: false })
        if (pr.type === 'bubble') for (let i = 0; i < 3; i++) g.set(x + 3 + i * 2, y + 2 - (Math.floor(pt * 6) % 3 === i ? 1 : 0), 'e')
        if (pr.type === 'heartBubble') sprite(g, x + 3, y + 1, ['aa.aa'.slice(0, 5), 'aaaaa', '.aaa.'].map((r) => r))
        if (pr.type === 'hiBubble') text(g, 'HI', x + 2, y, 'e')
        break
      }
      case 'trophy': {
        const x = (A.handL[0] + A.handR[0]) / 2
        const y = Math.min(A.handL[1], A.handR[1]) - 9
        sprite(g, x - 3.5, y, SPR.trophy, { outline: 'o' })
        break
      }
      case 'wrench': {
        const [x, y] = A.handR
        sprite(g, x - 1, y - 6, SPR.wrench, { outline: 'o', flip: true })
        if (pr.sparks) for (const [sx, sy] of [[-3, 2], [2, 4], [-1, 6]]) if ((f + sx) % 3 === 0) sprite(g, A.center[0] + A.w + 2 + sx, A.center[1] + sy, SPR.sparkS)
        break
      }
      case 'cloud': {
        const x = 24 + P.dx
        const y = Math.max(0, ty - 10)
        sprite(g, x - 5.5, y, SPR.cloud, { outline: 'o' })
        if (pr.x) sprite(g, x - 2, y + 1, ['e.e', '.e.', 'e.e'])
        if (pr.sync) {
          const s = Math.floor(pt * 4) % 2
          sprite(g, x - 2.5, y + 1, s ? ['.e.', 'eee'] : ['eee', '.e.'])
        }
        break
      }
      case 'bell': {
        const sway = [0, 1, 0, -1][Math.floor(pt * 10) % 4]
        sprite(g, tx - 3 + sway, ty - 9, SPR.bell, { outline: 'o' })
        if (f % 4 < 2) { g.set(tx - 6, ty - 7, 'F'); g.set(tx + 6, ty - 7, 'F'); g.set(tx - 7, ty - 5, 'F'); g.set(tx + 7, ty - 5, 'F') }
        break
      }
      case 'waves': {
        const [x, y] = A.mouth
        const n = Math.floor(pt * 6) % 3
        for (let i = 0; i <= n; i++) for (let j = -1 - i; j <= 1 + i; j++) g.set(x + A.w - 2 + i * 2 + (Math.abs(j) === 1 + i ? -1 : 0), y - 1 + j, 'F')
        break
      }
      case 'hourglass': {
        const flip = Math.floor(pt / 1.6) % 2
        const rows = flip ? [...SPR.hourglass].reverse() : SPR.hourglass
        sprite(g, A.center[0] - A.w - 9, A.center[1] - 3, rows, { outline: 'o' })
        break
      }
      case 'spinner': {
        const c = [tx, ty - 3]
        for (let i = 0; i < 8; i++) {
          const a = (i / 8) * Math.PI * 2 - Math.PI / 2
          const lit = (f - i + 80) % 8
          const k = lit === 0 ? 'F' : lit < 3 ? 'D' : null
          if (k) { g.set(c[0] + Math.cos(a) * 4 - 0.5, c[1] + Math.sin(a) * 4 - 0.5, k) }
        }
        break
      }
      case 'pat': {
        const sw = [-4, -2, 0, 2, 4, 2, 0, -2][Math.floor(pt * 8) % 8]
        sprite(g, tx - 3 + sw, A.top[1] + 1 + (Math.abs(sw) < 2 ? 2 : 0), SPR.pat, { outline: 'o' })
        break
      }
      case 'drips':
        for (let i = 0; i < 3; i++) {
          const l = (pt * 1.2 + i / 3) % 1
          const x = A.center[0] + (i - 1) * (A.w - 2)
          g.set(x, A.center[1] + l * 10, 'k')
          if (l > 0.2) g.set(x, A.center[1] + l * 10 - 1, 'k')
        }
        break
      case 'spray':
        for (let i = 0; i < 7; i++) {
          const l = Math.min(1, pt * 2)
          g.set(A.mouth[0] + 3 + l * (4 + i * 1.5), A.mouth[1] - 3 + (i - 3) * l * 1.3, 'D')
        }
        break
      case 'bubbleS': {
        const l = Math.min(1, pt * 1.5)
        sprite(g, tx + 6, ty + 4 - l * 8, l < 0.5 ? ['.D.', 'D.D', '.D.'] : ['.DD.', 'D..D', 'D..D', '.DD.'])
        break
      }
      case 'trail':
        for (let i = 0; i < 4; i++) sprite(g, 24 + P.dx + (i - 1.5) * 5, GROUND - 2 + i * 0 + ((f + i) % 2), SPR.sparkS)
        break
      case 'sign': {
        sprite(g, A.handR[0] - 2, A.handR[1] - 12, SPR.sign, { outline: 'o' })
        text(g, pr.text || 'K//', A.handR[0] + 0.5, A.handR[1] - 11, 'w')
        break
      }
      case 'text':
        text(g, pr.text, Math.round(pr.x ?? tx + 7), Math.round(pr.y ?? ty - 4), 'F')
        break
    }
  }
}

/* ------------------------------------------------------------------------ */
/* Animations                                                               */
/* ------------------------------------------------------------------------ */

export const BASE = {
  dx: 0, dy: 0, sx: 1, sy: 1, lean: 0, flip: false,
  eyes: 'open', eyesR: null, look: [0, 0], brow: null, mouth: 'smile', blush: false,
  armL: 14, armR: 14, step: 0, tail: 0, ear: 0, wag: 1, melt: 0, screen: null, props: [], clip: false,
}

export const CATEGORIES = [
  { id: 'idle', label: 'Idle & moods' },
  { id: 'move', label: 'Moving around' },
  { id: 'play', label: 'Interactions' },
  { id: 'ctx', label: 'In context (uploads, downloads, chat...)' },
  { id: 'own', label: 'Only this mascot' },
]

export const ANIMS = {}
function def(id, label, cat, dur, fn, extra = {}) {
  ANIMS[id] = { id, label, cat, dur, fn, ...extra }
}

const hop = (p) => Math.sin(Math.min(Math.max(p, 0), 1) * Math.PI)
const pick = (t, arr, fps = 6) => arr[Math.floor(t * fps) % arr.length]
const blinkAt = (t, per = 3.3, off = 0) => (t + off) % per > per - 0.17
const tri = (t, per) => {
  const x = (t / per) % 1
  return x < 0.5 ? x * 2 : 2 - x * 2
}
const lerp = (a, b, p) => a + (b - a) * Math.min(1, Math.max(0, p))
const open = (t, per = 3.3, off = 0) => (blinkAt(t, per, off) ? 'closed' : 'open')

// --- idle & moods
def('idle', 'Idle', 'idle', 4, (t) => ({ sy: t % 2 < 1 ? 1 : 0.95, eyes: open(t, 4, 0.6) }))
def('blink', 'Blink', 'idle', 1.6, (t) => ({ eyes: (t > 0.5 && t < 0.62) || (t > 0.78 && t < 0.9) ? 'closed' : 'open' }))
def('look', 'Look around', 'idle', 4.8, (t) => {
  const look = pick(t, [[0, 0], [-1, 0], [-1, 0], [-1, 0], [1, 0], [1, 0], [1, 0], [0, -1]], 1 / 0.6)
  return { look, lean: look[0], eyes: open(t, 2.4, 0.2), mouth: look[1] < 0 ? 'o' : 'smile' }
})
def('bounce', 'Bounce', 'idle', 0.9, (t) => {
  const p = t / 0.9
  const h = hop(p)
  const land = p < 0.1 || p > 0.9
  return {
    dy: Math.round(6 * h), sx: land ? 1.12 : h > 0.4 ? 0.94 : 1, sy: land ? 0.86 : h > 0.4 ? 1.06 : 1,
    armL: 20 + 140 * h, armR: 20 + 140 * h, eyes: h > 0.5 ? 'happy' : 'open', mouth: h > 0.3 ? 'open' : 'smile', ear: h > 0.5 ? -1 : 0,
  }
})
def('wave', 'Wave', 'idle', 1.6, (t) => ({
  armR: pick(t, [155, 115], 6), eyes: t > 0.3 && t < 1.3 ? 'happy' : 'open', mouth: 'open', lean: pick(t, [0, 1], 3),
  screen: { mode: 'text', text: 'HI!' },
}))
def('sleep', 'Sleep', 'idle', 4, (t) => ({
  eyes: 'closed', mouth: t % 2 < 1 ? 'small' : 'o', sy: t % 2 < 1 ? 1 : 0.94, lean: 1, look: [0, 1],
  armL: 4, armR: 4, ear: 1, wag: 0, props: [{ type: 'zzz' }],
  screen: { mode: 'text', text: 'ZZZ' },
}))
def('yawn', 'Yawn', 'idle', 3, (t) => {
  if (t < 0.6) return { eyes: 'sleepy' }
  if (t < 1.8) return { eyes: 'squeeze', mouth: 'yawn', armL: 170, armR: 170, sy: 1.06, sx: 0.95, ear: -1 }
  if (t < 2.2) return { eyes: 'closed', mouth: 'small' }
  return { eyes: 'sleepy', mouth: 'small' }
})
def('stretch', 'Stretch', 'idle', 2.4, (t) => ({
  lean: pick(t, [0, -2, -3, -2, 0, 2, 3, 2], 3.3), armL: 172, armR: 172, eyes: 'squeeze', mouth: 'o', sy: 1.06, sx: 0.96,
}))
def('bored', 'Bored', 'idle', 3.6, (t) => ({
  eyes: t > 2.4 && t < 3 ? 'line' : 'sleepy', look: t > 2.4 && t < 3 ? [0, -1] : [-1, 0], mouth: 'flat', step: pick(t, [0, 1], 4), wag: 0.2,
  screen: { mode: 'text', text: '...' },
}))
def('dance', 'Dance', 'idle', 2, (t) => {
  const b = Math.floor(t * 4) % 4
  return {
    lean: [-2, 0, 2, 0][b], dy: b % 2, armL: [160, 30, 160, 30][b], armR: [30, 160, 30, 160][b],
    eyes: 'happy', mouth: 'open', step: [-1, 0, 1, 0][b], props: [{ type: 'notes' }], tail: [-1, 0, 1, 0][b],
  }
})
def('spin', 'Spin', 'idle', 1.4, (t) => {
  const k = Math.floor(t * 8)
  const w = [1, 0.7, 0.3, 0.7, 1, 0.7, 0.3, 0.7, 1, 1, 1, 1][k] ?? 1
  return { sx: w, flip: k >= 3 && k < 7, eyes: k > 8 ? 'happy' : 'open', mouth: 'open', armL: 60, armR: 60 }
})
def('happy', 'Happy', 'idle', 1.2, (t) => ({
  dy: Math.round(3 * hop((t % 0.6) / 0.6)), eyes: 'happy', mouth: 'open', armL: pick(t, [150, 120], 5), armR: pick(t, [120, 150], 5),
  props: [{ type: 'sparkles' }], ear: -1, wag: 2,
}))
def('love', 'In love', 'idle', 2.4, (t) => ({
  eyes: 'heart', blush: true, mouth: 'smile', lean: pick(t, [-1, -1, 0, 1, 1, 0], 3), armL: 4, armR: 4,
  props: [{ type: 'hearts' }], screen: { mode: 'icon', icon: 'heart', color: 'c', oy: Math.floor(t * 3) % 2 },
}))
def('laugh', 'Laugh', 'idle', 1.6, (t) => ({
  eyes: 'squeeze', mouth: pick(t, ['grin', 'open'], 6), dy: pick(t, [0, 1], 8), armL: 40, armR: 40, lean: pick(t, [0, 1], 4),
  props: [{ type: 'text', text: pick(t, ['HA', 'HA'], 2), y: 4 + (Math.floor(t * 4) % 2) }], wag: 2,
}))
def('shy', 'Shy', 'idle', 3, (t) => ({
  look: t < 1.5 ? [0, 1] : [-1, 1], eyes: 'happy', blush: true, mouth: 'small', lean: pick(t, [-1, 0, 1, 0], 2),
  armL: 0, armR: 0, step: pick(t, [0, -1, 0, 1], 3), ear: 1,
}))
def('wink', 'Wink', 'idle', 2, (t) => {
  const w = t > 0.5 && t < 1.5
  return {
    eyesR: w ? 'happy' : null, mouth: w ? 'tongue' : 'smile', armR: w ? 150 : 14,
    props: w ? [{ type: 'sparkles' }] : [],
  }
})
def('think', 'Thinking', 'idle', 3.2, (t) => ({
  look: [-1, -1], armR: 145, mouth: pick(t, ['smirk', 'flat'], 1), eyes: open(t, 1.6, 0.4), lean: -1,
  props: [{ type: 'q' }], screen: { mode: 'icon', icon: 'question' },
}))
def('idea', 'Got an idea', 'idle', 2.4, (t) => {
  if (t < 1.1) return { look: [-1, -1], armR: 145, mouth: 'flat', eyes: open(t, 1, 0.5), screen: { mode: 'dots' } }
  return {
    eyes: 'wide', mouth: 'open', dy: t < 1.5 ? Math.round(4 * hop((t - 1.1) / 0.4)) : 0, armR: 175, armL: 30, ear: -1,
    props: [{ type: 'bulb' }], screen: { mode: 'icon', icon: 'bolt' },
  }
})
def('surprised', 'Surprised', 'idle', 2, (t) => {
  if (t < 0.4) return {}
  return {
    eyes: t > 1.5 ? 'open' : 'wide', mouth: 'o', dy: t < 0.75 ? Math.round(4 * hop((t - 0.4) / 0.35)) : 0, armL: 120, armR: 120, ear: -1,
    props: [{ type: '!' }], screen: { mode: 'text', text: '!!' },
  }
})
def('scared', 'Scared', 'idle', 1.6, (t) => ({
  dx: pick(t, [-1, 1], 12), eyes: 'wide', brow: 'sad', mouth: 'wavy', sy: 0.94, armL: 155, armR: 155, ear: 1, wag: 0,
  props: [{ type: 'sweat' }],
}))
def('sad', 'Sad', 'idle', 3.6, (t) => ({
  eyes: t > 2.2 && t < 2.6 ? 'closed' : 'teary', brow: 'sad', look: [0, 1], mouth: 'frown', sy: 0.96, armL: 0, armR: 0, ear: 1, wag: 0,
  screen: { mode: 'text', text: ':(' },
}))
def('cry', 'Cry', 'idle', 1.6, (t) => ({
  eyes: 'squeeze', mouth: pick(t, ['big', 'open'], 4), dy: pick(t, [0, 1], 6), armL: 150, armR: 150, ear: 1, wag: 0,
  props: [{ type: 'tears' }],
}))
def('angry', 'Angry', 'idle', 1.6, (t) => ({
  eyes: 'open', brow: 'angry', mouth: 'teeth', dx: pick(t, [-1, 1], 10), armL: 60, armR: 60, step: pick(t, [1, 0, -1, 0], 6), ear: 1,
  props: [{ type: 'anger' }, { type: 'steam' }], screen: { mode: 'text', text: 'GRR' },
}))
def('dizzy', 'Dizzy', 'idle', 2.4, (t) => ({
  eyes: 'spiral', mouth: 'wavy', lean: Math.round(3 * Math.sin((t / 1.2) * Math.PI * 2)), dx: Math.round(Math.sin((t / 1.2) * Math.PI * 2 + 1.5)),
  armL: 70, armR: 70, props: [{ type: 'stars' }], ear: 1,
}))
def('sneeze', 'Sneeze', 'idle', 3, (t) => {
  if (t < 0.6) return { eyes: 'sleepy', mouth: 'o', lean: -1 }
  if (t < 1.4) return { eyes: 'squeeze', mouth: 'yawn', lean: -2, sy: 1.04, ear: -1 }
  if (t < 1.8) return { eyes: 'squeeze', mouth: 'big', lean: 3, dx: 1, sx: 1.08, sy: 0.92, props: [{ type: 'spray', t: t - 1.4 }], screen: { mode: 'text', text: 'CHOO' } }
  return { eyes: 'happy', mouth: 'smile', blush: true }
})
def('hiccup', 'Hiccup', 'idle', 1.5, (t) => {
  const j = t > 0.6 && t < 0.8
  return {
    dy: j ? 2 : 0, eyes: j ? 'wide' : 'open', mouth: j ? 'o' : 'small', blush: true,
    props: t > 0.6 ? [{ type: 'bubbleS', t: t - 0.6 }] : [],
  }
})
def('cold', 'Shivering', 'idle', 1.2, (t) => ({
  dx: pick(t, [-1, 0, 1, 0], 12), eyes: 'squeeze', mouth: 'teeth', armL: 4, armR: 4, blush: true, sy: 0.95, ear: 1,
  props: [{ type: 'snow' }],
}))
def('sing', 'Sing', 'idle', 2.4, (t) => ({
  mouth: pick(t, ['open', 'o', 'smile', 'open'], 3), eyes: 'happy', lean: pick(t, [-1, 0, 1, 0], 2), armR: 120,
  props: [{ type: 'notes' }], screen: { mode: 'icon', icon: 'note', oy: Math.floor(t * 3) % 2 },
}))
def('nod', 'Yes (nod)', 'idle', 1.6, (t) => ({ look: [0, pick(t, [0, 1], 4)], sy: pick(t, [1, 0.95], 4), eyes: 'happy', mouth: 'smile' }))
def('no', 'No (head shake)', 'idle', 1.6, (t) => ({ look: [pick(t, [-1, 1], 6), 0], mouth: 'flat', eyes: 'closed', brow: 'flat' }))
def('shrug', 'Shrug', 'idle', 2, (t) => {
  const up = t > 0.3 && t < 1.6
  return { armL: up ? 100 : 14, armR: up ? 100 : 14, dy: up ? 1 : 0, look: [1, 0], eyes: 'sleepy', mouth: 'smirk', props: up ? [{ type: 'q' }] : [] }
})
def('meditate', 'Meditate', 'idle', 3, (t) => ({
  dy: 4 + Math.round(Math.sin((t / 3) * Math.PI * 2) * 1.4), eyes: 'line', mouth: 'smile', armL: 80, armR: 80,
  props: [{ type: 'sparkles' }], wag: 0.3,
}))
def('victory', 'Victory', 'idle', 1.6, (t) => ({
  armL: 175, armR: 175, dy: Math.round(3 * hop((t % 0.8) / 0.8)), eyes: 'happy', mouth: 'grin', props: [{ type: 'confetti' }], wag: 2,
}))

// --- moving
def('walk', 'Walk', 'move', 4, (t) => {
  const p = (t / 4) % 1
  const left = p >= 0.5
  return {
    dx: Math.round(lerp(-9, 9, tri(t, 4))), flip: left, lean: left ? -1 : 1, step: pick(t, [-1, 0, 1, 0], 8), dy: Math.floor(t * 8) % 2,
    eyes: open(t, 2), look: [left ? -1 : 1, 0], armL: pick(t, [30, 0], 4), armR: pick(t, [0, 30], 4),
  }
})
def('run', 'Run', 'move', 2, (t) => {
  const left = (t / 2) % 1 >= 0.5
  const dir = left ? -1 : 1
  return {
    dx: Math.round(lerp(-13, 13, tri(t, 2))), flip: left, lean: 2 * dir, step: pick(t, [-1, 1], 12), dy: Math.floor(t * 12) % 2,
    mouth: 'open', brow: 'angry', look: [dir, 0], armL: pick(t, [70, 10], 6), armR: pick(t, [10, 70], 6),
    props: [{ type: 'dust', dir }, { type: 'speed', dir }],
  }
})
def('jump', 'Big jump', 'move', 1.4, (t) => {
  if (t < 0.3) return { sy: 0.82, sx: 1.12, eyes: 'closed', armL: 0, armR: 0 }
  if (t < 0.9) {
    const p = (t - 0.3) / 0.6
    return { dy: Math.round(13 * hop(p)), sy: 1.08, sx: 0.94, armL: 170, armR: 170, eyes: 'wide', mouth: 'open', ear: -1 }
  }
  if (t < 1.05) return { sy: 0.84, sx: 1.14, eyes: 'squeeze', mouth: 'flat', props: [{ type: 'dust', t: t - 0.9 }] }
  return { eyes: 'happy' }
})
def('peek', 'Peekaboo', 'move', 4, (t) => {
  const base = { clip: true }
  if (t < 0.8) return { ...base, dy: -26 }
  if (t < 1.2) return { ...base, dy: -14, look: [0, -1] }
  if (t < 2.4) return { ...base, dy: -10, look: [t < 1.8 ? -1 : 1, 0], eyes: open(t, 1, 0.2), mouth: 'none' }
  if (t < 3) return { ...base, dy: -26 }
  return { dy: 0, eyes: 'happy', mouth: 'grin', armL: 165, armR: 165, props: [{ type: 'text', text: 'BOO', y: 3 }] }
})
def('trip', 'Trip and fall', 'move', 3.4, (t) => {
  if (t < 0.9) return { dx: Math.round(lerp(-8, -2, t / 0.9)), step: pick(t, [-1, 0, 1, 0], 8), dy: Math.floor(t * 8) % 2, look: [1, 0] }
  if (t < 1.2) return { dx: -1, lean: 4, eyes: 'wide', mouth: 'o', dy: 2, armL: 120, armR: 120 }
  if (t < 2.3) return { dx: 1, sy: 0.62, sx: 1.25, eyes: 'x', mouth: 'wavy', armL: 90, armR: 90, props: [{ type: 'stars' }], wag: 0 }
  return { dx: 1, sy: lerp(0.85, 1, (t - 2.3) / 0.5), eyes: t < 2.7 ? 'squeeze' : 'happy', mouth: 'smile', blush: true, props: [{ type: 'sweat' }] }
})
def('launch', 'Blast off', 'move', 2.4, (t) => {
  if (t < 0.6) return { sy: 0.82, sx: 1.1, brow: 'angry', mouth: 'flat', dx: pick(t, [-1, 1], 12), props: [{ type: 'dust' }] }
  if (t < 1.6) {
    const p = (t - 0.6) / 1
    return { dy: Math.round(30 * Math.sin(p * Math.PI)), sy: 1.12, sx: 0.9, armL: 175, armR: 175, eyes: 'wide', mouth: 'grin', props: [{ type: 'trail' }] }
  }
  return { sy: t < 1.75 ? 0.82 : 1, sx: t < 1.75 ? 1.15 : 1, eyes: 'happy', mouth: 'open', props: [{ type: 'sparkles' }] }
})

// --- interactions
def('poke', 'Poked', 'play', 1.4, (t) => {
  if (t < 0.15) return { sx: 1.14, sy: 0.84, eyes: 'squeeze', mouth: 'o' }
  if (t < 0.6) return { dy: 3, eyes: 'wide', mouth: 'o', armL: 120, armR: 120, ear: -1, props: [{ type: '!' }] }
  return { eyes: 'happy', mouth: 'open', dy: pick(t, [0, 1], 8), wag: 2 }
}, { once: true })
def('pet', 'Head pat', 'play', 2, (t) => {
  const c = Math.floor(t * 8) % 8
  return {
    sy: c === 2 || c === 6 ? 0.92 : 0.96, eyes: 'happy', blush: true, mouth: 'cat', lean: [-1, -1, 0, 1, 1, 1, 0, -1][c], ear: 1, wag: 2,
    props: [{ type: 'pat' }, { type: 'hearts', n: 2 }],
  }
})
def('tickle', 'Tickled', 'play', 1.4, (t) => ({
  eyes: 'squeeze', mouth: pick(t, ['grin', 'open'], 8), lean: pick(t, [-2, 2], 8), dy: pick(t, [0, 1], 8),
  armL: pick(t, [40, 140], 8), armR: pick(t, [140, 40], 8), wag: 3, props: [{ type: 'text', text: 'HEHE', y: 3 }],
}))
def('dragged', 'Picked up', 'play', 1.6, (t) => ({
  dy: 9, sy: 1.1, sx: 0.92, armL: 168, armR: 168, step: pick(t, [-1, 1], 6), kick: pick(t, [1, -1], 6), eyes: 'wide', mouth: 'wavy',
  lean: pick(t, [-1, 1], 3), ear: 1, props: [{ type: 'sweat' }],
}))
def('landed', 'Dropped', 'play', 1.2, (t) => {
  if (t < 0.15) return { sy: 0.68, sx: 1.25, eyes: 'squeeze', mouth: 'flat', props: [{ type: 'dust', t }] }
  if (t < 0.4) return { sy: 1.08, sx: 0.94, dy: 2, eyes: 'wide', mouth: 'o' }
  if (t < 0.6) return { sy: 0.95, eyes: 'spiral', mouth: 'wavy' }
  return { eyes: 'happy', mouth: 'smile' }
}, { once: true })
def('eat', 'Snack', 'play', 3.2, (t) => {
  const bites = Math.min(4, Math.floor(t / 0.7))
  return {
    armL: 120, armR: 120, eyes: bites >= 4 ? 'happy' : open(t, 1.4), blush: bites >= 4,
    mouth: bites >= 4 ? 'cat' : pick(t, ['open', 'flat'], 5), props: [{ type: 'cookie', bites, crumbs: bites > 0 && bites < 4 }],
  }
})
def('drink', 'Coffee', 'play', 3, (t) => {
  const up = t > 0.6 && t < 1.8
  return {
    armR: up ? 150 : 40, eyes: up ? 'closed' : t > 1.8 ? 'happy' : 'open', mouth: up ? 'small' : t > 1.8 ? 'open' : 'smile', blush: t > 1.8,
    props: [{ type: 'mug', steam: !up }],
  }
})
def('gaming', 'Gaming', 'play', 2, (t) => ({
  armL: pick(t, [45, 38], 8), armR: pick(t, [38, 45], 8), brow: 'angry', mouth: 'tongue', look: [0, pick(t, [0, 0, 0, -1], 2)],
  dy: pick(t, [0, 0, 0, 1], 4), props: [{ type: 'gamepad' }, ...(t > 1.2 ? [{ type: 'sweat' }] : [])],
  screen: { mode: 'icon', icon: 'gamepad' },
}))
def('greet', 'Say hi', 'play', 2, (t) => ({
  armR: pick(t, [155, 115], 6), eyes: 'happy', mouth: 'open', props: [{ type: 'hiBubble' }], screen: { mode: 'text', text: 'HI!' },
}))
def('sign', 'Holds the K// sign', 'play', 2.4, (t) => ({
  armR: 150, eyes: open(t, 2.4), mouth: 'open', lean: pick(t, [0, 1], 2), props: [{ type: 'sign' }],
}))

// --- in context. `p` is the real progress when the page passes one.
def('upload', 'Uploading', 'ctx', 3.2, (t, p) => {
  const done = p > 0.97
  return {
    armL: 168, armR: 168, step: done ? 0 : pick(t, [-1, 0, 1, 0], 6), look: [0, -1], eyes: done ? 'happy' : 'open', mouth: done ? 'open' : 'tongue',
    props: [{ type: 'file', bob: !done }, { type: 'bar', v: p }, ...(done ? [{ type: 'sparkles' }] : [{ type: 'arrows', dir: 'up' }])],
    screen: { mode: 'bar', v: p, icon: 'up' },
  }
}, { progress: true })
def('download', 'Downloading', 'ctx', 3.2, (t, p) => {
  const done = p > 0.97
  return {
    armL: done ? 60 : 125, armR: done ? 60 : 125, look: done ? [0, 0] : [0, -1], eyes: done ? 'happy' : 'wide', mouth: done ? 'open' : 'o',
    props: [{ type: 'bar', v: p }, ...(done ? [{ type: 'box', pos: null }] : [{ type: 'arrows', dir: 'down' }])],
    screen: { mode: 'bar', v: p, icon: 'down' },
  }
}, { progress: true })
def('loading', 'Loading', 'ctx', 2, (t) => {
  const f = Math.floor(t * FPS)
  const a = ((f % 8) / 8) * Math.PI * 2 - Math.PI / 2
  return {
    look: [Math.round(Math.cos(a)), Math.round(Math.sin(a))], step: pick(t, [0, 1], 4), mouth: 'small',
    props: [{ type: 'spinner' }], screen: { mode: 'spinner' },
  }
})
def('search', 'Searching', 'ctx', 3.2, (t) => {
  const lean = pick(t, [-2, -2, 0, 2, 2, 0], 2)
  return {
    armR: 120, lean, look: [Math.sign(lean), 0], eyes: 'wide', mouth: 'small', props: [{ type: 'magnifier' }],
    screen: { mode: 'icon', icon: 'search', ox: Math.sign(lean) * 3 },
  }
})
def('typing', 'Typing in chat', 'ctx', 1.8, (t) => ({
  armL: pick(t, [50, 40], 8), armR: pick(t, [40, 50], 8), look: [1, -1], mouth: 'small', eyes: open(t, 1.8),
  props: [{ type: 'bubble' }], screen: { mode: 'dots' },
}))
def('message', 'New message', 'ctx', 2.4, (t) => {
  if (t < 0.5) return { props: [{ type: 'envelope', t }], look: [0, -1], eyes: 'wide', mouth: 'o', screen: { mode: 'icon', icon: 'envelope' } }
  return {
    dy: t < 0.9 ? Math.round(3 * hop((t - 0.5) / 0.4)) : 0, armL: 165, armR: 165, eyes: 'happy', mouth: 'open',
    props: [{ type: 'envelope', held: true }, { type: '!' }], screen: { mode: 'icon', icon: 'envelope', oy: Math.floor(t * 4) % 2 },
  }
})
def('encrypted', 'Encrypted chat', 'ctx', 2.4, (t) => ({
  armL: 120, armR: 120, eyes: 'happy', mouth: 'smile', look: [0, pick(t, [0, 1], 2)], props: [{ type: 'lock' }, { type: 'sparkles' }],
  screen: { mode: 'icon', icon: 'lock' },
}))
def('success', 'Done!', 'ctx', 2, (t) => ({
  dy: t < 0.5 ? Math.round(5 * hop(t / 0.5)) : 0, armL: 172, armR: 172, eyes: 'happy', mouth: 'grin',
  props: [{ type: 'check', t }, { type: 'sparkles' }], screen: { mode: 'icon', icon: 'check' }, wag: 2,
}))
def('error', 'Something broke', 'ctx', 2.4, (t) => ({
  dx: t > 0.3 && t < 0.7 ? pick(t, [-1, 1], 12) : 0, eyes: t < 0.3 ? 'wide' : 'open', brow: 'sad', mouth: 'wavy', sy: 0.95, armL: 0, armR: 0, ear: 1, wag: 0,
  props: [{ type: 'cross', t }, { type: 'sweat' }], screen: t < 0.7 ? { mode: 'static' } : { mode: 'icon', icon: 'cross' },
}))
def('offline', 'Offline', 'ctx', 3, (t) => ({
  eyes: t % 1.5 > 1.3 ? 'closed' : 'sleepy', brow: 'sad', mouth: 'frown', look: [0, -1], armL: 0, armR: 0, sy: 0.95, ear: 1, wag: 0,
  props: [{ type: 'cloud', x: true }], screen: t % 1 < 0.5 ? { mode: 'static' } : { mode: 'text', text: 'OFF' },
}))
def('empty', 'No results', 'ctx', 2.4, (t) => ({
  armL: 100, armR: 100, dy: 1, look: [pick(t, [-1, 1], 1.5), 0], eyes: 'sleepy', mouth: 'smirk', props: [{ type: 'q' }],
  screen: { mode: 'icon', icon: 'question' },
}))
def('fixing', 'Repairing', 'ctx', 1.6, (t) => ({
  armR: pick(t, [170, 110], 6), brow: 'angry', mouth: 'tongue', props: [{ type: 'wrench', sparks: pick(t, [false, true], 6) }, { type: 'sweat' }],
  screen: { mode: 'icon', icon: 'wrench', oy: pick(t, [0, 1], 6) },
}))
def('syncing', 'Cloud saves', 'ctx', 2.4, (t) => {
  const l = (t % 1.2) / 1.2
  return {
    armL: 160, armR: 160, look: [0, -1], eyes: 'open', mouth: 'smile',
    props: [{ type: 'cloud', sync: true }, ...(l < 0.8 ? [{ type: 'box', pos: [24, 26 - l * 12] }] : [])],
    screen: { mode: 'icon', icon: 'cloud', oy: Math.floor(t * 3) % 2 },
  }
})
def('notify', 'Notification', 'ctx', 1.6, (t) => ({
  dy: pick(t, [0, 1], 8), eyes: 'wide', mouth: 'o', ear: -1, props: [{ type: 'bell' }], screen: { mode: 'icon', icon: 'bell', ox: pick(t, [-1, 1], 10) },
}))
def('voice', 'Voice call', 'ctx', 1.6, (t) => ({
  mouth: pick(t, ['open', 'small', 'o', 'smile'], 6), eyes: open(t, 1.6), armR: 130, props: [{ type: 'waves' }], screen: { mode: 'wave' },
}))
def('waiting', 'In the queue', 'ctx', 3.2, (t) => ({
  step: pick(t, [0, 1], 4), eyes: 'sleepy', look: [-1, 0], mouth: 'flat', armR: 110, props: [{ type: 'hourglass' }],
  screen: { mode: 'icon', icon: 'hourglass' },
}))
def('achievement', 'Achievement', 'ctx', 2, (t) => ({
  armL: 170, armR: 170, eyes: 'star', mouth: 'grin', dy: Math.round(2 * hop((t % 0.5) / 0.5)), props: [{ type: 'trophy' }, { type: 'confetti' }],
  screen: { mode: 'icon', icon: 'star' }, wag: 2,
}))
def('friend', 'Friend request', 'ctx', 2.4, (t) => ({
  armR: pick(t, [155, 115], 6), eyes: 'happy', blush: true, mouth: 'smile', props: [{ type: 'heartBubble' }],
  screen: { mode: 'icon', icon: 'heart', color: 'c' },
}))
def('playing', 'Game launching', 'ctx', 2.4, (t) => ({
  ...ANIMS.launch.fn(t), screen: { mode: 'icon', icon: 'gamepad' },
}))

// --- only one mascot
def('melt', 'Melting', 'own', 3.4, (t) => {
  const melt = lerp(0, 0.85, (t - 0.3) / 2)
  return {
    melt, eyes: melt > 0.5 ? 'squeeze' : 'open', brow: 'sad', mouth: melt > 0.5 ? 'wavy' : 'frown', armL: 0, armR: 0,
    props: [{ type: 'drips' }, { type: 'sweat' }],
  }
}, { only: 'kubi' })
def('refreeze', 'Refreezing', 'own', 3, (t) => {
  const melt = lerp(0.85, 0, (t - 0.2) / 1.6)
  return {
    melt, eyes: melt > 0.1 ? 'closed' : 'happy', mouth: melt > 0.1 ? 'small' : 'open', dy: t > 2 ? Math.round(3 * hop((t - 2) / 0.5)) : 0,
    armL: melt ? 14 : 160, armR: melt ? 14 : 160, props: [{ type: 'snow' }, { type: 'sparkles', key: 'w' }],
  }
}, { only: 'kubi' })
def('skate', 'Ice skating', 'own', 3, (t) => {
  const left = (t / 3) % 1 >= 0.5
  return {
    dx: Math.round(lerp(-12, 12, tri(t, 3))), lean: left ? -2 : 2, flip: left, armL: 90, armR: 90, eyes: 'happy', mouth: 'open',
    props: [{ type: 'trail' }],
  }
}, { only: 'kubi' })
def('static', 'Bad signal', 'own', 2, (t) => ({
  dx: t < 1.2 ? pick(t, [-1, 1], 12) : 0, screen: t < 1.2 ? { mode: 'static' } : null, ear: pick(t, [-1, 1], 4),
  eyes: t < 1.6 ? 'squeeze' : 'open', mouth: 'flat',
}), { only: 'telly' })
def('channels', 'Channel surfing', 'own', 3.6, (t) => {
  const i = Math.floor(t / 0.6)
  const between = t % 0.6 < 0.1
  const sc = [null, { mode: 'icon', icon: 'heart', color: 'c' }, { mode: 'text', text: 'K//' }, { mode: 'icon', icon: 'gamepad' }, { mode: 'icon', icon: 'note' }, { mode: 'icon', icon: 'star' }][i % 6]
  return { armR: 120, screen: between ? { mode: 'static' } : sc }
}, { only: 'telly' })
def('logo', 'K// on screen', 'own', 2, (t) => ({ screen: { mode: 'text', text: 'K//', cursor: true }, ear: pick(t, [0, 1, 0, -1], 4) }), { only: 'telly' })
def('tailchase', 'Tail chase', 'own', 2.4, (t) => ({
  flip: pick(t, [false, true], 4), dx: Math.round(Math.sin((t / 2.4) * Math.PI * 4) * 3), eyes: 'wide', mouth: 'open', wag: 3, tail: pick(t, [-1, 1], 4),
  lean: pick(t, [2, -2], 4),
}), { only: 'yuki' })
def('pounce', 'Pounce', 'own', 2.4, (t) => {
  if (t < 1) return { sy: 0.8, sx: 1.1, look: [1, 0], dx: pick(t, [0, 1], 8), eyes: 'wide', brow: 'angry', mouth: 'small', wag: 4, ear: -1 }
  if (t < 1.5) return { dx: Math.round(lerp(0, 9, (t - 1) / 0.5)), dy: Math.round(7 * hop((t - 1) / 0.5)), armL: 120, armR: 120, eyes: 'wide', mouth: 'open', sy: 1.06 }
  return { dx: 9, eyes: 'happy', mouth: 'cat', sy: t < 1.6 ? 0.85 : 1, wag: 2 }
}, { only: 'yuki' })
def('curl', 'Curled up nap', 'own', 4, (t) => ({
  sy: t % 2 < 1 ? 0.78 : 0.74, sx: 1.08, eyes: 'closed', mouth: 'small', ear: 1, tail: -1, wag: 0, armL: 0, armR: 0, props: [{ type: 'zzz' }],
}), { only: 'yuki' })
def('jelly', 'Jelly wobble', 'own', 1.2, (t) => {
  const i = Math.floor(t * 10) % 12
  const s = [[1.14, 0.86], [0.9, 1.12], [1.08, 0.93], [0.95, 1.05], [1.03, 0.98], [1, 1], [1, 1], [1, 1], [1, 1], [1, 1], [1, 1], [1, 1]][i]
  return { sx: s[0], sy: s[1], eyes: i < 5 ? 'squeeze' : 'happy', mouth: 'cat' }
}, { only: 'blip' })
def('puddle', 'Puddle', 'own', 3.2, (t) => {
  if (t < 1) return { sy: lerp(1, 0.42, t), sx: lerp(1, 1.4, t), eyes: 'closed', mouth: 'flat' }
  if (t < 2) return { sy: 0.42, sx: 1.4, eyes: 'line', mouth: 'small', props: [{ type: 'zzz' }] }
  if (t < 2.4) return { sy: 1.3, sx: 0.8, eyes: 'wide', mouth: 'o', dy: 3 }
  return { sy: t < 2.55 ? 0.86 : 1, sx: t < 2.55 ? 1.12 : 1, eyes: 'happy', mouth: 'open' }
}, { only: 'blip' })
def('stretchy', 'Stretch tall', 'own', 2, (t) => {
  if (t < 0.9) return { sy: lerp(1, 1.5, t / 0.9), sx: lerp(1, 0.78, t / 0.9), look: [0, -1], eyes: 'wide', mouth: 'o', armL: 170, armR: 170 }
  if (t < 1.1) return { sy: 0.75, sx: 1.2, eyes: 'squeeze', mouth: 'flat' }
  return { eyes: 'happy', mouth: 'smile' }
}, { only: 'blip' })

export function animsFor(mascot) {
  return Object.values(ANIMS).filter((a) => !a.only || a.only === mascot)
}

/* ------------------------------------------------------------------------ */
/* Rendering                                                                */
/* ------------------------------------------------------------------------ */

/**
 * One frame as a grid of palette keys.
 * opts: theme, mode, progress (0..1), override (pose fields), extras (bool).
 */
export function render(mascot, anim, t, opts = {}) {
  const M = MASCOTS[mascot]
  const A = ANIMS[anim] || ANIMS.idle
  const tq = Math.floor(t * FPS + 1e-6) / FPS
  const local = A.once ? Math.min(tq, A.dur) : tq % A.dur
  const prog = opts.progress ?? local / A.dur
  const pose = { ...BASE, ...A.fn(local, prog, mascot), ...(opts.override || {}) }
  return compose(M, pose, local, { theme: opts.theme || 'monochrome', mode: opts.mode || 'dark', extras: opts.extras !== false })
}

export function compose(M, P, t, ctx) {
  const g = new Grid()
  if (!P.clip && P.melt < 0.3) {
    const r = Math.max(3, M.shadow * P.sx - Math.min(Math.max(P.dy, 0), 14) * 0.45)
    fill(g, new Mask().ellipse(24 + P.dx, GROUND + 1.5, r, 1.1), 'x', { outline: null })
  }
  const mg = new Grid()
  const a = M.draw(mg, P, ctx, t)
  if (ctx.extras && !(THEMES[ctx.theme].neon && ctx.mode === 'dark')) accessory(mg, ctx.theme, a)
  const shift = (y) => Math.round((P.lean * (GROUND - y)) / 20)
  for (let y = 0; y < H; y++)
    for (let x = 0; x < W; x++) {
      const k = mg.px[y * W + x]
      if (!k) continue
      const ny = y - P.dy
      if (P.clip && ny > GROUND) continue
      g.set((P.flip ? W - 1 - x : x) + P.dx + shift(y), ny, k)
    }
  const tf = ([x, y]) => [(P.flip ? W - x : x) + P.dx + shift(y), y - P.dy]
  const anchors = {}
  for (const k in a) anchors[k] = Array.isArray(a[k]) ? tf(a[k]) : a[k]
  if (P.flip) [anchors.handL, anchors.handR] = [anchors.handR, anchors.handL]
  if (!(P.clip && P.dy < -6)) drawProps(g, P, anchors, t)
  return g
}

/** Palette keys -> RGBA bytes. `bg` (rgba array) flattens transparency. */
export function toRGBA(grid, colors, bg = null) {
  const out = new Uint8ClampedArray(grid.w * grid.h * 4)
  for (let i = 0; i < grid.px.length; i++) {
    const k = grid.px[i]
    const c = k ? colors[k] || [255, 0, 255, 255] : null
    let r = 0, gg = 0, b = 0, a = 0
    if (bg) [r, gg, b, a] = bg
    if (c) {
      const al = c[3] / 255
      if (bg) {
        r = Math.round(c[0] * al + r * (1 - al))
        gg = Math.round(c[1] * al + gg * (1 - al))
        b = Math.round(c[2] * al + b * (1 - al))
        a = 255
      } else [r, gg, b, a] = c
    }
    out[i * 4] = r
    out[i * 4 + 1] = gg
    out[i * 4 + 2] = b
    out[i * 4 + 3] = a
  }
  return out
}

export function bgColor(theme, mode, which = 'bg') {
  return [...oklch(...THEMES[theme][mode][which]), 255]
}
