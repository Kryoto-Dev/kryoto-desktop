import { useRef, useState } from 'react'
import { errorText } from '@/lib/bridge'
import { chatReport, here, type ChatMessage } from '@/lib/chat'
import { EmojiButton, RichTextarea } from '@/rich/RichEditor'
import { plainRich } from '@/rich/rich-text'
import { Button, Caption, Check, Dropdown, Modal } from '@/ui'

const REASONS = [
  { value: 'spam', label: 'Spam' },
  { value: 'harassment', label: 'Harassment or threats' },
  { value: 'scam', label: 'Scam or phishing' },
  { value: 'inappropriate', label: 'Inappropriate content' },
  { value: 'other', label: 'Something else' },
] as const
type Reason = (typeof REASONS)[number]['value']

/**
 * Report messages to Kryoto staff. Chat is end-to-end encrypted, so staff can
 * only see what you choose to send from here; it becomes a support ticket.
 */
export function ReportDialog({
  peer,
  messages,
  around,
  onClose,
  onDone,
  conversation,
}: {
  peer: { id: string; name: string }
  /** "g:<id>" when reporting someone in a group. */
  conversation?: string
  messages: ChatMessage[]
  /** The message "Report" was pressed on: it and a few before it start ticked. */
  around: string
  onClose: () => void
  onDone: (blocked: boolean) => void
}) {
  const usable = messages.filter((m) => !m.deleted).slice(-30)
  const at = usable.findIndex((m) => m.msgId === around)
  const [picked, setPicked] = useState<Set<string>>(
    () => new Set(usable.slice(Math.max(0, at - 5), at + 1).map((m) => m.msgId)),
  )
  const [reason, setReason] = useState<Reason>('spam')
  const [note, setNote] = useState('')
  const box = useRef<HTMLTextAreaElement>(null)
  const [block, setBlock] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const toggle = (id: string) =>
    setPicked((s) => {
      const next = new Set(s)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })

  const send = async () => {
    setBusy(true)
    setError(null)
    try {
      await chatReport(peer.id, [...picked], reason, note, block, conversation)
      onDone(block)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal
      title={`Report ${peer.name}`}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="danger" disabled={busy || picked.size === 0} onClick={() => send()}>
            Send report
          </Button>
        </>
      }
    >
      <p className="text-xs text-muted-foreground">
        Your chats are end-to-end encrypted, so Kryoto staff cannot read them. The messages you tick below leave{' '}
        {here()} and go to staff as a support ticket; nothing else does. {peer.name} is not told.
      </p>
      <div className="grid gap-1.5">
        <Caption>Why</Caption>
        <Dropdown value={reason} onChange={setReason} options={[...REASONS]} label="Reason" />
      </div>
      <div className="grid gap-1.5">
        <Caption>Messages to send ({picked.size})</Caption>
        <ul className="kryo-radius grid max-h-56 gap-1 overflow-auto border border-border p-2">
          {usable.map((m) => (
            <li key={m.msgId}>
              <Check
                checked={picked.has(m.msgId)}
                onChange={() => toggle(m.msgId)}
                label={
                  <span className="text-xs">
                    <b>{m.outgoing ? 'You' : peer.name}:</b>{' '}
                    {m.kind === 'gif' ? 'GIF' : m.kind === 'invite' ? 'Game invite' : m.kind === 'file' ? 'A file' : plainRich(m.body).slice(0, 140)}
                  </span>
                }
              />
            </li>
          ))}
        </ul>
      </div>
      <div className="flex items-end gap-2">
        <RichTextarea
          ref={box}
          value={note}
          onValueChange={setNote}
          maxLength={500}
          wrapperClassName="grow"
          rows={2}
          placeholder="Anything staff should know (optional)"
          aria-label="Note for staff"
          className="kryo-radius block w-full resize-none border border-border bg-background px-3 py-2 text-sm text-foreground outline-none focus:border-foreground"
        />
        <EmojiButton target={box} value={note} onValueChange={setNote} stickers={false} />
      </div>
      <Check checked={block} onChange={setBlock} label={`Also block ${peer.name}`} />
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </Modal>
  )
}
