import { useEffect, useRef, useState } from 'react'
import { Camera, Inbox, MessageSquarePlus, Paperclip, Send, Sparkles, TriangleAlert, X } from '@/ui/pixel-icons'
import { Button, Label, Modal } from '@/ui'
import { EmojiButton, RichTextarea } from '@/rich/RichEditor'
import { RichText } from '@/rich/RichText'
import { call } from '@/lib/bridge'
import { IS_PREVIEW_BUILD } from '@/lib/endpoint'
import { cn } from '@/lib/utils'

/**
 * Report a bug / Suggest something, in Kryoto Desktop Preview's title bar (the
 * same button every page of preview.kryo.to has; kryo.to docs/preview.md).
 *
 * Pressing it takes a picture of the window first (src-tauri preview_report.rs,
 * Store included), then opens a short form. The version, the OS, where in the
 * app it happened and the account go with it by themselves, and the form says
 * so. It is a submission: staff decide it in Admin > Preview and Yours shows
 * what they said.
 */

type Kind = 'bug' | 'idea' | 'yours'
type Report = { id: string; kind: 'bug' | 'idea'; what: string; status: string; staffNote: string | null; rewardKryos: number; paid: boolean; createdAt: string }

const STATUS: Record<string, string> = { new: 'Waiting', accepted: 'Accepted', fixed: 'Fixed', duplicate: 'Already known', declined: 'Not taken' }
const TYPES = ['image/png', 'image/jpeg', 'image/webp', 'image/gif', 'video/mp4', 'video/webm']
/** A box of several lines takes the reader's box corners: the one-line field's pill would make it a capsule. */
const BOX = 'kryo-radius block w-full resize-y border border-border bg-background px-3 py-2 text-sm text-foreground outline-none transition-colors placeholder:text-muted-foreground focus:border-foreground'

export function PreviewReportButton({ page, username }: { page: string; username: string | null }) {
  const [open, setOpen] = useState(false)
  const [taking, setTaking] = useState(false)
  const [shot, setShot] = useState<string | null>(null)
  if (!IS_PREVIEW_BUILD || !username) return null

  async function press() {
    setTaking(true)
    // The picture first, so it shows the app and not the form.
    const png = await call<string | null>('preview_report_screenshot').catch(() => null)
    setTaking(false)
    setShot(png)
    setOpen(true)
  }

  return (
    <>
      <button
        type="button"
        onClick={() => void press()}
        disabled={taking}
        title="Report a bug or suggest something"
        aria-label="Report a bug or suggest something"
        className="kryo-pill flex h-7 items-center gap-1.5 px-2.5 text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
      >
        <MessageSquarePlus className="size-3.5" />
        <span>{taking ? 'Taking a picture' : 'Report'}</span>
      </button>
      {open ? <ReportDialog page={page} username={username} shot={shot} dropShot={() => setShot(null)} onClose={() => setOpen(false)} /> : null}
    </>
  )
}

function ReportDialog({ page, username, shot, dropShot, onClose }: { page: string; username: string; shot: string | null; dropShot: () => void; onClose: () => void }) {
  const [kind, setKind] = useState<Kind>('bug')
  const [what, setWhat] = useState('')
  const [steps, setSteps] = useState('')
  const [files, setFiles] = useState<{ name: string; mime: string; data: string }[]>([])
  const [error, setError] = useState<string | null>(null)
  const [sent, setSent] = useState(false)
  const picker = useRef<HTMLInputElement>(null)
  const box = useRef<HTMLTextAreaElement>(null)
  const stepsBox = useRef<HTMLTextAreaElement>(null)

  async function add(list: FileList) {
    for (const f of [...list].slice(0, 4 - files.length)) {
      if (!TYPES.includes(f.type)) {
        setError('A report takes pictures and clips: PNG, JPEG, WebP, GIF, MP4 or WebM.')
        continue
      }
      if (f.size > 25 * 1024 * 1024) {
        setError(`${f.name} is larger than 25 MB.`)
        continue
      }
      const data = await new Promise<string>((resolve, reject) => {
        const r = new FileReader()
        r.onload = () => resolve(String(r.result).replace(/^data:[^,]*,/, ''))
        r.onerror = () => reject(r.error)
        r.readAsDataURL(f)
      })
      setFiles((x) => [...x, { name: f.name, mime: f.type, data }])
    }
  }

  async function send() {
    setError(null)
    if (!what.trim()) return setError(kind === 'bug' ? 'Say what happened.' : 'Say what you would like.')
    const all = [...(shot ? [{ name: 'screenshot.png', mime: 'image/png', data: shot }] : []), ...files]
    try {
      await call('preview_report_send', { report: { kind, what, steps: kind === 'bug' ? steps : '', page, files: all } })
      setSent(true)
    } catch (e) {
      setError(typeof e === 'string' ? e : e instanceof Error ? e.message : 'Could not send it.')
    }
  }

  return (
    <Modal title={kind === 'yours' ? 'What you sent' : kind === 'bug' ? 'Report a bug' : 'Suggest something'} onClose={onClose}>
      <div role="tablist" className="kryo-pill flex w-fit items-center gap-1 border border-border p-0.5">
        {(['bug', 'idea', 'yours'] as const).map((k) => (
          <button
            key={k}
            type="button"
            role="tab"
            aria-selected={kind === k}
            onClick={() => setKind(k)}
            className={cn('kryo-pill flex h-7 items-center gap-1.5 px-3 text-[11px] uppercase tracking-wider', kind === k ? 'bg-primary font-bold text-primary-foreground' : 'text-muted-foreground hover:text-foreground')}
          >
            {k === 'bug' ? <TriangleAlert className="size-3.5" /> : k === 'idea' ? <Sparkles className="size-3.5" /> : <Inbox className="size-3.5" />}
            {k === 'bug' ? 'Bug' : k === 'idea' ? 'Idea' : 'Yours'}
          </button>
        ))}
      </div>
      {kind === 'yours' ? (
        <Yours />
      ) : sent ? (
        <div className="grid justify-items-center gap-2 py-6 text-center">
          <Send className="size-5 text-muted-foreground" />
          <p className="text-sm font-bold text-foreground">Sent. Thank you.</p>
          <p className="max-w-xs text-xs text-muted-foreground">Staff read every one. You will hear when they decide, and an accepted one can earn Kryos.</p>
          <Button onClick={() => setKind('yours')}>See it under Yours</Button>
        </div>
      ) : (
        <>
          <div className="grid gap-1.5">
            <Label>{kind === 'bug' ? 'What happened?' : 'What would you like?'}</Label>
            <div className="relative">
              <RichTextarea ref={box} aria-label={kind === 'bug' ? 'What happened' : 'What you would like'} rows={4} maxLength={4000} className={BOX} value={what} onValueChange={setWhat} />
              <EmojiButton target={box} value={what} onValueChange={setWhat} className="absolute bottom-1.5 right-1.5" />
            </div>
          </div>
          {kind === 'bug' ? (
            <div className="grid gap-1.5">
              <Label>Steps to see it (if you know them)</Label>
              <div className="relative">
                <RichTextarea ref={stepsBox} aria-label="Steps to see it" rows={3} maxLength={4000} className={BOX} value={steps} onValueChange={setSteps} />
                <EmojiButton target={stepsBox} value={steps} onValueChange={setSteps} className="absolute bottom-1.5 right-1.5" />
              </div>
            </div>
          ) : null}
          <div className="grid gap-1.5">
            <Label>Picture</Label>
            {shot ? (
              <div className="kryo-radius relative overflow-hidden border border-border">
                <img src={`data:image/png;base64,${shot}`} alt="Kryoto Desktop when you pressed Report" className="block max-h-40 w-full object-cover object-top" />
                <div className="absolute inset-x-0 bottom-0 flex items-center gap-2 bg-background/85 px-2 py-1 text-[11px] text-muted-foreground">
                  <span className="flex-1">Taken when you pressed Report</span>
                  <button type="button" className="hover:text-foreground" onClick={dropShot} aria-label="Leave the picture out">
                    <X className="size-3" />
                  </button>
                </div>
              </div>
            ) : (
              <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Camera className="size-3.5" /> No picture of the window here. Add one of your own below if it helps.
              </p>
            )}
          </div>
          <div className="grid gap-1.5">
            <div className="flex items-center gap-2">
              <Label>Files</Label>
              <Button size="sm" onClick={() => picker.current?.click()} disabled={files.length >= 4}>
                <Paperclip className="size-3.5" /> Add
              </Button>
              <input ref={picker} type="file" multiple hidden accept={TYPES.join(',')} onChange={(e) => {
                if (e.target.files?.length) void add(e.target.files)
                e.target.value = ''
              }} />
            </div>
            {files.map((f, i) => (
              <p key={`${f.name}-${i}`} className="flex items-center gap-2 text-xs text-foreground">
                <span className="min-w-0 flex-1 truncate">{f.name}</span>
                <button type="button" className="text-muted-foreground hover:text-foreground" onClick={() => setFiles((x) => x.filter((_, j) => j !== i))} aria-label={`Remove ${f.name}`}>
                  <X className="size-3" />
                </button>
              </p>
            ))}
          </div>
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            Sent with it, so nobody has to ask: where you were ({page}), Kryoto Desktop Preview {__APP_VERSION__}, your system, and your account ({username}).
          </p>
          {error ? <p className="text-xs text-destructive" role="alert">{error}</p> : null}
          <div>
            <Button variant="primary" busyLabel="Sending..." onClick={send}>
              <Send className="size-3.5" /> {kind === 'bug' ? 'Send the report' : 'Send the suggestion'}
            </Button>
          </div>
        </>
      )}
    </Modal>
  )
}

function Yours() {
  const [reports, setReports] = useState<Report[] | null>(null)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    call<{ reports: Report[] }>('preview_reports_mine')
      .then((d) => setReports(d.reports))
      .catch(() => setFailed(true))
  }, [])
  if (failed) return <p className="text-xs text-destructive">Could not load them. Try again in a moment.</p>
  if (!reports) return <p className="text-xs text-muted-foreground">Loading...</p>
  if (!reports.length) return <p className="text-xs text-muted-foreground">Nothing sent yet. What you send shows up here with what staff decided.</p>
  return (
    <ul className="grid gap-3">
      {reports.map((r) => (
        <li key={r.id} className="grid gap-1 border-b border-border pb-3 last:border-b-0">
          <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
            {r.kind === 'bug' ? <TriangleAlert className="size-3.5" /> : <Sparkles className="size-3.5" />}
            <span>{new Date(r.createdAt).toLocaleDateString()}</span>
            <span className="grow" />
            {r.paid && r.rewardKryos > 0 ? <span className="font-bold text-foreground">+{r.rewardKryos} Kryos</span> : null}
            <span className="kryo-pill border border-border px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wider">{STATUS[r.status] ?? r.status}</span>
          </div>
          <RichText text={r.what} className="line-clamp-3 text-sm text-foreground" />
          {r.staffNote ? (
            <div className="border-l-2 border-primary bg-secondary/40 px-2.5 py-1.5 text-xs">
              <span className="mr-1 text-[10px] uppercase tracking-wider text-muted-foreground">Staff</span>
              <RichText text={r.staffNote} className="text-foreground" />
            </div>
          ) : null}
        </li>
      ))}
    </ul>
  )
}
