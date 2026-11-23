import { useEffect, useState } from 'react'
import { Button, Modal } from '@/ui'
import { on } from '@/lib/bridge'
import { cloudSaves, formatAgo, formatBytes, type IdFolder, type SavesAnswer, type SavesPrompt } from '@/lib/cloud-saves'

/**
 * The question Play asks when the account has newer saves than this PC
 * (src-tauri saves/sync.rs `ask`): whose saves the game starts with. Play
 * waits for the answer; closing the dialog starts the game as it is.
 *
 * Also the one for saves left under an earlier Steam id (KB-0376,
 * `offer_stranded`): bring them over, with both folders backed up first.
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
  const answer = (choice: SavesAnswer) => {
    void cloudSaves.answer(prompt.gameId, choice).catch(() => {})
    setPrompt(null)
  }
  if (prompt.kind === 'idMoved') return <EarlierSaves title={prompt.title} earlier={prompt.earlier} current={prompt.current} answer={answer} />
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

/** One side of the Steam id question: where, how much, how recent. */
function folderLine(f: IdFolder): string {
  return `${f.changedAt ? `changed ${formatAgo(f.changedAt)}` : 'no saves yet'} · ${f.files} files · ${formatBytes(f.bytes)}`
}

/**
 * KB-0376: the game was moved to the account's Steam id when the player
 * signed in, and its earlier saves stayed under the id it had as a guest.
 */
function EarlierSaves({ title, earlier, current, answer }: { title: string; earlier: IdFolder; current: IdFolder; answer: (choice: SavesAnswer) => void }) {
  return (
    <Modal
      title={`Your earlier ${title} saves`}
      onClose={() => answer('later')}
      footer={
        <>
          <Button variant="ghost" onClick={() => answer('never')}>
            Don&apos;t ask again
          </Button>
          <Button variant="ghost" onClick={() => answer('later')}>
            Not now
          </Button>
          <Button variant="primary" onClick={() => answer('bring')}>
            Bring them over
          </Button>
        </>
      }
    >
      <div className="grid gap-4 text-xs leading-relaxed text-muted-foreground">
        <p>
          Your earlier saves are in another folder: the game looked for them under your account&apos;s Steam id after you signed in. Bring them over? Both
          folders are backed up first, and the earlier saves stay where they are too.
        </p>
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
          <dt className="text-[10px] uppercase tracking-wider">Earlier saves</dt>
          <dd className="m-0 grid min-w-0 text-foreground">
            <span>{folderLine(earlier)}</span>
            <span className="kryo-ascii-art select-text break-all text-[11px] text-muted-foreground">{earlier.path}</span>
          </dd>
          <dt className="text-[10px] uppercase tracking-wider">The game uses</dt>
          <dd className="m-0 grid min-w-0 text-foreground">
            <span>{folderLine(current)}</span>
            <span className="kryo-ascii-art select-text break-all text-[11px] text-muted-foreground">{current.path}</span>
          </dd>
        </dl>
        <p className="text-[11px]">Bringing them over replaces what the game made since you signed in; that is in the backup.</p>
      </div>
    </Modal>
  )
}
