import { useEffect, useState } from 'react'
import { Download } from '@/ui/pixel-icons'
import { Button, Section } from '@/ui'
import { fetchAddons, type KryoAddon } from '@/lib/library'

/**
 * Properties > Repair: a fix kryo.to keeps for this game (a release of kind
 * `repair`, Online-Fix's generic fix packed by Kryoto Forge). It downloads
 * through the game's download window like any add-on, and the download runner
 * applies it as a repair (src-tauri addons.rs `install_fix`): checked against
 * this game and copied with backups, so Undo below puts back what it replaced.
 */
export function KryoFix({ slug, onGet }: { slug: string; onGet: () => void }) {
  const [fixes, setFixes] = useState<KryoAddon[]>([])
  useEffect(() => {
    let cancelled = false
    void fetchAddons(slug)
      .then((all) => !cancelled && setFixes(all.filter((a) => a.kind === 'repair' && a.offered !== false)))
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [slug])
  if (fixes.length === 0) return null
  return (
    <Section
      title="From kryo.to"
      hint="A fix kryo.to keeps for this game. It replaces only the files it ships, keeps a backup of each, and Undo puts them back. Online-Fix needs Steam running and signed in."
    >
      {fixes.map((f) => (
        <div key={String(f.id)} className="flex flex-wrap items-center gap-3">
          <span className="grow text-xs">
            <b className="text-foreground">{f.label ?? 'Fix'}</b>
            {f.download_size ? <span className="text-muted-foreground"> · {f.download_size}</span> : null}
            {f.note ? <span className="block text-muted-foreground">{f.note}</span> : null}
          </span>
          <Button size="sm" variant="outline" onClick={onGet}>
            <Download className="size-3.5" />
            Get it
          </Button>
        </div>
      ))}
    </Section>
  )
}
