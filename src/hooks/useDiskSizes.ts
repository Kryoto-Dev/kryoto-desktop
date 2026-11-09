import { useEffect, useState } from 'react'
import { library, type LibraryGame } from '@/lib/library'

const LIBRARY_KEY = 'kryoto.library.sizes'

/** What `useDiskSizes` needs of a game: Settings, Storage has its own idea of the folder. */
type Sized = Pick<LibraryGame, 'id' | 'installDir' | 'version'>

type Measured = { bytes: number; dir: string; version: string | null }
type Sizes = Record<string, Measured>

function read(key: string): Sizes {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key) ?? '{}')
    return value && typeof value === 'object' ? (value as Sizes) : {}
  } catch {
    return {}
  }
}

function write(key: string, sizes: Sizes) {
  try {
    localStorage.setItem(key, JSON.stringify(sizes))
  } catch {
    /* storage unavailable: measured again next visit */
  }
}

/** Still right: the same folder at the same version. */
const fresh = (m: Measured | undefined, g: Sized) => m != null && m.dir === g.installDir && m.version === g.version

/**
 * What each game takes on disk, for the Library's size sort and Settings, Storage.
 *
 * Walking a game's folder is slow on a hard disk, so this only runs while
 * `active`, one game at a time, and remembers the answer until the game moves
 * or updates. `measuring` counts the games still to go; a number for a game
 * appears as soon as it is known.
 */
export function useDiskSizes(games: Sized[], active: boolean, key = LIBRARY_KEY) {
  const [sizes, setSizes] = useState<Sizes>(() => read(key))
  const [measuring, setMeasuring] = useState(0)
  // The library's list is a new array after any change, a play session included.
  const stamp = games.map((g) => `${g.id}\n${g.installDir}\n${g.version}`).join('\n')

  useEffect(() => {
    if (!active) {
      setMeasuring(0)
      return
    }
    let stopped = false
    const known = read(key)
    const todo = games.filter((g) => !fresh(known[g.id], g))
    setMeasuring(todo.length)
    void (async () => {
      for (const [done, g] of todo.entries()) {
        if (stopped) return
        // A folder that cannot be read is 0, the same as an unknown size.
        const bytes = await library.diskSize(g.installDir).catch(() => 0)
        if (stopped) return
        known[g.id] = { bytes, dir: g.installDir, version: g.version }
        write(key, known)
        setSizes({ ...known })
        setMeasuring(todo.length - done - 1)
      }
    })()
    return () => {
      stopped = true
    }
  }, [stamp, active]) // eslint-disable-line react-hooks/exhaustive-deps

  return { sizes, measuring }
}
