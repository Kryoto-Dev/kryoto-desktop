import { useEffect, useState } from 'react'
import { Button, Modal } from '@/ui'
import { on } from '@/lib/bridge'
import { cloudSaves, formatAgo, formatBytes, type SavesPrompt } from '@/lib/cloud-saves'

/**
 * The question Play asks when the account has newer saves than this PC
 * (src-tauri saves/sync.rs `ask`): whose saves the game starts with. Play
 * waits for the answer; closing the dialog starts the game as it is.
 */
export function CloudSavesPrompt() {
  const [prompt, setPrompt] = useState<SavesPrompt | null>(null)

  useEffect(() => {
    const stops: (() => void)[] = []
    let cancelled = false
    const keep = (fn: () => void) => (cancelled ? fn() : stops.push(fn))
    void on<SavesPrompt>('cloud-saves-prompt', setPrompt).then(keep)
    void on<{ gameId: string }>('cloud-saves-prompt-done', (e) => setPrompt((p) => (p?.gameId === e.gameId ? null : p))).then(keep)
    return () => {
      cancelled = true
      stops.forEach((s) => s())
    }
  }, [])

  if (!prompt) return null
  const answer = (choice: 'cloud' | 'local' | 'later') => {
    void cloudSaves.answer(prompt.gameId, choice).catch(() => {})
    setPrompt(null)
  }
  const conflict = prompt.kind === 'conflict'
  const from = prompt.cloud.machine ?? 'another PC'

  return (
    <Modal
      title={conflict ? `Which ${prompt.title} saves?` : `Newer ${prompt.title} saves`}
      onClose={() => answer('later')}
      footer={
        <>
          <Button variant="ghost" onClick={() => answer('later')}>{conflict ? 'Decide later' : 'Not now'}</Button>
          {conflict ? <Button onClick={() => answer('local')}>Keep this PC&apos;s</Button> : null}
          <Button variant="primary" onClick={() => answer('cloud')}>{conflict ? "Use the account's" : 'Put them here'}</Button>
        </>
      }
    >
      <div className="grid gap-4 text-xs leading-relaxed text-muted-foreground">
        <p>
          {conflict
            ? `You played on this PC and on ${from} since they last synced. Pick the saves the game starts with. The others are kept as a backup.`
            : `${from} backed up newer saves. Put them here before the game starts? What is here now is kept as a backup.`}
        </p>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
          <dt className="text-[10px] uppercase tracking-wider">Your account</dt>
          <dd className="m-0 text-foreground">
            {from}, {formatAgo(prompt.cloud.createdAt)} · {prompt.cloud.files} files · {formatBytes(prompt.cloud.size)}
          </dd>
          <dt className="text-[10px] uppercase tracking-wider">This PC</dt>
          <dd className="m-0 text-foreground">
            {prompt.local.changedAt ? `changed ${formatAgo(prompt.local.changedAt)}` : 'no saves yet'} · {prompt.local.files} files · {formatBytes(prompt.local.bytes)}
          </dd>
        </dl>
        <p className="text-[11px]">Decide later starts the game with this PC&apos;s saves and asks again next time.</p>
      </div>
    </Modal>
  )
}
