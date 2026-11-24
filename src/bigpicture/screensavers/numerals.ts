/**
 * Clock numerals in matrix dots: five across, seven down, `#` lit. Drawn for
 * the 3x3 matrix's look: straight strokes, one diagonal where a stroke has to
 * turn (2, 4, 7), no rounding tricks a dot cannot do. Each numeral has a dot in
 * its top and bottom row, so a row of them reads as one line of type.
 *
 * Kept free of imports so `pnpm test` can load it.
 */

export const NUMERAL_W = 5
export const NUMERAL_H = 7

export const NUMERALS: Record<string, readonly string[]> = {
  '0': ['.###.', '#...#', '#..##', '#.#.#', '##..#', '#...#', '.###.'],
  '1': ['..#..', '.##..', '..#..', '..#..', '..#..', '..#..', '.###.'],
  '2': ['.###.', '#...#', '....#', '...#.', '..#..', '.#...', '#####'],
  '3': ['#####', '...#.', '..#..', '...#.', '....#', '#...#', '.###.'],
  '4': ['...#.', '..##.', '.#.#.', '#..#.', '#####', '...#.', '...#.'],
  '5': ['#####', '#....', '####.', '....#', '....#', '#...#', '.###.'],
  '6': ['..##.', '.#...', '#....', '####.', '#...#', '#...#', '.###.'],
  '7': ['#####', '....#', '...#.', '..#..', '.#...', '.#...', '.#...'],
  '8': ['.###.', '#...#', '#...#', '.###.', '#...#', '#...#', '.###.'],
  '9': ['.###.', '#...#', '#...#', '.####', '....#', '...#.', '.##..'],
  ' ': ['.....', '.....', '.....', '.....', '.....', '.....', '.....'],
}

/** The colon: one column, two dots. */
export const COLON: readonly string[] = ['.', '.', '#', '.', '#', '.', '.']

/** Whether the numeral `ch` has a dot at (x, y). */
export function numeralDot(ch: string, x: number, y: number): boolean {
  return (NUMERALS[ch] ?? NUMERALS[' ']!)[y]?.[x] === '#'
}

/**
 * Whether this system shows the time in 12 hours, as the clock in its
 * taskbar does (the locale's own hour cycle, read through Intl).
 */
export function systemTwelveHour(locale?: string): boolean {
  try {
    return !!new Intl.DateTimeFormat(locale, { hour: 'numeric' }).resolvedOptions().hour12
  } catch {
    return false
  }
}

/**
 * The four numerals of the time and whether it is afternoon: "0941" in 24
 * hours; " 941" in 12 (the first numeral blank, so the clock never jumps
 * sideways when the hour gains a digit).
 */
export function clockDigits(date: Date, twelve: boolean): { digits: string; pm: boolean } {
  const h = date.getHours()
  const m = String(date.getMinutes()).padStart(2, '0')
  if (!twelve) return { digits: `${String(h).padStart(2, '0')}${m}`, pm: h >= 12 }
  const h12 = h % 12 === 0 ? 12 : h % 12
  return { digits: `${String(h12).padStart(2, ' ')}${m}`, pm: h >= 12 }
}
