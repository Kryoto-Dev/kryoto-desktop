import { useState } from 'react'
import { AtSign, Check, Copy, Link2, UserPlus } from 'lucide-react'
import { errorText } from '@/lib/bridge'
import { friendAction } from '@/lib/friends'
import { QrCode } from '@/settings/QrCode'
import { Button, Caption, Modal } from '@/ui'

/**
 * Adding a friend without leaving the friends list: by username, or with an
 * invite link (and its QR code) for someone whose username you do not know.
 */
export function AddFriendDialog({ onClose, initial = '' }: { onClose: () => void; initial?: string }) {
  return (
    <Modal title="Add a friend" onClose={onClose}>
      <AddFriendForm initial={initial} autoFocus />
      <InviteLink />
    </Modal>
  )
}

/** The username form on its own, for the empty friends list too. */
export function AddFriendForm({ initial = '', autoFocus = false }: { initial?: string; autoFocus?: boolean }) {
  const [username, setUsername] = useState(initial)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null)
  const send = async () => {
    const name = username.trim().replace(/^@/, '')
    if (!name || busy) return
    setBusy(true)
    setResult(null)
    try {
      const r = await friendAction({ kind: 'request', username: name })
      setResult({ ok: true, text: r.status === 'friends' ? `You and ${name} are friends now.` : `Request sent to ${name}.` })
      setUsername('')
    } catch (e) {
      setResult({ ok: false, text: errorText(e) })
    } finally {
      setBusy(false)
    }
  }
  return (
    <form
      className="grid gap-2"
      onSubmit={(e) => {
        e.preventDefault()
        void send()
      }}
    >
      <label htmlFor="add-friend-username">
        <Caption>Their username</Caption>
      </label>
      <div className="flex gap-2">
        <span className="kryo-pill flex h-9 grow items-center gap-2 border border-border bg-background px-3 focus-within:border-foreground">
          <AtSign className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
          <input
            id="add-friend-username"
            autoFocus={autoFocus}
            value={username}
            onChange={(e) => {
              setUsername(e.target.value)
              setResult(null)
            }}
            placeholder="username"
            autoComplete="off"
            spellCheck={false}
            className="min-w-0 grow bg-transparent text-xs text-foreground outline-none placeholder:text-muted-foreground"
          />
        </span>
        <Button type="submit" variant="primary" disabled={busy || !username.trim()}>
          <UserPlus className="size-3.5" aria-hidden />
          Send request
        </Button>
      </div>
      <p aria-live="polite" className={result ? (result.ok ? 'text-xs text-success' : 'text-xs text-destructive') : 'text-[11px] text-muted-foreground'}>
        {result ? result.text : 'The name after the @ on their profile. They can accept or decline.'}
      </p>
    </form>
  )
}

function InviteLink() {
  const [link, setLink] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [copied, setCopied] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const make = async () => {
    setBusy(true)
    setError(null)
    try {
      const r = await friendAction({ kind: 'invite' })
      if (typeof r.url !== 'string') throw new Error('kryo.to sent no link.')
      setLink(r.url)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }
  const copy = () => {
    if (!link) return
    void navigator.clipboard.writeText(link).then(() => {
      setCopied(true)
      window.setTimeout(() => setCopied(false), 2000)
    })
  }
  return (
    <div className="grid gap-3 border-t border-border pt-4">
      <Caption>Or share an invite link</Caption>
      {link ? (
        <div className="flex items-start gap-4">
          <QrCode text={link} label="QR code for your invite link" className="kryo-radius size-28 shrink-0" />
          <div className="grid min-w-0 gap-2">
            <code className="kryo-radius block truncate border border-border bg-background px-3 py-2 text-[11px] text-foreground">{link}</code>
            <div>
              <Button size="sm" onClick={copy}>
                {copied ? <Check className="size-3" aria-hidden /> : <Copy className="size-3" aria-hidden />}
                {copied ? 'Copied' : 'Copy link'}
              </Button>
            </div>
            <p className="text-[11px] text-muted-foreground">
              Anyone signed in to kryo.to who opens it becomes your friend. It works for 7 days, for up to 5 people.
            </p>
          </div>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-3">
          <Button size="sm" disabled={busy} onClick={() => void make()}>
            <Link2 className="size-3" aria-hidden />
            Make an invite link
          </Button>
          <span className="text-[11px] text-muted-foreground">For someone whose username you do not know.</span>
        </div>
      )}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
  )
}
