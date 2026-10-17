/**
 * Kryoto Desktop's sounds, as recipes: which tones, when, how loud.
 *
 * One family: soft, low mallet tones, like a wooden marimba heard from the
 * next room, never a sample and never shrill. Everything sits between about
 * 130 and 520 Hz, with a low body under the bigger moments. The app plays
 * these live (sound.ts) and `scripts/render-sounds.mts` renders the same
 * recipes to MP3 for review, so a preview is what the app plays. Kept free of
 * imports so Node can load it.
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
  /** A soft repeat, for space. */
  echo?: { delay: number; feedback: number; mix: number }
}

/**
 * One struck note: a sine with a quieter octave on top that dies away faster,
 * which is what makes it sound like wood rather than a beep.
 */
function mallet(freq: number, at: number, gain: number, decay = 0.22): Voice[] {
  const dur = decay * 6
  return [
    { wave: 'sine', freq, at, dur, gain, attack: 0.006, decay },
    { wave: 'sine', freq: freq * 2, at, dur: dur * 0.5, gain: gain * 0.22, attack: 0.004, decay: decay * 0.4 },
  ]
}

/** A held, warm tone under a moment: a sine that swells in and out. */
const body = (freq: number, at: number, dur: number, gain: number): Voice => ({ wave: 'sine', freq, at, dur, gain, attack: Math.min(0.08, dur / 3), release: dur * 0.6 })

// Notes, from C3 up to C5. Nothing goes higher.
const C3 = 130.81, G3 = 196, A3 = 220, C4 = 261.63, D4 = 293.66, E4 = 329.63, G4 = 392, A4 = 440, C5 = 523.25

export const SOUNDS = {
  ready: {
    label: 'A game is ready to play',
    voices: [...mallet(D4, 0, 0.09), ...mallet(A4, 0.11, 0.08, 0.32), body(146.83, 0, 0.6, 0.035)],
    echo: { delay: 0.16, feedback: 0.2, mix: 0.18 },
  },
  downloadStart: {
    label: 'A download starts',
    voices: [...mallet(G3, 0, 0.08, 0.14), ...mallet(D4, 0.07, 0.06, 0.16)],
  },
  downloadFailed: {
    label: 'A download fails',
    voices: [...mallet(E4, 0, 0.08, 0.16), ...mallet(C4, 0.15, 0.08, 0.26), body(110, 0.15, 0.4, 0.03)],
  },
  achievement: {
    label: 'An achievement unlocks',
    voices: [
      ...mallet(C4, 0, 0.075, 0.18),
      ...mallet(E4, 0.08, 0.075, 0.18),
      ...mallet(G4, 0.16, 0.075, 0.2),
      ...mallet(C5, 0.24, 0.08, 0.45),
      body(C3, 0.24, 0.9, 0.04),
      body(G3, 0.24, 0.9, 0.025),
    ],
    echo: { delay: 0.18, feedback: 0.25, mix: 0.2 },
  },
  notify: {
    label: 'A notification',
    voices: [...mallet(G4, 0, 0.075, 0.16), ...mallet(C5, 0.1, 0.06, 0.24)],
  },
  message: {
    label: 'A chat message',
    voices: [...mallet(E4, 0, 0.07, 0.12), ...mallet(A4, 0.08, 0.06, 0.16)],
  },
  error: {
    label: 'Something went wrong',
    voices: [
      { wave: 'triangle', freq: A3, at: 0, dur: 0.12, gain: 0.07, attack: 0.005, release: 0.06, filter: { type: 'lowpass', freq: 700 } },
      { wave: 'triangle', freq: A3, at: 0.16, dur: 0.18, gain: 0.07, attack: 0.005, release: 0.12, filter: { type: 'lowpass', freq: 600 } },
    ],
  },

  // ---- Big Picture -----------------------------------------------------------
  bpIntro: {
    label: 'Big Picture opens',
    voices: [
      // Played so the chord lands as the mark lights (BigPictureIntro.tsx).
      // The cases start their spiral in: a low swell and air, rising.
      { wave: 'sine', freq: 65.41, freqEnd: C3, at: 0, dur: 0.95, gain: 0.05, attack: 0.3, release: 0.3 },
      { wave: 'noise', at: 0.05, dur: 0.85, gain: 0.025, attack: 0.5, release: 0.3, filter: { type: 'lowpass', freq: 300, freqEnd: 1400, q: 0.8 } },
      // They land in the mark, one soft knock each.
      ...[C4, D4, E4, G4, A4].flatMap((f, i) => mallet(f, 0.42 + i * 0.09, 0.035, 0.08)),
      // The mark lights: one warm chord, with a low root.
      ...mallet(C4, 0.9, 0.06, 0.6),
      ...mallet(E4, 0.9, 0.05, 0.6),
      ...mallet(G4, 0.91, 0.05, 0.6),
      body(C3, 0.9, 1.4, 0.05),
      { wave: 'square', freq: 65.41, at: 0.9, dur: 1.2, gain: 0.03, attack: 0.02, release: 0.9, filter: { type: 'lowpass', freq: 220 } },
    ],
    echo: { delay: 0.2, feedback: 0.25, mix: 0.18 },
  },
  bpMove: {
    label: 'Big Picture: move',
    voices: [...mallet(G4, 0, 0.04, 0.04)],
  },
  bpSelect: {
    label: 'Big Picture: open',
    voices: [...mallet(E4, 0, 0.07, 0.1), ...mallet(A4, 0.06, 0.06, 0.16)],
  },
  bpBack: {
    label: 'Big Picture: back',
    voices: [...mallet(A4, 0, 0.06, 0.1), ...mallet(E4, 0.06, 0.06, 0.14)],
  },
  bpTab: {
    label: 'Big Picture: switch row',
    voices: [...mallet(D4, 0, 0.05, 0.06), ...mallet(G4, 0.05, 0.05, 0.08)],
  },
  bpLaunch: {
    label: 'Big Picture: play a game',
    voices: [
      { wave: 'sine', freq: 110, freqEnd: G3, at: 0, dur: 0.4, gain: 0.05, attack: 0.08, release: 0.15 },
      { wave: 'noise', at: 0, dur: 0.38, gain: 0.02, attack: 0.2, release: 0.15, filter: { type: 'lowpass', freq: 250, freqEnd: 1100 } },
      ...mallet(C4, 0.34, 0.065, 0.4),
      ...mallet(E4, 0.36, 0.055, 0.4),
      ...mallet(G4, 0.38, 0.055, 0.45),
      body(C3, 0.34, 0.9, 0.04),
    ],
    echo: { delay: 0.18, feedback: 0.22, mix: 0.16 },
  },
  bpExit: {
    label: 'Big Picture closes',
    voices: [
      ...mallet(G4, 0, 0.055, 0.12),
      ...mallet(C4, 0.1, 0.06, 0.3),
      { wave: 'noise', at: 0, dur: 0.35, gain: 0.015, attack: 0.05, release: 0.25, filter: { type: 'lowpass', freq: 1000, freqEnd: 250 } },
    ],
  },
} satisfies Record<string, Recipe>

export type SoundName = keyof typeof SOUNDS

/** Big Picture's own, which play there by default, like a console's. */
export const BIG_PICTURE_SOUNDS: readonly SoundName[] = ['bpIntro', 'bpMove', 'bpSelect', 'bpBack', 'bpTab', 'bpLaunch', 'bpExit']

/** How long a recipe lasts, tail included. */
export function lengthOf(r: Recipe): number {
  const end = Math.max(...r.voices.map((v) => v.at + v.dur))
  return end + (r.echo ? r.echo.delay * 4 : 0) + 0.05
}
