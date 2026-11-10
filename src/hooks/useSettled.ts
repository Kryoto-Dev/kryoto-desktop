import { useEffect, useRef, useState } from 'react'
import { callsInFlight } from '@/lib/bridge'

/**
 * A page made of parts that each ask for their own data arrives a piece at a
 * time, and every piece that lands pushes the ones under it down. So what would
 * move stays out of sight, holding its place, until the calls it made on
 * opening have answered: then it all shows at once and nothing moves. Most
 * answers take a few milliseconds. A slow one (a hard disk, Windows Security)
 * only holds the page for `maxMs`.
 *
 * `key` starts it over (another section, another game). `waiting` is for
 * something that is not a `call`, like an answer from kryo.to.
 */
export function useSettled(key: unknown, waiting = false, maxMs = 600): boolean {
  const [settled, setSettled] = useState<unknown>(Symbol())
  const wait = useRef(waiting)
  wait.current = waiting
  useEffect(() => {
    const start = performance.now()
    const timer = window.setInterval(() => {
      if ((callsInFlight() > 0 || wait.current) && performance.now() - start < maxMs) return
      window.clearInterval(timer)
      setSettled(key)
    }, 30)
    return () => window.clearInterval(timer)
  }, [key, maxMs])
  return settled === key
}
