// Types for engine.js, the mascot's pixel engine. A verbatim copy of kryo.to's
// lib/mascot/engine.js and engine.d.ts: change them there and copy them here.

export type MascotId = 'kubi' | 'telly' | 'yuki' | 'blip'
export type ThemeId = 'monochrome' | 'oled' | 'amber' | 'emerald' | 'nord' | 'sepia' | 'blossom'
export type Mode = 'light' | 'dark'
export type RGBA = [number, number, number, number]

export const W: number
export const H: number
export const GROUND: number
export const FPS: number

export class Grid {
  constructor(w?: number, h?: number)
  w: number
  h: number
  px: (string | null)[]
  get(x: number, y: number): string | null
  set(x: number, y: number, key: string | null): void
}

export type Anim = {
  id: string
  label: string
  cat: 'idle' | 'move' | 'play' | 'ctx' | 'own'
  dur: number
  once?: boolean
  only?: MascotId
  progress?: boolean
}

export type Pose = Partial<{
  dx: number
  dy: number
  sx: number
  sy: number
  lean: number
  flip: boolean
  look: [number, number]
  eyes: string
  mouth: string
}> &
  Record<string, unknown>

export const MASCOTS: Record<MascotId, { id: MascotId; name: string; tagline: string; pitch: string }>
export const MASCOT_IDS: MascotId[]
export const ANIMS: Record<string, Anim>
export const THEMES: Record<ThemeId, { label: string }>
export const EXTRAS: Record<ThemeId, string>

export function animsFor(mascot: MascotId): Anim[]
export function render(
  mascot: MascotId,
  anim: string,
  t: number,
  opts?: { theme?: ThemeId; mode?: Mode; progress?: number; override?: Pose; extras?: boolean },
): Grid
export function makeColors(theme: ThemeId, mode: Mode): Record<string, RGBA>
export function toRGBA(grid: Grid, colors: Record<string, RGBA>, bg?: RGBA | null): Uint8ClampedArray<ArrayBuffer>
