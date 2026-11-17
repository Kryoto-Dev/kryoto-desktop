import { useCallback, useEffect, useRef, useState } from 'react'
import { Copy, FolderOpen, RotateCw, Send } from '@/ui/pixel-icons'
import { Button, Check, Section } from '@/ui'
import { call, errorText } from '@/lib/bridge'
import { library } from '@/lib/library'

/**
 * Settings > Troubleshooting: what the client wrote down, and whether errors go to
 * kryo.to. The same file is in the log folder for attaching to a support
 * message.
 */
export function LogsPane({ sendReports, onSendReports }: { sendReports: boolean; onSendReports: (v: boolean) => void }) {
  const [text, setText] = useState('')
  const [folder, setFolder] = useState('')
  const [note, setNote] = useState<string | null>(null)
  const box = useRef<HTMLPreElement | null>(null)
  const last = useRef('')

  // The 3 s poll follows new lines only while the box is at the bottom, and leaves
  // a selection in it alone. Refresh always goes to the newest lines.
  const refresh = useCallback(async (force = false) => {
    const t = await call<string>('logs_tail', { lines: 400 }).catch((e: unknown) => `Could not read the log: ${errorText(e)}`)
    if (t === last.current && !force) return
    const el = box.current
    const sel = document.getSelection()
    if (!force && el && sel && !sel.isCollapsed && sel.containsNode(el, true)) return
    const stick = force || !el || el.scrollHeight - el.scrollTop - el.clientHeight < 24
    // Scrolled up, the lines the tail drops off the top for new ones come off scrollTop
    // too, so the lines being read stay put. Rows are reused in place, so the browser's
    // scroll anchoring has nothing to hold on to.
    const rows = el?.children
    const gone = rows ? last.current.split('\n').indexOf(t.split('\n', 1)[0] ?? '') : -1
    const up = !stick && rows && gone > 0 && gone < rows.length ? rows[gone]!.getBoundingClientRect().top - rows[0]!.getBoundingClientRect().top : 0
    last.current = t
    setText(t)
    requestAnimationFrame(() => {
      if (!box.current) return
      if (stick) box.current.scrollTop = box.current.scrollHeight
      else if (up) box.current.scrollTop -= up
    })
  }, [])
  useEffect(() => {
    void refresh()
    void call<string>('logs_folder').then(setFolder).catch(() => {})
    const t = window.setInterval(() => void refresh(), 3000)
    return () => window.clearInterval(t)
  }, [refresh])

  const lines = text ? text.split('\n') : []
  return (
    <>
      <Section title="Error reports" hint="Sends error details, app version, OS, your signed-in username and a persistent app install ID to Kryoto. Error details may include game names and file paths.">
        <Check checked={sendReports} onChange={onSendReports} label="Send error reports to kryo.to" />
      </Section>
      <Section title="Log">
        <pre
          ref={box}
          className="kryo-radius kryo-ascii-art m-0 h-72 select-text overflow-auto border border-border bg-background p-3 text-[10.5px] leading-relaxed"
        >
          {lines.length ? (
            lines.map((l, i) => (
              <div key={i} className={/ ERROR | CRASH /.test(l) ? 'text-destructive' : / WARN /.test(l) ? 'text-warning' : 'text-muted-foreground'}>
                {l}
              </div>
            ))
          ) : (
            <span className="text-muted-foreground">Nothing logged yet.</span>
          )}
        </pre>
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" onClick={() => library.openFolder(folder, true)} disabled={!folder}>
            <FolderOpen className="size-3" />
            Open log folder
          </Button>
          <Button
            size="sm"
            onClick={() =>
              void navigator.clipboard
                .writeText(text)
                .then(() => setNote('Copied.'))
                .catch(() => setNote('Could not copy.'))
            }
          >
            <Copy className="size-3" />
            Copy
          </Button>
          <Button size="sm" onClick={() => refresh(true)}>
            <RotateCw className="size-3" />
            Refresh
          </Button>
          <Button
            size="sm"
            disabled={!sendReports}
            onClick={() =>
              void call<number>('logs_send')
                .then((n) => setNote(n ? `Sent ${n} report${n === 1 ? '' : 's'}.` : 'Nothing waiting to send.'))
                .catch((e: unknown) => setNote(errorText(e)))
            }
          >
            <Send className="size-3" />
            Send now
          </Button>
          {note ? <span className="text-[11px] text-muted-foreground">{note}</span> : null}
        </div>
      </Section>
    </>
  )
}
