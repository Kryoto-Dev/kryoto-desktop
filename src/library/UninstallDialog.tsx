import { useEffect, useState } from 'react'
import { Trash2 } from '@/ui/pixel-icons'
import { AsciiBar, Button, Caption, Check, Modal } from '@/ui'
import { errorText, on } from '@/lib/bridge'
import { formatBytes } from '@/lib/downloads'
import { launcherName } from '@/lib/launchers'
import { gameLogs, isWindowsHost, library, type LibraryGame, type UninstallPlan } from '@/lib/library'

/** What the dialog needs of a game: a Library row, or a row of Settings > Storage. */
export type UninstallTarget = Pick<LibraryGame, 'id' | 'title' | 'installDir'> & { launcher?: string | null }

/** How far deleting a game's files has got (library.rs `DeleteProgress`). */
type DeleteProgress = { id: string; removed: number; total: number; done: boolean }

const BIN = isWindowsHost() ? 'Recycle Bin' : 'Trash'

/** The start of library.rs's answer when the Recycle Bin would not take it after all. */
const BIN_REFUSED = 'Could not move it to the'

/** "1,284 files, 12.4 GB": what goes. */
function amount(files: number, bytes: number): string {
  return `${files.toLocaleString()} file${files === 1 ? '' : 's'}, ${formatBytes(bytes)}`
}

/**
 * The one uninstall dialog, for one game or several (Settings > Storage): it
 * asks first, with the choice to keep the files, and shows the files going
 * the way moving a game shows them copied (KB-0304, KB-0305).
 *
 * Before the button does anything it shows exactly what would go, worked out
 * on the PC (src-tauri safe_delete.rs, KB-0375): the folder, how much, what
 * stays, and whether the Recycle Bin can take it. A game Kryoto did not
 * install only leaves the Library; one whose folder is not provably its own
 * says why and deletes nothing.
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
  /** What each game's uninstall would do, by id; missing while it is worked out. */
  const [plans, setPlans] = useState<Record<string, UninstallPlan>>({})
  /** The Recycle Bin refused these after all: the player may delete them for good. */
  const [forGood, setForGood] = useState<string[]>([])
  const oneId = one?.id ?? null
  const ids = ours.map((g) => g.id).join('\n')
  useEffect(() => {
    if (!oneId) return
    let off = false
    gameLogs.prefix(oneId).then((p) => !off && setHasPrefix(!!p)).catch(() => {})
    return () => {
      off = true
    }
  }, [oneId])
  useEffect(() => {
    let off = false
    const all = ids ? ids.split('\n') : []
    void (async () => {
      for (const id of all) {
        const plan = await library.uninstallPlan(id, all).catch(
          (e: unknown): UninstallPlan => ({ kind: 'refused', reason: errorText(e), folder: null, whole: false, files: 0, bytes: 0, keptFiles: 0, keptBytes: 0, bin: false, binNote: null }),
        )
        if (off) return
        setPlans((p) => ({ ...p, [id]: plan }))
      }
    })()
    return () => {
      off = true
    }
  }, [ids])
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

  const planned = ours.every((g) => plans[g.id])
  const deleting = (id: string) => deleteFiles && plans[id]?.kind === 'delete'
  const permanent = (id: string) => deleting(id) && (!plans[id]!.bin || forGood.includes(id))
  const anyDelete = ours.some((g) => plans[g.id]?.kind === 'delete')
  const allForGood = anyDelete && deleteFiles && ours.filter((g) => deleting(g.id)).every((g) => permanent(g.id))

  const uninstall = async () => {
    setBusy(true)
    setError(null)
    const gone: string[] = []
    let deleted = false
    // Only games whose files go may share a folder, and they go first: one
    // that keeps its files still holds the folder around it while that is
    // deleted, which it no longer would once it had left the Library.
    const going = games.filter((g) => deleting(g.id))
    const together = going.map((g) => g.id)
    try {
      for (const g of [...going, ...games.filter((x) => !deleting(x.id))]) {
        setCurrent(g.id)
        try {
          deleted = (await library.remove(g.id, deleting(g.id), hasPrefix && deletePrefix, together, permanent(g.id))) || deleted
        } catch (e) {
          const text = errorText(e)
          if (text.startsWith(BIN_REFUSED)) setForGood((f) => [...f, g.id])
          throw e
        }
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
  const onePlan = one ? plans[one.id] : undefined
  const button = keeper || (one && onePlan && onePlan.kind !== 'delete') || !deleteFiles ? 'Remove from Library' : allForGood ? 'Delete for good' : 'Uninstall'
  return (
    <Modal
      title={keeper || onePlan?.kind === 'listOnly' ? `Remove ${label}` : `Uninstall ${label}`}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button variant="danger" disabled={busy || !planned} onClick={() => void uninstall()}>
            <Trash2 className="size-3.5" />
            {busy ? 'Uninstalling' : button}
          </Button>
        </>
      }
    >
      {keeper ? (
        <p className="text-xs text-muted-foreground">{`It stays installed in ${keeper}. Only its place in your Kryoto Library goes.`}</p>
      ) : !planned ? (
        <div className="grid gap-1.5" role="status">
          <Caption>Checking what would be deleted</Caption>
          <AsciiBar fraction={null} cells={24} showPct={false} />
        </div>
      ) : one && onePlan ? (
        <OnePlan plan={onePlan} deleteFiles={deleteFiles} setDeleteFiles={setDeleteFiles} busy={busy} forGood={forGood.includes(one.id)} />
      ) : (
        <>
          {anyDelete ? <Check checked={deleteFiles} disabled={busy} onChange={setDeleteFiles} label="Delete the games' files" /> : null}
          <ul className="grid gap-1 text-[11px] text-muted-foreground">
            {ours.map((g) => {
              const p = plans[g.id]!
              const what =
                p.kind === 'delete'
                  ? `${formatBytes(p.bytes)}${p.bin && !forGood.includes(g.id) ? '' : ', deleted for good'}`
                  : p.kind === 'listOnly'
                    ? 'only leaves the Library'
                    : 'nothing deleted'
              return (
                <li key={g.id} className="grid min-w-0" title={p.reason ?? p.folder ?? g.installDir}>
                  <span className="truncate">
                    <span className="text-foreground">{g.title}</span> · {what}
                  </span>
                  {p.kind !== 'delete' && p.reason ? <span className="truncate">{p.reason}</span> : null}
                </li>
              )
            })}
          </ul>
          {deleteFiles && ours.some((g) => deleting(g.id) && !plans[g.id]!.bin) ? (
            <p className="text-xs text-warning">Some are too big for the {BIN}: those are deleted for good.</p>
          ) : null}
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
      {keeper || !planned ? null : (
        <p className="text-xs leading-relaxed text-muted-foreground">
          Kryoto only deletes files it installed. Saves outside the game's folder stay. Play time is forgotten either way.
        </p>
      )}
      {busy && working && deleting(working.id) ? (
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
      {error ? (
        <p className="text-xs text-destructive">
          {error}
          {forGood.length ? ` Press Delete for good to delete it without the ${BIN}.` : ''}
        </p>
      ) : null}
    </Modal>
  )
}

/** One game: the exact folder, how much goes, what stays, and where it goes. */
function OnePlan({
  plan,
  deleteFiles,
  setDeleteFiles,
  busy,
  forGood,
}: {
  plan: UninstallPlan
  deleteFiles: boolean
  setDeleteFiles: (on: boolean) => void
  busy: boolean
  forGood: boolean
}) {
  if (plan.kind === 'listOnly') return <p className="text-xs text-muted-foreground">{plan.reason}</p>
  if (plan.kind === 'refused') {
    return (
      <div className="kryo-radius grid gap-1.5 border border-warning/50 bg-warning/10 p-3 text-xs">
        <p className="select-text break-words text-foreground">{plan.reason}</p>
        <p className="text-muted-foreground">Nothing will be deleted. You can still remove it from the Library only.</p>
      </div>
    )
  }
  return (
    <>
      <Check checked={deleteFiles} disabled={busy} onChange={setDeleteFiles} label="Delete the game's files" />
      {deleteFiles ? (
        <div className="grid gap-1 text-[11px] text-muted-foreground">
          <p className="kryo-ascii-art select-text break-all text-foreground">{plan.folder}</p>
          <p>
            {plan.whole ? `This whole folder goes: ${amount(plan.files, plan.bytes)}.` : `${amount(plan.files, plan.bytes)} Kryoto installed go from it.`}
          </p>
          {plan.keptFiles ? (
            <p>{`Kept there: ${amount(plan.keptFiles, plan.keptBytes)} Kryoto did not install, such as saves, settings and mods.`}</p>
          ) : null}
          {plan.bin && !forGood ? (
            <p>{`They go to the ${BIN}, so you can get them back.`}</p>
          ) : (
            <p className="text-warning">{`${plan.binNote ?? `The ${BIN} did not take them.`} They will be deleted for good.`}</p>
          )}
        </div>
      ) : null}
    </>
  )
}
