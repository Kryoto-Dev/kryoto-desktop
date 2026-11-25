import { useEffect, useState } from 'react'
import { Button, Section } from '@/ui'
import { ChevronDown, ChevronUp } from '@/ui/pixel-icons'
import { call } from '@/lib/bridge'
import { moveIn, orderList, OURS } from '@/lib/download-order'
import type { Settings } from '@/lib/settings'

/**
 * Settings > Downloads: where downloads come from first (suggestion #110).
 * Every host Kryoto downloads from itself, and our own copy, in one list the
 * player moves up and down. The Library's and the Downloads page's buttons
 * start a build from the first one it has; the Store page, phone and
 * collection downloads still take kryo.to's pick. A mirror that has lost the
 * file, or fails before any of it came, moves on to the next one below it the
 * build has, then to our copy (src-tauri downloads.rs `hand_on`). Untouched,
 * the order is kryo.to's and nothing moves on.
 */
export function DownloadOrderSection({
  settings: s,
  set,
}: {
  settings: Settings
  set: <K extends keyof Settings>(k: K, v: Settings[K]) => void
}) {
  const [hosts, setHosts] = useState<string[]>([])
  useEffect(() => {
    let cancelled = false
    void call<string[]>('download_hosts')
      .then((h) => !cancelled && setHosts(h))
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])
  const order = s.downloadOrder ?? []
  const list = orderList(order, hosts)
  const move = (i: number, by: -1 | 1) => set('downloadOrder', moveIn(list, i, by))
  const starts = "Download buttons in the Library and on the Downloads page start from the first of these a build has. The Store page and your phone keep kryo.to's pick."
  const handsOn =
    "When a mirror has lost the file, or fails before any of it arrives, the next one below it that the build has takes over, then Kryoto's own copy if you are signed in."
  return (
    <Section title="Download order" anchor="download-order" hint={order.length ? `${starts} ${handsOn}` : starts}>
      <ol aria-label="Download order" className="kryo-radius grid border border-border">
        {list.map((name, i) => (
          <li key={name} className="flex items-center gap-2 border-b border-border px-3 py-1.5 last:border-b-0">
            <span className="w-5 text-right font-mono text-[11px] text-muted-foreground">{i + 1}</span>
            <span className="grow text-xs text-foreground">{label(name)}</span>
            <Button size="sm" variant="ghost" aria-label={`Move ${label(name)} up`} disabled={i === 0} onClick={() => move(i, -1)}>
              <ChevronUp className="size-3" />
            </Button>
            <Button size="sm" variant="ghost" aria-label={`Move ${label(name)} down`} disabled={i === list.length - 1} onClick={() => move(i, 1)}>
              <ChevronDown className="size-3" />
            </Button>
          </li>
        ))}
      </ol>
      <div className="flex flex-wrap items-center gap-2">
        {order.length ? (
          <Button size="sm" variant="outline" onClick={() => set('downloadOrder', [])}>
            Use kryo.to's order
          </Button>
        ) : (
          <span className="text-[11px] text-muted-foreground">kryo.to's order: our copy first, then its mirrors, and nothing taken over when one fails.</span>
        )}
      </div>
    </Section>
  )
}

function label(name: string): string {
  return name === OURS ? "Kryoto's own copy" : name
}
