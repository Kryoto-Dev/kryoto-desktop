import { hash, OFF, STILL_MS, type Runner, type Scene, type VariantSpec } from './engine.ts'

/**
 * Shuffle: the other screensavers in turn, a few minutes each, every one
 * before any comes back, never the same one twice in a row. Between two, the
 * picture goes out dot by dot into the dark and the next one comes in the same
 * way. With motion reduced, each one's still frames in turn, with no dissolve.
 */

/** Each one shows this long. */
export const SHUFFLE_MS = 4 * 60_000
/** Going out, and coming in: dot by dot over this long. */
const DISSOLVE_MS = 1_600

/**
 * The order for `rounds` rounds: each round every id once, shuffled, and a
 * round never starts with the one the last round ended on.
 */
export function shuffleOrder(ids: readonly string[], rounds: number, rand: () => number): string[] {
  const out: string[] = []
  for (let round = 0; round < rounds; round++) {
    const deck = ids.slice()
    for (let i = deck.length - 1; i > 0; i--) {
      const j = Math.floor(rand() * (i + 1))
      ;[deck[i], deck[j]] = [deck[j]!, deck[i]!]
    }
    if (deck.length > 1 && deck[0] === out[out.length - 1]) [deck[0], deck[1]] = [deck[1]!, deck[0]!]
    out.push(...deck)
  }
  return out
}

/** Where each cell falls in a dissolve: its own moment in [0, 1). */
function ranks(n: number, seed: number): Float32Array {
  const out = new Float32Array(n)
  for (let i = 0; i < n; i++) out[i] = hash(i * 0.37 + seed, seed)
  return out
}

export function makeShuffle(children: readonly VariantSpec[]): VariantSpec {
  return {
    id: 'shuffle',
    name: 'Shuffle',
    make(scene: Scene): Runner {
      const seed = Math.floor(scene.rand() * 1_000_000)
      let order = shuffleOrder(
        children.map((c) => c.id),
        4,
        (() => {
          let n = seed
          return () => hash(++n, seed)
        })(),
      )
      let index = 0
      let spec = children.find((c) => c.id === order[0])!
      scene.setScale(spec.scale ?? 1, spec.rowPitch)
      let child = spec.make(scene)
      let from = 0
      // When the last still was drawn: the next one comes by the end of the turn.
      let drawn = 0
      let dissolve = ranks(scene.grid.cells.length, seed)
      // In thumbnails, a turn is short enough to see; with motion reduced, a
      // thumbnail's still lasts as long as the full screen's.
      const turn = scene.thumb && !scene.reduced ? 9_000 : SHUFFLE_MS

      const next = (t: number) => {
        index++
        if (index >= order.length) {
          order = shuffleOrder(children.map((c) => c.id), 4, () => hash(t, ++index))
          index = 0
        }
        scene.underlay(null)
        spec = children.find((c) => c.id === order[index])!
        scene.setScale(spec.scale ?? 1, spec.rowPitch)
        child = spec.make(scene)
        dissolve = ranks(scene.grid.cells.length, seed + index)
        from = t
      }

      return {
        frame(t: number) {
          if (scene.reduced) {
            if (t - from >= turn) next(t)
            drawn = t
            return child.frame(t)
          }
          let local = t - from
          if (local >= turn) {
            next(t)
            local = 0
          }
          child.frame(local)
          // Coming in, then going out: dot by dot, in hard steps.
          const cells = scene.grid.cells
          let p = -1
          if (local < DISSOLVE_MS) p = 1 - local / DISSOLVE_MS
          else if (local > turn - DISSOLVE_MS) p = (local - (turn - DISSOLVE_MS)) / DISSOLVE_MS
          if (p > 0) for (let i = 0; i < cells.length; i++) if (dissolve[i]! < p) cells[i] = OFF
        },
        // A new still when the one showing wants it (Clock, each minute), and
        // no later than the end of its turn.
        stillFor(now: Date) {
          return Math.max(0, Math.min(spec.stillFor?.(now) ?? STILL_MS, from + turn - drawn))
        },
      }
    },
  }
}
