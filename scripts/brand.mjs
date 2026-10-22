// Render Kryoto Desktop's brand files from code. # yes i used AI to generate this, im not doing this manually.. - Myth.k//
//
//   pnpm brand
//
// Everything comes from src/ui/ascii/cells.ts - the same vector lettering the
// app draws its logo with - so the icon, the installer and the window agree
// to the pixel. Writes:
//
//   src-tauri/icons/   icon.png, 32x32.png, 128x128.png, 128x128@2x.png,
//                      icon.ico (16-256, a simpler mark below 64px), and
//                      icon.icns on macOS
//   src-tauri/installer/   header.bmp (150x57), sidebar.bmp (164x314)
//   docs/logo.png          the mark for the README
//
// Deterministic: same code in, same pixels out.

import sharp from 'sharp'
import { mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { asciiSvg, gridSize, WORDMARK } from '../src/ui/ascii/cells.ts'
import { MARK_H, MARK_PATH, MARK_W } from '../src/ui/ascii/mark.ts'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const icons = path.join(root, 'src-tauri', 'icons')
const installer = path.join(root, 'src-tauri', 'installer')
const docs = path.join(root, 'docs')
for (const d of [icons, installer, docs]) mkdirSync(d, { recursive: true })

const INK = '#ededed'
const TILE = '#0a0a0a'

/**
 * The app icon, drawn at `size` as one SVG so every edge is exact.
 *
 * kryo.to's K// - the real one, traced from the site - on a screen: a dark
 * rounded tile, the mark inside a bezel drawn as the same double line the
 * mark's shadow uses, and a small stand under it. "kryo.to, on your
 * computer". Below 48px the bezel would be a smudge, so the mark goes alone
 * and larger.
 */
function iconSvg(size) {
  const S = 1024
  const small = size < 48
  const r = S * 0.22
  const markW = small ? S * 0.8 : S * 0.56
  const k = markW / MARK_W
  const markH = MARK_H * k
  const mx = (S - markW) / 2
  const my = small ? (S - markH) / 2 : S * 0.335
  // The screen: two thin strokes, a gap between, like the mark's shadow.
  const bx = S * 0.13
  const by = S * 0.2
  const bw = S - bx * 2
  const bh = S * 0.5
  const t = S * 0.012
  const gap = S * 0.014
  const br = S * 0.07
  const bezel = small
    ? ''
    : [0, 1]
        .map((i) => {
          const o = i * (t + gap)
          return `<rect x="${bx + o}" y="${by + o}" width="${bw - o * 2}" height="${bh - o * 2}" rx="${br - o}" fill="none" stroke="${INK}" stroke-width="${t}"/>`
        })
        .join('') +
      // The stand: a neck and a foot, in solid blocks like the mark.
      `<rect x="${S / 2 - S * 0.045}" y="${by + bh + t}" width="${S * 0.09}" height="${S * 0.07}" fill="${INK}"/>` +
      `<rect x="${S / 2 - S * 0.17}" y="${by + bh + t + S * 0.07}" width="${S * 0.34}" height="${S * 0.045}" rx="${S * 0.012}" fill="${INK}"/>`
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${S} ${S}">
  <defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#1a1a1a"/><stop offset="1" stop-color="#060606"/></linearGradient></defs>
  <rect width="${S}" height="${S}" rx="${r}" fill="url(#bg)"/>
  <rect x="3" y="3" width="${S - 6}" height="${S - 6}" rx="${r - 3}" fill="none" stroke="#ffffff" stroke-opacity="0.09" stroke-width="6"/>
  ${bezel}
  <path transform="translate(${mx} ${my}) scale(${k})" d="${MARK_PATH}" fill="${INK}"/>
</svg>`
}

/**
 * Taskbar and tray sizes (up to 48px) are drawn the way Forge's are: a black
 * square edge to edge with the mark as big as it will go, rendered at 8x and
 * brought down with a sharpening filter - small icons read on contrast, and a
 * tile, a border or a screen outline only takes pixels away from the mark.
 */
async function smallIcon(size) {
  const S = size * 8
  const markW = S * 0.86
  const k = markW / MARK_W
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${S}" height="${S}" viewBox="0 0 ${S} ${S}"><rect width="${S}" height="${S}" rx="${S * 0.12}" fill="#000"/><path transform="translate(${(S - markW) / 2} ${(S - MARK_H * k) / 2}) scale(${k})" d="${MARK_PATH}" fill="#fff"/></svg>`
  return sharp(Buffer.from(svg)).resize(size, size, { kernel: 'lanczos3' }).sharpen({ sigma: 0.6 }).png({ compressionLevel: 9 }).toBuffer()
}

const icon = (size) => (size <= 48 ? smallIcon(size) : sharp(Buffer.from(iconSvg(size))).png({ compressionLevel: 9 }).toBuffer())
const markArt = (width) =>
  sharp(Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${MARK_W} ${MARK_H}" width="${width}" height="${Math.round((MARK_H * width) / MARK_W)}"><path d="${MARK_PATH}" fill="${INK}"/></svg>`))
    .png()
    .toBuffer()

/** A .ico of PNG layers (Vista and later read PNG-in-ICO). */
function ico(layers) {
  const header = Buffer.alloc(6)
  header.writeUInt16LE(0, 0)
  header.writeUInt16LE(1, 2)
  header.writeUInt16LE(layers.length, 4)
  const dir = Buffer.alloc(16 * layers.length)
  let offset = 6 + dir.length
  layers.forEach(({ size, png }, i) => {
    const e = i * 16
    dir.writeUInt8(size >= 256 ? 0 : size, e)
    dir.writeUInt8(size >= 256 ? 0 : size, e + 1)
    dir.writeUInt8(0, e + 2)
    dir.writeUInt8(0, e + 3)
    dir.writeUInt16LE(1, e + 4)
    dir.writeUInt16LE(32, e + 6)
    dir.writeUInt32LE(png.length, e + 8)
    dir.writeUInt32LE(offset, e + 12)
    offset += png.length
  })
  return Buffer.concat([header, dir, ...layers.map((l) => l.png)])
}

/** A 24-bit BMP, which is what NSIS wants for its pictures. */
async function bmp(pngBuffer) {
  const { data, info } = await sharp(pngBuffer).flatten({ background: TILE }).removeAlpha().raw().toBuffer({ resolveWithObject: true })
  const { width, height } = info
  const rowSize = Math.ceil((width * 3) / 4) * 4
  const pixels = Buffer.alloc(rowSize * height)
  for (let y = 0; y < height; y++) {
    const src = (height - 1 - y) * width * 3
    for (let x = 0; x < width; x++) {
      const s = src + x * 3
      const d = y * rowSize + x * 3
      pixels[d] = data[s + 2]
      pixels[d + 1] = data[s + 1]
      pixels[d + 2] = data[s]
    }
  }
  const header = Buffer.alloc(54)
  header.write('BM', 0)
  header.writeUInt32LE(54 + pixels.length, 2)
  header.writeUInt32LE(54, 10)
  header.writeUInt32LE(40, 14)
  header.writeInt32LE(width, 18)
  header.writeInt32LE(height, 22)
  header.writeUInt16LE(1, 26)
  header.writeUInt16LE(24, 28)
  header.writeUInt32LE(pixels.length, 34)
  header.writeInt32LE(2835, 38)
  header.writeInt32LE(2835, 42)
  return Buffer.concat([header, pixels])
}

async function art(lines, width, color = INK) {
  const g = gridSize(lines)
  return sharp(Buffer.from(asciiSvg(lines, { color })))
    .resize({ width, height: Math.round((g.height * width) / g.width), fit: 'fill' })
    .png()
    .toBuffer()
}

/* ── App icons ── */
const write = (file, buf) => {
  writeFileSync(file, buf)
  console.log(`${path.relative(root, file)}  ${(buf.length / 1024).toFixed(1)} KB`)
}
write(path.join(icons, 'icon.png'), await icon(1024))
write(path.join(icons, '32x32.png'), await icon(32))
write(path.join(icons, '128x128.png'), await icon(128))
write(path.join(icons, '128x128@2x.png'), await icon(256))
const layers = []
for (const size of [16, 20, 24, 32, 40, 48, 64, 128, 256]) layers.push({ size, png: await icon(size) })
write(path.join(icons, 'icon.ico'), ico(layers))
if (process.platform === 'darwin') {
  const iconset = path.join(icons, 'icon.iconset')
  mkdirSync(iconset, { recursive: true })
  for (const size of [16, 32, 128, 256, 512]) {
    writeFileSync(path.join(iconset, `icon_${size}x${size}.png`), await icon(size))
    writeFileSync(path.join(iconset, `icon_${size}x${size}@2x.png`), await icon(size * 2))
  }
  const result = spawnSync('iconutil', ['-c', 'icns', iconset, '-o', path.join(icons, 'icon.icns')], { stdio: 'inherit' })
  rmSync(iconset, { recursive: true, force: true })
  if (result.status !== 0) throw new Error('iconutil could not create src-tauri/icons/icon.icns')
}
write(path.join(docs, 'logo.png'), await icon(256))

/* ── Installer pictures ── */
// The sidebar: the kryo.to link-preview backdrop (the wall of covers), dimmed,
// with the screen mark and the wordmark over it.
{
  const W = 164
  const H = 314
  const backdrop = readFileSync(path.join(root, 'public', 'brand', 'og-backdrop.jpg'))
  const wall = await sharp(backdrop)
    .resize({ width: W * 2, height: H, fit: 'cover' })
    .extract({ left: Math.round(W / 2), top: 0, width: W, height: H })
    .modulate({ brightness: 0.55, saturation: 0.7 })
    .png()
    .toBuffer()
  const shade = `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${TILE}" stop-opacity="0.1"/><stop offset="0.55" stop-color="${TILE}" stop-opacity="0.45"/><stop offset="1" stop-color="${TILE}" stop-opacity="0.95"/></linearGradient></defs><rect width="${W}" height="${H}" fill="url(#g)"/></svg>`
  const mark = await sharp(await icon(120)).png().toBuffer()
  const words = await art(WORDMARK, 118)
  const sidebar = await sharp(wall)
    .composite([
      { input: Buffer.from(shade), top: 0, left: 0 },
      { input: mark, top: 44, left: 22 },
      { input: words, top: H - 58, left: 23 },
    ])
    .png()
    .toBuffer()
  write(path.join(installer, 'sidebar.bmp'), await bmp(sidebar))
}
// The header strip's picture, on the right of every page after the first.
{
  const W = 150
  const H = 57
  const mark = await markArt(80)
  const header = await sharp({ create: { width: W, height: H, channels: 3, background: TILE } })
    .composite([{ input: mark, gravity: 'centre' }])
    .png()
    .toBuffer()
  write(path.join(installer, 'header.bmp'), await bmp(header))
}
