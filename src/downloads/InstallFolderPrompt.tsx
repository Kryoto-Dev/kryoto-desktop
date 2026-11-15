import { useEffect, useState } from 'react'
import { Button, Check, Modal } from '@/ui'
import { errorText } from '@/lib/bridge'
import { downloads, formatBytes, useDownloads } from '@/lib/downloads'
import { FolderChoice, defaultFolder, useLibraryFolders } from '@/downloads/FolderChoice'

/**
 * "Where should it go?" (suggestion #59): with two library folders or more, a
 * new download waits (queued, `askFolder`) until the player picks one. The
 * default is picked, each folder shows its free space, and "Always use the
 * default" stops the question (Settings > Storage turns it back on).
 */
export function InstallFolderPrompt() {
  const { list } = useDownloads()
  const waiting = list.find((d) => d.askFolder && d.status === 'queued') ?? null
  const { folders, error: foldersError } = useLibraryFolders(!!waiting, waiting?.id)
  const [picked, setPicked] = useState('')
  const [always, setAlways] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    setError(null)
    setAlways(false)
    setPicked(defaultFolder(folders)?.path ?? '')
  }, [folders])

  if (!waiting) return null
  const size = waiting.total ?? waiting.meta.sizeBytes ?? null
  // What the drive needs: the game, as it is unpacked in place (HatchZip eats the
  // archive as the game appears), plus a little; the same sum the download checks
  // before it starts (src-tauri storage.rs check_download_room).
  const needs = size ? Math.max(size, waiting.meta.installBytes ?? size) + 1.5 * 2 ** 30 + size / 50 : null
  const answer = (job: Promise<void>) => {
    setBusy(true)
    setError(null)
    void job.catch((e: unknown) => setError(errorText(e))).finally(() => setBusy(false))
  }

  return (
    <Modal
      title="Where should it go?"
      onClose={() => answer(downloads.pickFolder(waiting.id, defaultFolder(folders)?.path ?? picked, false))}
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={() => answer(downloads.cancel(waiting.id))}>
            Cancel download
          </Button>
          <Button variant="primary" disabled={busy || !picked} onClick={() => answer(downloads.pickFolder(waiting.id, picked, always))}>
            Install here
          </Button>
        </>
      }
    >
      <p className="text-xs leading-relaxed text-muted-foreground">
        Pick the library folder for <span className="font-bold text-foreground">{waiting.meta.title}</span>
        {size ? `, about ${formatBytes(size)} to download` : ''}.
      </p>
      <FolderChoice folders={folders ?? []} picked={picked} onPick={setPicked} needs={needs} />
      <Check
        checked={always}
        onChange={(v) => {
          setAlways(v)
          const def = defaultFolder(folders)
          if (v && def) setPicked(def.path)
        }}
        label="Always use the default, and stop asking"
      />
      {error || foldersError ? <p className="text-xs text-destructive">{error ?? foldersError}</p> : null}
    </Modal>
  )
}
