import { useCallback, useEffect, useRef, useState } from 'react'
import { Globe, Send, Trash2 } from 'lucide-react'
import { artSrc } from '@/lib/art'
import { call, errorText } from '@/lib/bridge'
import { Button, IconButton } from '@/ui'
import { cn } from '@/lib/utils'

type RoomMessage = {
  id: string
  body: string
  createdAt: string
  user: { id: string; username: string; displayName: string | null; avatarUrl: string | null; supporter: boolean }
}

const MAX = 500
const POLL_MS = 4000

/**
 * The public room: one chat for everyone. Unlike every other chat here it is
 * NOT end-to-end encrypted - kryo.to keeps the messages for 30 days and
 * moderators can remove them - and the header says so.
 */
export function RoomPane({ myUsername, onProfile }: { myUsername: string; onProfile: (username: string) => void }) {
  const [messages, setMessages] = useState<RoomMessage[]>([])
  const [canModerate, setCanModerate] = useState(false)
  const [text, setText] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const last = useRef<string | null>(null)
  const bottom = useRef<HTMLDivElement>(null)

  const poll = useCallback(async () => {
    const r = await call<{ messages: RoomMessage[]; canModerate: boolean }>('room_list', { after: last.current })
    setCanModerate(r.canModerate)
    const newest = r.messages[r.messages.length - 1]
    if (!newest) return
    last.current = newest.id
    setMessages((list) => {
      const seen = new Set(list.map((m) => m.id))
      return [...list, ...r.messages.filter((m) => !seen.has(m.id))].slice(-300)
    })
  }, [])

  useEffect(() => {
    let alive = true
    let timer: number | undefined
    const tick = () =>
      void poll()
        .then(() => alive && setError(null))
        .catch((e) => alive && setError(errorText(e)))
        .finally(() => {
          if (alive) timer = window.setTimeout(tick, POLL_MS)
        })
    tick()
    return () => {
      alive = false
      window.clearTimeout(timer)
    }
  }, [poll])

  useEffect(() => {
    bottom.current?.scrollIntoView({ block: 'end' })
  }, [messages.length])

  const send = async () => {
    const value = text.trim()
    if (!value || value.length > MAX || busy) return
    setBusy(true)
    setError(null)
    try {
      await call('room_post', { text: value })
      setText('')
      await poll()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  const remove = (id: string) =>
    void call('room_delete', { id })
      .then(() => setMessages((list) => list.filter((m) => m.id !== id)))
      .catch((e) => setError(errorText(e)))

  return (
    <section className="grid min-h-0 grid-rows-[auto_1fr_auto]">
      <header className="flex items-center gap-3 border-b border-border px-5 py-3">
        <Globe className="size-4 text-muted-foreground" aria-hidden />
        <div className="grid">
          <b className="text-sm text-foreground">Public room</b>
          <span className="text-[10px] uppercase tracking-wider text-muted-foreground">
            Everyone can read this. Not end-to-end encrypted; moderated by Kryoto staff.
          </span>
        </div>
      </header>
      <div className="grid min-h-0 content-start gap-2 overflow-auto px-5 py-4" aria-live="polite">
        {messages.length === 0 ? <p className="py-10 text-center text-xs text-muted-foreground">Quiet in here. Say hi.</p> : null}
        {messages.map((m) => {
          const name = m.user.displayName || m.user.username
          const mine = m.user.username === myUsername
          return (
            <div key={m.id} className="group flex items-start gap-2.5">
              <button type="button" onClick={() => onProfile(m.user.username)} className="shrink-0" aria-label={`${name}'s profile`}>
                {m.user.avatarUrl ? (
                  <img decoding="async" src={artSrc(m.user.avatarUrl) ?? undefined} alt="" className="kryo-pill size-7 object-cover" />
                ) : (
                  <span className="kryo-pill grid size-7 place-items-center bg-secondary text-[10px] font-bold">{name.slice(0, 1).toUpperCase()}</span>
                )}
              </button>
              <div className="grid min-w-0 gap-0.5">
                <span className="flex items-center gap-2 text-[11px]">
                  <button type="button" onClick={() => onProfile(m.user.username)} className={cn('font-bold hover:underline', mine ? 'text-primary' : 'text-foreground')}>
                    {name}
                  </button>
                  <span className="text-muted-foreground">
                    {new Date(m.createdAt).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}
                  </span>
                  {mine || canModerate ? (
                    <IconButton label="Remove message" className="size-5 opacity-0 group-hover:opacity-100 focus:opacity-100" onClick={() => remove(m.id)}>
                      <Trash2 className="size-3" />
                    </IconButton>
                  ) : null}
                </span>
                <p className="whitespace-pre-wrap break-words text-sm text-foreground">{m.body}</p>
              </div>
            </div>
          )
        })}
        <div ref={bottom} />
      </div>
      <footer className="grid gap-2 border-t border-border px-5 py-3">
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
        <div className="flex items-end gap-2">
          <textarea
            value={text}
            onChange={(e) => setText(e.target.value.slice(0, MAX))}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                void send()
              }
            }}
            rows={1}
            placeholder="Message everyone"
            aria-label="Message the public room"
            className="kryo-radius max-h-32 min-h-9 grow resize-none border border-border bg-background px-3 py-2 text-sm text-foreground outline-none placeholder:text-muted-foreground focus:border-foreground"
          />
          <Button variant="primary" size="sm" disabled={!text.trim() || busy} onClick={() => void send()} aria-label="Send">
            <Send className="size-3" />
          </Button>
        </div>
      </footer>
    </section>
  )
}
