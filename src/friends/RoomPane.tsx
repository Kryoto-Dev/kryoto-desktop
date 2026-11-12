import { TeamName } from '@/ui/TeamName'
import { useCallback, useEffect, useRef, useState } from 'react'
import { ArrowLeft, Globe, Reply, Send, Trash2, X } from '@/ui/pixel-icons'
import { artSrc } from '@/lib/art'
import { call, errorText } from '@/lib/bridge'
import { Button, IconButton } from '@/ui'
import { cn } from '@/lib/utils'
import { RichText } from '@/rich/RichText'
import { EmojiButton, RichTextarea, type MentionCandidate } from '@/rich/RichEditor'
import { jumpToMessage, messageAnchor } from '@/rich/jump'
import { plainRich } from '@/rich/rich-text'

/** People for the room's `@` list: anybody with an account kryo.to will name. */
const findPeople = (q: string) =>
  q ? call<{ people: MentionCandidate[] }>('chat_mention_search', { query: q }).then((r) => r.people ?? []) : Promise.resolve([] as MentionCandidate[])

type RoomMessage = {
  id: string
  body: string
  /** The message this answers; `gone` when it was removed or cannot be shown. */
  replyTo?: { id: string; gone: boolean; name: string | null; username: string | null; excerpt: string } | null
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
export function RoomPane({
  myUsername,
  onProfile,
  onBack,
  meSupporter = false,
  meRank = meSupporter ? 'supporter' : null,
}: {
  myUsername: string
  onProfile: (username: string) => void
  onBack?: () => void
  meSupporter?: boolean
  /** Your donation rank: which rank emoji and stickers you are offered (kryo.to checks them on save). */
  meRank?: string | null
}) {
  const [messages, setMessages] = useState<RoomMessage[]>([])
  const [canModerate, setCanModerate] = useState(false)
  const [text, setText] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [replyTo, setReplyTo] = useState<RoomMessage | null>(null)
  const last = useRef<string | null>(null)
  const bottom = useRef<HTMLDivElement>(null)
  const box = useRef<HTMLTextAreaElement>(null)

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

  const send = async (said?: string) => {
    const value = (said ?? text).trim()
    if (!value || value.length > MAX || busy) return
    setBusy(true)
    setError(null)
    try {
      await call('room_post', { text: value, replyTo: replyTo?.id ?? null })
      if (said === undefined) setText('')
      setReplyTo(null)
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
    <section className="grid h-full min-h-0 min-w-0 grid-rows-[auto_minmax(0,1fr)_auto] overflow-hidden">
      <header className={cn('flex items-center gap-3 border-b border-border py-3', onBack ? 'px-3' : 'px-5')}>
        {onBack ? (
          <IconButton label="Back to the list" onClick={onBack}>
            <ArrowLeft className="size-4" />
          </IconButton>
        ) : null}
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
            <div key={m.id} id={messageAnchor(m.id)} className="group flex items-start gap-2.5">
              <button type="button" onClick={() => onProfile(m.user.username)} className="shrink-0" aria-label={`${name}'s profile`}>
                {m.user.avatarUrl ? (
                  <img decoding="async" src={artSrc(m.user.avatarUrl) ?? undefined} alt="" className="kryo-pill size-7 object-cover" />
                ) : (
                  <span className="kryo-pill grid size-7 place-items-center bg-secondary text-[10px] font-bold">{name.slice(0, 1).toUpperCase()}</span>
                )}
              </button>
              <div className="grid min-w-0 gap-0.5">
                {m.replyTo ? (
                  m.replyTo.gone ? (
                    <span className="kryo-reply-quote">
                      <Reply className="size-3 shrink-0" aria-hidden />
                      <span className="italic">Replying to a message that was removed</span>
                    </span>
                  ) : (
                    <span className="kryo-reply-quote">
                      <Reply className="size-3 shrink-0" aria-hidden />
                      <span className="sr-only">Replying to</span>
                      <button type="button" onClick={() => m.replyTo?.username && onProfile(m.replyTo.username)} className="shrink-0 font-bold text-foreground hover:underline">
                        {m.replyTo.name}
                      </button>
                      <button
                        type="button"
                        onClick={() => m.replyTo && jumpToMessage(m.replyTo.id)}
                        className="min-w-0 truncate text-left hover:text-foreground"
                        aria-label="Go to the message this answers"
                      >
                        <RichText text={m.replyTo.excerpt} />
                      </button>
                    </span>
                  )
                ) : null}
                <span className="flex items-center gap-2 text-[11px]">
                  <button type="button" onClick={() => onProfile(m.user.username)} className={cn('font-bold hover:underline', mine ? 'text-primary' : 'text-foreground')}>
                    <TeamName username={m.user.username}>{name}</TeamName>
                  </button>
                  <span className="text-muted-foreground">
                    {new Date(m.createdAt).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}
                  </span>
                  <IconButton
                    label="Reply"
                    className="size-5 opacity-0 group-hover:opacity-100 focus:opacity-100 pointer-coarse:opacity-100"
                    onClick={() => {
                      setReplyTo(m)
                      box.current?.focus()
                    }}
                  >
                    <Reply className="size-3" />
                  </IconButton>
                  {mine || canModerate ? (
                    <IconButton label="Remove message" className="size-5 opacity-0 group-hover:opacity-100 focus:opacity-100 pointer-coarse:opacity-100" onClick={() => remove(m.id)}>
                      <Trash2 className="size-3" />
                    </IconButton>
                  ) : null}
                </span>
                {/* The room is not encrypted: kryo.to stored its mentions as ids,
                    unwrapped effects the writer does not own, and resolved the names. */}
                <p className="whitespace-pre-wrap break-words text-sm text-foreground">
                  <RichText text={m.body} />
                </p>
              </div>
            </div>
          )
        })}
        <div ref={bottom} />
      </div>
      <footer className="grid gap-2 border-t border-border px-5 py-3">
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
        {replyTo ? (
          <div className="flex items-center justify-between gap-2 text-[11px] text-muted-foreground">
            <span className="min-w-0 truncate">
              Replying to <b className="text-foreground">{replyTo.user.displayName || replyTo.user.username}</b>: {plainRich(replyTo.body).slice(0, 80)}
            </span>
            <IconButton label="Cancel" className="size-6" onClick={() => setReplyTo(null)}>
              <X className="size-3" />
            </IconButton>
          </div>
        ) : null}
        <div className="flex items-end gap-2">
          <RichTextarea
            ref={box}
            value={text}
            onValueChange={(v) => setText(v.slice(0, MAX))}
            people={findPeople}
            effects={meSupporter}
            rank={meRank}
            wrapperClassName="grow"
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                void send()
              }
            }}
            rows={1}
            placeholder="Message everyone"
            aria-label="Message the public room"
            className="kryo-radius block max-h-32 min-h-9 w-full resize-none border border-border bg-background px-3 py-2 text-sm text-foreground outline-none placeholder:text-muted-foreground focus:border-foreground"
          />
          <EmojiButton rank={meRank} target={box} value={text} onValueChange={(v) => setText(v.slice(0, MAX))} onPick={(token) => void send(token)} />
          <Button variant="primary" size="sm" disabled={!text.trim() || busy} onClick={() => send()} aria-label="Send">
            <Send className="size-3" />
          </Button>
        </div>
      </footer>
    </section>
  )
}
