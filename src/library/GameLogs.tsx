import { useCallback, useEffect, useState } from 'react'
import { Copy, FolderOpen, RefreshCw } from '@/ui/pixel-icons'
import { Button, Section } from '@/ui'
import { errorText } from '@/lib/bridge'
import { formatBytes } from '@/lib/downloads'
import { gameLogs, library, type GameLogInfo } from '@/lib/library'
import { cn } from '@/lib/utils'

/**
 * A game's launch logs (src-tauri/src/game_logs.rs), the way Heroic shows
 * them: one per Play press, newest first, with the system, the settings, the
 * exact command and everything the game and Wine/Proton printed. Copy one to
 * paste it to staff, or open the folder.
 */
export function GameLogs({ id }: { id: string }) {
  const [list, setList] = useState<GameLogInfo[] | null>(null)
  const [picked, setPicked] = useState<string | null>(null)
  const [text, setText] = useState<string>('')
  const [error, setError] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)

  const load = useCallback(async () => {
    try {
      const l = await gameLogs.list(id)
      setList(l)
      setPicked((p) => (p && l.some((x) => x.name === p) ? p : (l[0]?.name ?? null)))
    } catch (e) {
      setError(errorText(e))
    }
  }, [id])

  useEffect(() => {
    void load()
  }, [load])

  useEffect(() => {
    if (!picked) {
      setText('')
      return
    }
    setError(null)
    gameLogs
      .read(id, picked)
      .then(setText)
      .catch((e: unknown) => setError(errorText(e)))
  }, [id, picked])

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    } catch {
      setError('Could not copy. Select the text and copy it instead.')
    }
  }

  return (
    <Section title="Launch logs" hint="A new log every time you press Play: what was started, on which system, and everything the game printed. The last 10 are kept.">
      <div className="flex flex-wrap items-center gap-2">
        {(list ?? []).map((l) => (
          <button
            key={l.name}
            type="button"
            onClick={() => setPicked(l.name)}
            className={cn(
              'kryo-pill border px-2.5 py-1 text-[10px] tabular-nums transition-colors',
              picked === l.name ? 'border-foreground bg-secondary text-foreground' : 'border-border text-muted-foreground hover:text-foreground',
            )}
            title={formatBytes(l.size)}
          >
            {l.name.replace('.log', '').replace('_', ' ').replace(/-(\d\d)-(\d\d)$/, ':$1:$2')}
          </button>
        ))}
        <span className="ml-auto flex gap-2">
          <Button size="sm" onClick={() => load()} title="Look for new logs">
            <RefreshCw className="size-3" aria-hidden />
          </Button>
          <Button size="sm" disabled={!text} onClick={() => copy()}>
            <Copy className="size-3" aria-hidden /> {copied ? 'Copied' : 'Copy'}
          </Button>
          <Button size="sm" onClick={() => gameLogs.folder(id).then((dir) => library.openFolder(dir, true))}>
            <FolderOpen className="size-3" aria-hidden /> Open folder
          </Button>
        </span>
      </div>
      {list && !list.length ? <p className="text-xs text-muted-foreground">No logs yet. Press Play and one is written.</p> : null}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
      {text ? (
        <pre className="kryo-radius max-h-[50vh] select-text overflow-auto whitespace-pre-wrap break-all border border-border bg-background/60 p-3 font-mono text-[11px] leading-relaxed text-foreground">
          {text}
        </pre>
      ) : null}
    </Section>
  )
}
