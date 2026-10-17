import { useEffect, useState } from 'react'
import { Button } from '@/ui'
import { errorText } from '@/lib/bridge'
import { isWindowsHost } from '@/lib/library'
import { askLater, busy, mayAskAtStartup, runtimesApi, setAskAtStartup, useRuntimesProgress, type Runtime } from '@/lib/runtimes'
import { RuntimesProgressView } from '@/settings/RuntimesPane'

/**
 * The missing game runtimes the start screen should offer, or none: Windows
 * only, not after "Don't ask again", not for a week after "Later", and only
 * the recommended set (Settings > Game runtimes has the rest). Asked as the
 * splash starts so the answer is there by the time the mark has lit.
 */
export function useStartupRuntimes(): Runtime[] | null {
  const [missing, setMissing] = useState<Runtime[] | null>(null)
  useEffect(() => {
    if (!isWindowsHost() || !mayAskAtStartup()) {
      setMissing([])
      return
    }
    let alive = true
    void runtimesApi
      .list()
      .then((l) => alive && setMissing(l.filter((r) => r.recommended && !r.installed)))
      .catch(() => alive && setMissing([]))
    // Never hold the splash on a slow answer.
    const t = window.setTimeout(() => alive && setMissing((m) => m ?? []), 2500)
    return () => {
      alive = false
      window.clearTimeout(t)
    }
  }, [])
  return missing
}

/** The groups a list names, the way the prompt says them: "Visual C++, DirectX and XNA". */
function namesOf(list: Runtime[]): string {
  const groups = [...new Set(list.map((r) => r.group))]
  return groups.length > 1 ? `${groups.slice(0, -1).join(', ')} and ${groups.at(-1)}` : (groups[0] ?? '')
}

/**
 * On the start screen, under the mark: offer to install the missing runtimes.
 * Install runs it right there in the ASCII bar (Windows asks once); Later
 * asks again in a week; Don't ask again leaves it to Settings. While it runs,
 * Cancel stops it and Continue goes on into the app with it still running.
 */
export function RuntimesPrompt({ missing, onDone }: { missing: Runtime[]; onDone: () => void }) {
  const progress = useRuntimesProgress()
  const [started, setStarted] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const mb = missing.filter((r) => !r.cached).reduce((n, r) => n + r.mb, 0)

  // Finished cleanly: a moment to read it, then on into the app.
  useEffect(() => {
    if (!started || progress?.phase !== 'done') return
    const t = window.setTimeout(onDone, 1400)
    return () => window.clearTimeout(t)
  }, [started, progress?.phase, onDone])

  if (started && progress && progress.phase !== 'idle') {
    return (
      <RuntimesProgressView compact progress={progress} onCancel={() => void runtimesApi.cancel()}>
        {busy(progress) ? (
          <Button size="sm" variant="ghost" onClick={onDone}>
            Continue
          </Button>
        ) : progress.phase !== 'done' ? (
          <Button size="sm" onClick={onDone}>
            Continue
          </Button>
        ) : null}
      </RuntimesProgressView>
    )
  }

  return (
    <div className="grid justify-items-center gap-3 text-center">
      <p className="text-[10px] font-bold uppercase tracking-[0.3em] text-muted-foreground">Game runtimes</p>
      <p className="max-w-[17rem] text-[11px] leading-relaxed text-foreground/85">
        {missing.length === 1 ? `${missing[0]!.name} is` : `${namesOf(missing)} runtimes are`} missing. Most Windows games need {missing.length === 1 ? 'it' : 'them'} to start.
        {mb ? ` About ${mb} MB from Microsoft.` : ''}
      </p>
      {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
      <div className="flex gap-2">
        <Button
          size="sm"
          variant="primary"
          onClick={() => {
            setError(null)
            void runtimesApi
              .install(missing.map((r) => r.id), false)
              .then(() => setStarted(true))
              .catch((e: unknown) => setError(errorText(e)))
          }}
        >
          Install
        </Button>
        <Button
          size="sm"
          onClick={() => {
            askLater()
            onDone()
          }}
        >
          Later
        </Button>
      </div>
      <button
        type="button"
        onClick={() => {
          setAskAtStartup(false)
          onDone()
        }}
        className="text-[10px] text-muted-foreground underline-offset-2 hover:text-foreground hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
      >
        Don&apos;t ask again
      </button>
    </div>
  )
}
