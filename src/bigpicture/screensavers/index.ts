import type { VariantSpec } from './engine.ts'
import { CLOCK_SAVER } from './clock.ts'
import { FIELD_SAVER } from './field.ts'
import { MARK_SAVER } from './mark.ts'
import { makeShuffle } from './shuffle.ts'
import { SNAKE_SAVER } from './snake.ts'

/** Big Picture's screensavers, in the order Settings shows them. Mark is the default. */
const ONES = [MARK_SAVER, CLOCK_SAVER, FIELD_SAVER, SNAKE_SAVER] as const
export const SAVERS: readonly VariantSpec[] = [...ONES, makeShuffle(ONES)]

export type SaverId = 'mark' | 'clock' | 'field' | 'snake' | 'shuffle'
export const SAVER_IDS = SAVERS.map((s) => s.id) as SaverId[]
export const DEFAULT_SAVER: SaverId = 'mark'

/** What each one is, in a line, for Settings. */
export const SAVER_HINTS: Record<SaverId, string> = {
  mark: 'The K// in matrix dots, with light passing through.',
  clock: 'The time in matrix dots, and the game you played last.',
  field: 'The start screen, with the names of your games coming and going.',
  snake: 'A wall of matrices, waves passing, now and then the K//.',
  shuffle: 'Each of the others in turn, a few minutes each.',
}

export function saverSpec(id: string): VariantSpec {
  return SAVERS.find((s) => s.id === id) ?? SAVERS[0]!
}
