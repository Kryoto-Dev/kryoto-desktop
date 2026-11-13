import { useEffect, useState } from 'react'
import { Trash2 } from '@/ui/pixel-icons'
import { AsciiBar, Button, Caption, Check, Modal } from '@/ui'
import { errorText, on } from '@/lib/bridge'
import { formatBytes } from '@/lib/downloads'
import { launcherName } from '@/lib/launchers'
import { gameLogs, library, type LibraryGame } from '@/lib/library'

/** What the dialog needs of a game: a Library row, or a row of Settings > Storage. */
export type UninstallTarget = Pick<LibraryGame, 'id' | 'title' | 'installDir'> & { launcher?: string | null }

/** How far deleting a game's files has got (library.rs `DeleteProgress`). */
type DeleteProgress = { id: string; removed: number; total: number; done: boolean }

/**
 * The one uninstall dialog, for one game or several (Settings > Storage): it
 * asks first, with the choice to keep the files, and shows the files going
 * the way moving a game shows them copied (KB-0304, KB-0305).
 */
export function UninstallDialog({
  games,
  onClose,
  onDone,
}: {
  games: UninstallTarget[]
  onClose: () => void
  /** After the last one: whether files were deleted, and the games that went. */
  onDone: (deleted: boolean, gone: string[]) => void
}) {
  const one = games.length === 1 ? games[0]! : null
  // Another launcher's game is only ever taken off the list.
  const keeper = one ? launcherName(one.launcher) : null
  const ours = games.filter((g) => !launcherName(g.launcher))
  const [deleteFiles, setDeleteFiles] = useState(ours.length > 0)
  const [deletePrefix, setDeletePrefix] = useState(false)
  const [hasPrefix, setHasPrefix] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [current, setCurrent] = useState<string | null>(null)
  const [progress, setProgress] = useState<DeleteProgress | null>(null)
  const oneId = one?.id ?? null
  useEffect(() => {
    if (!oneId) return
    let off = false
    gameLogs.prefix(oneId).then((p) => !off && setHasPrefix(!!p)).catch(() => {})
    return () => {
      off = true
    }
  }, [oneId])
  useEffect(() => {
    let cancelled = false
    let stop: (() => void) | undefined
    void on<DeleteProgress>('library-uninstall', (p) => {
      if (!cancelled) setProgress(p.done ? null : p)
    }).then((fn) => {
      if (cancelled) fn()
      else stop = fn
    })
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])

  const uninstall = async () => {
    setBusy(true)
    setError(null)
    const gone: string[] = []
    let deleted = false
    try {
      for (const g of games) {
        setCurrent(g.id)
        deleted = (await library.remove(g.id, deleteFiles, hasPrefix && deletePrefix)) || deleted
        gone.push(g.id)
      }
      onDone(deleted, gone)
    } catch (e) {
      // Open, with what went wrong; the ones before it are gone all the same,
      // and the screen behind follows the library when this closes.
      setError(errorText(e))
      setBusy(false)
      setCurrent(null)
    }
  }

  const label = one ? one.title : `${games.length} games`
  const working = games.find((g) => g.id === current)
  return (
    <Modal
      title={keeper ? `Remove ${label}` : `Uninstall ${label}`}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button variant="danger" disabled={busy} onClick={() => void uninstall()}>
            <Trash2 className="size-3.5" />
            {keeper ? 'Remove' : busy ? 'Uninstalling' : 'Uninstall'}
          </Button>
        </>
      }
    >
      {keeper ? (
        <p className="text-xs text-muted-foreground">{`It stays installed in ${keeper}. Only its place in your Kryoto Library goes.`}</p>
      ) : (
        <>
          <Check checked={deleteFiles} disabled={busy} onChange={setDeleteFiles} label={one ? "Delete the game's files" : "Delete the games' files"} />
          {one ? (
            <p className="kryo-ascii-art select-text break-all text-[11px] text-muted-foreground">{one.installDir}</p>
          ) : (
            <ul className="grid gap-0.5 text-[11px] text-muted-foreground">
              {games.map((g) => (
                <li key={g.id} className="truncate" title={g.installDir}>
                  {g.title}
                </li>
              ))}
            </ul>
          )}
        </>
      )}
      {hasPrefix ? (
        <Check
          checked={deletePrefix}
          disabled={busy}
          onChange={setDeletePrefix}
          label="Also delete its Wine prefix (its settings, and any saves it keeps there)"
        />
      ) : null}
      {keeper ? null : (
        <p className="text-xs leading-relaxed text-muted-foreground">
          Files are only deleted for games installed into your library folder. A game you added from somewhere else is only taken
          off the list. Play time is forgotten either way.
        </p>
      )}
      {busy && deleteFiles && working ? (
        <div className="kryo-radius grid gap-1.5 border border-border p-3" role="status">
          <Caption>
            Deleting {working.title}
            {progress && progress.id === working.id && progress.total
              ? ` · ${formatBytes(progress.removed)} of ${formatBytes(progress.total)}`
              : ''}
          </Caption>
          <AsciiBar
            fraction={progress && progress.id === working.id && progress.total ? progress.removed / progress.total : null}
            cells={36}
          />
        </div>
      ) : null}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </Modal>
  )
}
