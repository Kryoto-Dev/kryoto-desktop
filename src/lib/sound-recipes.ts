/**
 * Kryoto Desktop's sounds, as recipes: which notes, when, how loud.
 *
 * One family: struck wooden bars, like a marimba played softly in a small
 * room. Each note is the bar's low fundamental, a twin a hair out of tune
 * (real bars beat slowly, an oscillator never does), the bar's own overtone
 * near four times the pitch that dies almost at once, and the soft knock of a
 * felt mallet. Nothing is tuned in octaves and nothing rings above G4 (392
 * Hz), so it reads as wood, not as a beep. The app plays these live
 * (sound.ts) and `scripts/render-sounds.mts` renders the same recipes to MP3
 * for review, so a preview is what the app plays. Kept free of imports so
 * Node can load it.
 *
 * Times and lengths are seconds; gains are per voice, before the master
 * volume.
 */

export type Wave = 'sine' | 'triangle' | 'square' | 'sawtooth' | 'noise'

export type Voice = {
  wave: Wave
  /** Hz; ignored for noise. */
  freq?: number
  /** Glide to this pitch over the voice's length. */
  freqEnd?: number
  at: number
  dur: number
  gain: number
  /** Seconds to full volume. */
  attack?: number
  /** Seconds to silence at the end (ignored with `decay`). */
  release?: number
  /**
   * Fades away on its own after the attack, like a struck note: the time for
   * the level to fall to about a third. `dur` should be several of these.
   */
  decay?: number
  filter?: { type: 'lowpass' | 'highpass' | 'bandpass'; freq: number; freqEnd?: number; q?: number }
}

export type Recipe = {
  /** What it is for, for the preview page and Settings. */
  label: string
  voices: Voice[]
  /** A small room around it: how much of it is heard (0 to 1), and how big (1 is a small room). */
  room?: { mix: number; size?: number }
}

/**
 * The room both players build: four feedback delays side by side, each with
 * a lowpass in its loop so the walls soak up the highs (a Schroeder comb
 * bank). sound.ts builds it from DelayNodes, render-sounds.mts sample by
 * sample.
 */
export const ROOM = { combs: [0.0297, 0.0371, 0.0411, 0.0437], feedback: 0.74, damp: 2400 }

/**
 * One struck wooden bar: the fundamental that rings, its slightly detuned
 * twin, the bar's own overtone (3.93 times the pitch, as a marimba's is)
 * dying away fast, and the felt mallet's knock, a short burst of filtered air.
 */
function bar(freq: number, at: number, gain: number, decay = 0.3): Voice[] {
  const dur = decay * 6
  return [
    { wave: 'sine', freq, at, dur, gain, attack: 0.003, decay },
    { wave: 'sine', freq: freq * 1.0035, at, dur, gain: gain * 0.35, attack: 0.003, decay: decay * 0.8 },
    { wave: 'sine', freq: freq * 3.93, at, dur: Math.max(0.12, decay * 1.2), gain: gain * 0.18, attack: 0.002, decay: decay * 0.12 },
    { wave: 'noise', at, dur: 0.04, gain: gain * 0.45, attack: 0.001, decay: 0.007, filter: { type: 'bandpass', freq: Math.min(freq * 5, 1600), q: 1.1 } },
  ]
}

/** A held, warm low tone under a bigger moment: a sine that swells in and out. */
const body = (freq: number, at: number, dur: number, gain: number): Voice => ({ wave: 'sine', freq, at, dur, gain, attack: Math.min(0.08, dur / 3), release: dur * 0.6 })

// Notes, from G2 up to G4. Nothing goes higher.
const G2 = 98, A2 = 110, C3 = 130.81, D3 = 146.83, E3 = 164.81, G3 = 196, A3 = 220, B3 = 246.94, C4 = 261.63, D4 = 293.66, E4 = 329.63, G4 = 392

export const SOUNDS = {
  ready: {
    label: 'A game is ready to play',
    voices: [...bar(G3, 0, 0.11, 0.3), ...bar(D4, 0.12, 0.1, 0.5), body(G2, 0, 0.8, 0.035)],
    room: { mix: 0.22 },
  },
  downloadStart: {
    label: 'A download starts',
    voices: [...bar(E3, 0, 0.1, 0.2), ...bar(B3, 0.08, 0.08, 0.26)],
    room: { mix: 0.14 },
  },
  downloadFailed: {
    label: 'A download fails',
    voices: [...bar(C4, 0, 0.09, 0.24), ...bar(A3, 0.16, 0.1, 0.36), body(A2, 0.16, 0.5, 0.03)],
    room: { mix: 0.16 },
  },
  achievement: {
    label: 'An achievement unlocks',
    voices: [
      ...bar(G3, 0, 0.09, 0.24),
      ...bar(B3, 0.09, 0.09, 0.24),
      ...bar(D4, 0.18, 0.09, 0.28),
      ...bar(G4, 0.27, 0.09, 0.6),
      body(G2, 0.27, 1.1, 0.04),
      body(D3, 0.27, 1.1, 0.02),
    ],
    room: { mix: 0.26, size: 1.2 },
  },
  notify: {
    label: 'A notification',
    voices: [...bar(D4, 0, 0.09, 0.24), ...bar(G4, 0.1, 0.07, 0.34)],
    room: { mix: 0.16 },
  },
  message: {
    label: 'A chat message',
    voices: [...bar(B3, 0, 0.08, 0.18), ...bar(E4, 0.08, 0.07, 0.24)],
    room: { mix: 0.12 },
  },
  error: {
    label: 'Something went wrong',
    voices: [...bar(E3, 0, 0.12, 0.12), ...bar(E3, 0.16, 0.11, 0.2), body(82.41, 0, 0.4, 0.03)],
    room: { mix: 0.1 },
  },

  // ---- Calls (they ring whatever the everyday switch says; calls.ts) ----------
  ring: {
    label: 'Someone is calling (repeats)',
    voices: [...bar(G3, 0, 0.09, 0.2), ...bar(E3, 0.14, 0.09, 0.2), ...bar(G3, 0.28, 0.09, 0.2), ...bar(C4, 0.42, 0.09, 0.4), body(C3, 0.42, 0.5, 0.025)],
    room: { mix: 0.18 },
  },
  calling: {
    label: 'Calling someone (repeats)',
    voices: [...bar(G3, 0, 0.06, 0.28), ...bar(B3, 0.22, 0.06, 0.34)],
    room: { mix: 0.14 },
  },
  callJoin: {
    label: 'A call connects',
    voices: [...bar(G3, 0, 0.08, 0.18), ...bar(D4, 0.09, 0.08, 0.3)],
    room: { mix: 0.14 },
  },
  callEnd: {
    label: 'A call ends',
    voices: [...bar(D4, 0, 0.075, 0.18), ...bar(G3, 0.1, 0.08, 0.34)],
    room: { mix: 0.14 },
  },

  // ---- Big Picture -----------------------------------------------------------
  bpIntro: {
    label: 'Big Picture opens',
    voices: [
      // Played so the chord lands as the mark lights (BigPictureIntro.tsx).
      // The cases start their spiral in: a low swell and air, rising.
      { wave: 'sine', freq: 49, freqEnd: G2, at: 0, dur: 0.95, gain: 0.06, attack: 0.3, release: 0.3 },
      { wave: 'noise', at: 0.05, dur: 0.85, gain: 0.022, attack: 0.5, release: 0.3, filter: { type: 'lowpass', freq: 250, freqEnd: 1000, q: 0.8 } },
      // They land in the mark, one soft knock each.
      ...[G3, A3, B3, D4, E4].flatMap((f, i) => bar(f, 0.42 + i * 0.09, 0.035, 0.1)),
      // The mark lights: one warm chord, with a low root.
      ...bar(G3, 0.9, 0.07, 0.7),
      ...bar(B3, 0.9, 0.055, 0.7),
      ...bar(D4, 0.91, 0.055, 0.7),
      body(G2, 0.9, 1.4, 0.05),
      { wave: 'square', freq: 49, at: 0.9, dur: 1.2, gain: 0.025, attack: 0.02, release: 0.9, filter: { type: 'lowpass', freq: 180 } },
    ],
    room: { mix: 0.28, size: 1.3 },
  },
  bpMove: {
    label: 'Big Picture: move',
    voices: [...bar(G3, 0, 0.05, 0.05)],
  },
  bpSelect: {
    label: 'Big Picture: open',
    voices: [...bar(B3, 0, 0.08, 0.12), ...bar(E4, 0.06, 0.07, 0.18)],
    room: { mix: 0.1 },
  },
  bpBack: {
    label: 'Big Picture: back',
    voices: [...bar(E4, 0, 0.07, 0.12), ...bar(B3, 0.06, 0.07, 0.16)],
    room: { mix: 0.1 },
  },
  bpTab: {
    label: 'Big Picture: switch row',
    voices: [...bar(A3, 0, 0.06, 0.07), ...bar(D4, 0.05, 0.055, 0.09)],
  },
  bpLaunch: {
    label: 'Big Picture: play a game',
    voices: [
      { wave: 'sine', freq: 82.41, freqEnd: G3, at: 0, dur: 0.4, gain: 0.05, attack: 0.08, release: 0.15 },
      { wave: 'noise', at: 0, dur: 0.38, gain: 0.018, attack: 0.2, release: 0.15, filter: { type: 'lowpass', freq: 220, freqEnd: 900 } },
      ...bar(G3, 0.34, 0.075, 0.45),
      ...bar(B3, 0.36, 0.06, 0.45),
      ...bar(D4, 0.38, 0.06, 0.5),
      body(G2, 0.34, 0.9, 0.045),
    ],
    room: { mix: 0.22, size: 1.2 },
  },
  bpExit: {
    label: 'Big Picture closes',
    voices: [
      ...bar(D4, 0, 0.065, 0.14),
      ...bar(G3, 0.1, 0.07, 0.34),
      { wave: 'noise', at: 0, dur: 0.35, gain: 0.014, attack: 0.05, release: 0.25, filter: { type: 'lowpass', freq: 800, freqEnd: 220 } },
    ],
    room: { mix: 0.14 },
  },
} satisfies Record<string, Recipe>

export type SoundName = keyof typeof SOUNDS

/** Big Picture's own, which play there by default, like a console's. */
export const BIG_PICTURE_SOUNDS: readonly SoundName[] = ['bpIntro', 'bpMove', 'bpSelect', 'bpBack', 'bpTab', 'bpLaunch', 'bpExit']

/** How long a recipe lasts, tail included. */
export function lengthOf(r: Recipe): number {
  const end = Math.max(...r.voices.map((v) => v.at + v.dur))
  return end + (r.room ? 0.9 * (r.room.size ?? 1) : 0) + 0.05
}
