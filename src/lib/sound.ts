/**
 * Kryoto's sounds: short synthesised tones, like dots of the matrix lighting
 * (the recipes are in sound-recipes.ts, where `scripts/render-sounds.mts`
 * renders the same ones to MP3 for review).
 *
 * Everyday sounds are off unless the player turns them on (Settings >
 * Sounds), and even then only while this window is in front: behind it, the
 * system notification already makes its own. Big Picture's are on by
 * default, like a console's, with their own switch. Synthesised rather than
 * files, so there is nothing to ship and nothing to load before the first one.
 */
import { call } from '@/lib/bridge'
import { BIG_PICTURE_SOUNDS, ROOM, SOUNDS, lengthOf, type Recipe, type SoundName, type Voice } from './sound-recipes'

export { SOUNDS, BIG_PICTURE_SOUNDS, type SoundName }

const KEY = 'kryo:sounds'
const BP_KEY = 'kryo:sounds-bp'
const VOLUME_KEY = 'kryo:sounds-volume'

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function write(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    /* storage unavailable: the default again next launch */
  }
}

/** Everyday sounds (downloads, notifications, achievements): off by default. */
export function soundsOn(): boolean {
  return read(KEY) === 'on'
}

export function setSoundsOn(on: boolean) {
  write(KEY, on ? 'on' : 'off')
}

/** Big Picture's sounds: on by default. */
export function bigPictureSoundsOn(): boolean {
  return read(BP_KEY) !== 'off'
}

export function setBigPictureSoundsOn(on: boolean) {
  write(BP_KEY, on ? 'on' : 'off')
}

/** 0 to 1; 0.8 by default. */
export function soundVolume(): number {
  const v = Number(read(VOLUME_KEY))
  return read(VOLUME_KEY) !== null && Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0.8
}

export function setSoundVolume(v: number) {
  write(VOLUME_KEY, String(Math.min(1, Math.max(0, v))))
}

let ctx: AudioContext | null = null
let noiseBuffer: AudioBuffer | null = null
/** Little cues fired many times a second (holding a direction) play once per gap. */
const lastPlayed = new Map<SoundName, number>()
const MIN_GAP_MS: Partial<Record<SoundName, number>> = { bpMove: 45, bpTab: 60 }

/**
 * Play one of Kryoto's sounds, if its switch is on: Big Picture's switch for
 * Big Picture's sounds, the everyday one for the rest. `force` plays it
 * regardless, for previews in Settings. Everyday sounds stay quiet while the
 * window is behind others, unless forced.
 */
export function play(name: SoundName, { force = false }: { force?: boolean; bigPicture?: boolean } = {}) {
  const bp = BIG_PICTURE_SOUNDS.includes(name)
  if (!force) {
    if (bp ? !bigPictureSoundsOn() : !soundsOn()) return
    if (!bp && typeof document !== 'undefined' && document.visibilityState === 'hidden') return
  }
  const now = performance.now()
  const gap = MIN_GAP_MS[name]
  if (gap && now - (lastPlayed.get(name) ?? -Infinity) < gap) return
  lastPlayed.set(name, now)
  const volume = soundVolume()
  if (volume <= 0) return
  try {
    ctx ??= new AudioContext()
    // A PC with no output device, or one Windows has just unplugged, rejects
    // this ("Failed to start the audio device"): silence, not a report.
    if (ctx.state === 'suspended') void ctx.resume().catch(() => {})
    const own = custom.get(name)
    if (own) void playOwn(ctx, name, own, volume)
    else render(ctx, SOUNDS[name], ctx.currentTime + 0.01, volume)
  } catch {
    /* no audio device: silence is the fallback */
  }
}

// ---- The player's own sounds (custom_sounds.rs) -----------------------------

export type CustomSound = { name: SoundName; file: string }

/** Longer files are cut here: a sound, not a song. */
const MAX_OWN_SECONDS = 8
/** By sound: the file's bytes until it first plays, then the decoded audio. */
const custom = new Map<SoundName, ArrayBuffer | AudioBuffer>()
let customList: CustomSound[] = []
const customListeners = new Set<() => void>()

/** Which sounds are the player's own: read at start, and again after every change. */
export async function loadCustomSounds(): Promise<void> {
  try {
    const list = (await call<CustomSound[]>('sound_custom_list')).filter((c) => c.name in SOUNDS)
    const read = await Promise.all(
      list.map((c) =>
        call<ArrayBuffer>('sound_custom_read', { name: c.name })
          .then((bytes) => [c.name, bytes] as const)
          .catch(() => null),
      ),
    )
    custom.clear()
    for (const r of read) if (r) custom.set(r[0], r[1])
    customList = list
  } catch {
    /* outside Kryoto Desktop, or the folder is unreadable: Kryoto's own sounds play */
  }
  customListeners.forEach((f) => f())
}

export function customSounds(): CustomSound[] {
  return customList
}

export function onCustomSounds(f: () => void): () => void {
  customListeners.add(f)
  return () => customListeners.delete(f)
}

/** Play the file at `path` instead of `name`. It is copied into Kryoto's custom sounds folder. */
export async function setCustomSound(name: SoundName, path: string): Promise<void> {
  await call('sound_custom_set', { name, path })
  await loadCustomSounds()
}

/** Back to Kryoto's own sound for `name`. */
export async function clearCustomSound(name: SoundName): Promise<void> {
  await call('sound_custom_clear', { name })
  await loadCustomSounds()
}

async function playOwn(c: AudioContext, name: SoundName, own: ArrayBuffer | AudioBuffer, volume: number) {
  try {
    // decodeAudioData takes the bytes over, so it gets a copy.
    const audio = own instanceof AudioBuffer ? own : await c.decodeAudioData(own.slice(0))
    custom.set(name, audio)
    const src = c.createBufferSource()
    const gain = c.createGain()
    src.buffer = audio
    gain.gain.value = volume
    src.connect(gain).connect(c.destination)
    src.start()
    src.stop(c.currentTime + Math.min(audio.duration, MAX_OWN_SECONDS))
  } catch {
    // A file the system cannot decode: Kryoto's own sound, not silence.
    render(c, SOUNDS[name], c.currentTime + 0.01, volume)
  }
}

if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) void loadCustomSounds()

function noise(c: AudioContext): AudioBuffer {
  if (noiseBuffer && noiseBuffer.sampleRate === c.sampleRate) return noiseBuffer
  const buf = c.createBuffer(1, c.sampleRate, c.sampleRate)
  const data = buf.getChannelData(0)
  for (let i = 0; i < data.length; i++) data[i] = Math.random() * 2 - 1
  noiseBuffer = buf
  return buf
}

function render(c: AudioContext, recipe: Recipe, at: number, volume: number) {
  const master = c.createGain()
  master.gain.value = volume
  master.connect(c.destination)
  let bus: AudioNode = master
  if (recipe.room) {
    // The room (ROOM in sound-recipes.ts): dry straight to the master, and
    // into four damped feedback delays whose sum is the wet part.
    const dry = c.createGain()
    const wet = c.createGain()
    wet.gain.value = recipe.room.mix / ROOM.combs.length
    dry.connect(master)
    wet.connect(master)
    for (const d of ROOM.combs) {
      const delay = c.createDelay(1)
      const damp = c.createBiquadFilter()
      const feedback = c.createGain()
      delay.delayTime.value = d * (recipe.room.size ?? 1)
      damp.type = 'lowpass'
      damp.frequency.value = ROOM.damp
      feedback.gain.value = ROOM.feedback
      dry.connect(delay)
      delay.connect(damp).connect(feedback).connect(delay)
      delay.connect(wet)
    }
    bus = dry
  }
  for (const v of recipe.voices) voice(c, v, at, bus)
  // Let go of the graph once the tail has rung out.
  const end = lengthOf(recipe) + 0.1
  window.setTimeout(() => master.disconnect(), end * 1000 + 50)
}

function voice(c: AudioContext, v: Voice, start: number, out: AudioNode) {
  const at = start + v.at
  const end = at + v.dur
  const attack = Math.min(v.attack ?? 0.004, v.dur / 2)
  const release = Math.min(v.release ?? 0.03, v.dur - attack)
  let src: AudioScheduledSourceNode
  if (v.wave === 'noise') {
    const n = c.createBufferSource()
    n.buffer = noise(c)
    n.loop = true
    src = n
  } else {
    const osc = c.createOscillator()
    osc.type = v.wave
    osc.frequency.setValueAtTime(v.freq ?? 440, at)
    if (v.freqEnd) osc.frequency.exponentialRampToValueAtTime(v.freqEnd, end)
    src = osc
  }
  const gain = c.createGain()
  gain.gain.setValueAtTime(0, at)
  gain.gain.linearRampToValueAtTime(v.gain, at + attack)
  if (v.decay) {
    // A struck note: it dies away on its own (the recipe makes it long enough).
    gain.gain.setTargetAtTime(0, at + attack, v.decay)
  } else {
    gain.gain.setValueAtTime(v.gain, end - release)
    gain.gain.linearRampToValueAtTime(0, end)
  }
  let node: AudioNode = src
  if (v.filter) {
    const f = c.createBiquadFilter()
    f.type = v.filter.type
    f.Q.value = v.filter.q ?? 0.7071
    f.frequency.setValueAtTime(v.filter.freq, at)
    if (v.filter.freqEnd) f.frequency.exponentialRampToValueAtTime(v.filter.freqEnd, end)
    node.connect(f)
    node = f
  }
  node.connect(gain).connect(out)
  src.start(at)
  src.stop(end + 0.02)
}
