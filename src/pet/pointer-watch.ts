/** Asks in a row with no answer before the pet takes clicks on its whole window: 3 s at 150 ms. */
export const MISSES = 20

/**
 * Whether the pet's window takes every click on its square because the pointer
 * cannot be asked for (pet.rs `pet_hit`, KB-0369).
 *
 * Windows does not say where the pointer is while its secure desktop is up: a
 * UAC prompt, Ctrl+Alt+Del, a locked screen. One such miss used to turn
 * click-through off until Kryoto restarted, and the see-through corners took
 * clicks again. Nobody can click the pet then anyway, so only a run of misses
 * counts, and the next answer gives the drawn pet alone the clicks again.
 */
export function pointerWatch() {
  let misses = 0
  return {
    answered() {
      misses = 0
    },
    /** Whether the window should take every click now. */
    missed() {
      misses += 1
      return misses >= MISSES
    },
  }
}
