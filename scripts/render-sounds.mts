/**
 * Render Kryoto Desktop's sounds (src/lib/sound-recipes.ts) to MP3s for
 * review, with the same tones, envelopes, filters and echo the app plays live
 * through Web Audio (src/lib/sound.ts).
 *
 *   pnpm exec tsx scripts/render-sounds.mts <out-folder>
 *
 * Needs ffmpeg on PATH for the MP3s; without it the WAVs are kept.
 */
import { mkdirSync, writeFileSync, unlinkSync } from 'node:fs'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import { SOUNDS, lengthOf, type Recipe, type Voice } from '../src/lib/sound-recipes.ts'

const RATE = 44100
/** The app's default master volume (sound.ts). */
const MASTER = 0.8
/** Previews are a little louder than the app so they can be judged on any speaker. */
const PREVIEW_GAIN = 3

let seed = 1
const noise = () => {
  seed = (seed * 1664525 + 1013904223) >>> 0
  return (seed / 0xffffffff) * 2 - 1
}

/** PolyBLEP, so square and saw do not alias (Web Audio's are band-limited). */
function blep(t: number, dt: number): number {
  if (t < dt) {
    const x = t / dt
    return x + x - x * x - 1
  }
  if (t > 1 - dt) {
    const x = (t - 1) / dt
    return x * x + x + x + 1
  }
  return 0
}

/** One RBJ biquad, the same filter Web Audio's BiquadFilterNode is. */
class Biquad {
  b0 = 1; b1 = 0; b2 = 0; a1 = 0; a2 = 0; x1 = 0; x2 = 0; y1 = 0; y2 = 0
  set(type: 'lowpass' | 'highpass' | 'bandpass', f: number, q: number) {
    const w = (2 * Math.PI * Math.min(f, RATE / 2 - 100)) / RATE
    const cos = Math.cos(w)
    const alpha = Math.sin(w) / (2 * q)
    let b0: number, b1: number, b2: number
    if (type === 'lowpass') { b0 = (1 - cos) / 2; b1 = 1 - cos; b2 = (1 - cos) / 2 }
    else if (type === 'highpass') { b0 = (1 + cos) / 2; b1 = -(1 + cos); b2 = (1 + cos) / 2 }
    else { b0 = alpha; b1 = 0; b2 = -alpha }
    const a0 = 1 + alpha
    this.b0 = b0 / a0; this.b1 = b1 / a0; this.b2 = b2 / a0
    this.a1 = (-2 * cos) / a0; this.a2 = (1 - alpha) / a0
  }
  run(x: number) {
    const y = this.b0 * x + this.b1 * this.x1 + this.b2 * this.x2 - this.a1 * this.y1 - this.a2 * this.y2
    this.x2 = this.x1; this.x1 = x; this.y2 = this.y1; this.y1 = y
    return y
  }
}

function envelope(v: Voice, t: number): number {
  const attack = v.attack ?? 0.004
  const release = v.release ?? 0.03
  if (t < 0 || t > v.dur) return 0
  if (t < attack) return t / attack
  // Web Audio's setTargetAtTime, which sound.ts uses for struck notes.
  if (v.decay) return Math.exp(-(t - attack) / v.decay)
  if (t > v.dur - release) return Math.max(0, (v.dur - t) / release)
  return 1
}

function render(recipe: Recipe): Float32Array {
  const out = new Float32Array(Math.ceil(lengthOf(recipe) * RATE))
  for (const v of recipe.voices) {
    const start = Math.floor(v.at * RATE)
    const n = Math.ceil(v.dur * RATE)
    const filter = v.filter ? new Biquad() : null
    let phase = 0
    for (let i = 0; i < n && start + i < out.length; i++) {
      const p = i / n
      const t = i / RATE
      // Web Audio's exponentialRampToValueAtTime, which sound.ts uses for glides.
      const f = v.freqEnd ? v.freq! * Math.pow(v.freqEnd / v.freq!, p) : v.freq ?? 0
      const dt = f / RATE
      let s: number
      switch (v.wave) {
        case 'sine': s = Math.sin(2 * Math.PI * phase); break
        case 'triangle': s = 1 - 4 * Math.abs(Math.round(phase - 0.25) - (phase - 0.25)); break
        case 'square': s = (phase < 0.5 ? 1 : -1) + blep(phase, dt) - blep((phase + 0.5) % 1, dt); break
        case 'sawtooth': s = 2 * phase - 1 - blep(phase, dt); break
        default: s = noise()
      }
      phase = (phase + dt) % 1
      if (filter && v.filter) {
        if (i % 32 === 0) {
          const ff = v.filter.freqEnd ? v.filter.freq * Math.pow(v.filter.freqEnd / v.filter.freq, p) : v.filter.freq
          filter.set(v.filter.type, ff, v.filter.q ?? 0.7071)
        }
        s = filter.run(s)
      }
      out[start + i]! += s * v.gain * envelope(v, t)
    }
  }
  if (recipe.echo) {
    const d = Math.floor(recipe.echo.delay * RATE)
    const wet = new Float32Array(out.length)
    for (let i = d; i < out.length; i++) wet[i] = (out[i - d]! + wet[i - d]! * recipe.echo.feedback)
    for (let i = 0; i < out.length; i++) out[i]! += wet[i]! * recipe.echo.mix
  }
  for (let i = 0; i < out.length; i++) out[i] = Math.max(-1, Math.min(1, out[i]! * MASTER * PREVIEW_GAIN))
  return out
}

function wav(samples: Float32Array): Buffer {
  const buf = Buffer.alloc(44 + samples.length * 2)
  buf.write('RIFF', 0); buf.writeUInt32LE(36 + samples.length * 2, 4); buf.write('WAVE', 8)
  buf.write('fmt ', 12); buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20); buf.writeUInt16LE(1, 22)
  buf.writeUInt32LE(RATE, 24); buf.writeUInt32LE(RATE * 2, 28); buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34)
  buf.write('data', 36); buf.writeUInt32LE(samples.length * 2, 40)
  samples.forEach((s, i) => buf.writeInt16LE(Math.round(s * 32767), 44 + i * 2))
  return buf
}

const outDir = process.argv[2] ?? 'sound-previews'
mkdirSync(outDir, { recursive: true })
for (const [name, recipe] of Object.entries(SOUNDS)) {
  // A short silence before, so a player does not clip the attack.
  const body = render(recipe)
  const padded = new Float32Array(body.length + Math.floor(RATE * 0.15))
  padded.set(body, Math.floor(RATE * 0.15))
  const wavPath = join(outDir, `${name}.wav`)
  writeFileSync(wavPath, wav(padded))
  const mp3 = join(outDir, `${name}.mp3`)
  const r = spawnSync('ffmpeg', ['-y', '-loglevel', 'error', '-i', wavPath, '-codec:a', 'libmp3lame', '-b:a', '192k', mp3])
  if (r.status === 0) unlinkSync(wavPath)
  console.log(`${name.padEnd(16)} ${(padded.length / RATE).toFixed(2)}s  ${recipe.label}${r.status === 0 ? '' : '  (WAV only: ffmpeg failed)'}`)
}
