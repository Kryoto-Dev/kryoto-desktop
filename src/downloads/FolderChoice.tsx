import { useEffect, useState } from 'react'
import { HardDrive, Star } from '@/ui/pixel-icons'
import { call, errorText } from '@/lib/bridge'
import { formatBytes } from '@/lib/downloads'
import { cn } from '@/lib/utils'

export type LibraryFolder = { path: string; drive: string; isDefault: boolean; exists: boolean; total: number; free: number }

/** The library folders with their free space, read while `active`. Each new `key` reads them again. */
export function useLibraryFolders(active: boolean, key?: string) {
  const [folders, setFolders] = useState<LibraryFolder[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!active) return
    let alive = true
    setFolders(null)
    setError(null)
    void call<LibraryFolder[]>('storage_folders')
      .then((f) => alive && setFolders(f))
      .catch((e: unknown) => alive && setError(errorText(e)))
    return () => {
      alive = false
    }
  }, [active, key])
  return { folders, error }
}

export const defaultFolder = (folders: LibraryFolder[] | null) => folders?.find((f) => f.isDefault) ?? folders?.[0] ?? null

/** Which library folder: the "Where should it go?" list. A folder too small for `needs` says so. */
export function FolderChoice({ folders, picked, onPick, needs }: { folders: LibraryFolder[]; picked: string; onPick: (path: string) => void; needs: number | null }) {
  return (
    <div className="grid gap-2" role="radiogroup" aria-label="Library folder">
      {folders.map((f) => {
        const on = f.path === picked
        const short = !!needs && f.total > 0 && f.free < needs
        return (
          <button
            key={f.path}
            type="button"
            role="radio"
            aria-checked={on}
            disabled={!f.exists}
            onClick={() => onPick(f.path)}
            className={cn(
              'kryo-radius grid gap-1 border px-3 py-2.5 text-left transition-colors disabled:opacity-40',
              on ? 'border-foreground bg-secondary' : 'border-border hover:border-foreground/50',
            )}
          >
            <span className="flex items-center gap-2 text-xs font-bold text-foreground">
              <HardDrive className="size-3.5" aria-hidden />
              {f.drive}
              {f.isDefault ? <Star className="size-3 fill-current text-warning" aria-label="Default" /> : null}
              <span className={cn('ml-auto text-[10px] font-normal tabular-nums', short ? 'text-warning' : 'text-muted-foreground')}>
                {!f.exists ? 'Missing' : f.total ? `${formatBytes(f.free)} free` : 'Free space unknown'}
              </span>
            </span>
            <span className="kryo-ascii-art truncate text-[11px] text-muted-foreground">{f.path}</span>
          </button>
        )
      })}
    </div>
  )
}
