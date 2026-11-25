import { TeamName } from '@/ui/TeamName'
import { useCallback, useEffect, useRef, useState } from 'react'
import { ArrowLeft, Globe, Pencil, Reply, Send, SmilePlus, Trash2, X } from '@/ui/pixel-icons'
import { artSrc } from '@/lib/art'
import { call, errorText } from '@/lib/bridge'
import { QUICK_REACTIONS } from '@/lib/chat'
import { Button, IconButton } from '@/ui'
import { cn } from '@/lib/utils'
import { RichText } from '@/rich/RichText'
import { EmojiButton, RichTextarea, type MentionCandidate } from '@/rich/RichEditor'
import { EmojiGlyph } from '@/rich/EmojiGlyph'
import { jumpToMessage, messageAnchor } from '@/rich/jump'
import { editableText } from '@/rich/mentions'
import { isEmojiName } from '@/rich/rich-catalog'
import { plainRich } from '@/rich/rich-text'
import { applyRoomRead, canEditRoomMessage, type RoomMessage, type RoomReaction, type RoomRead } from './room-feed'

/** People for the room's `@` list: anybody with an account kryo.to will name. */
const findPeople = (q: string) =>
  q ? call<{ people: MentionCandidate[] }>('chat_mention_search', { query: q }).then((r) => r.people ?? []) : Promise.resolve([] as MentionCandidate[])

const MAX = 500
const POLL_MS = 4000

/**
 * The public room: one chat for everyone. Unlike every other chat here it is
 * NOT end-to-end encrypted - kryo.to keeps the messages for 30 days and
 * moderators can remove them - and the header says so.
 *
 * You can change your own message for a day (it then reads "edited"), remove
 * it, and react to any message with kryo.to's own reactions (the same ones as
 * on write-ups and comments: kryo.to decides everything). The room is joined
 * with #kryoto-chat on the Kryoto Discord, so some messages were written
 * there, by somebody whose Discord is linked to the account shown.
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
  const [editing, setEditing] = useState<RoomMessage | null>(null)
  const [reactingTo, setReactingTo] = useState<string | null>(null)
  const last = useRef<string | null>(null)
  /** kryo.to's clock at the last read: what changed on older messages since then comes with the next one. */
  const since = useRef<string | null>(null)
  const bottom = useRef<HTMLDivElement>(null)
  const box = useRef<HTMLTextAreaElement>(null)

  const poll = useCallback(async () => {
    const r = await call<RoomRead>('room_list', { after: last.current, since: since.current })
    setCanModerate(r.canModerate)
    if (r.now) since.current = r.now
    const newest = r.messages[r.messages.length - 1]
    if (newest) last.current = newest.id
    if (newest || r.changed?.length || r.removed?.length) setMessages((list) => applyRoomRead(list, r))
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

  // Starting a reply or an edit puts you in the message box.
  useEffect(() => {
    if (replyTo || editing) box.current?.focus()
  }, [replyTo, editing])

  const stopEditing = () => {
    setEditing(null)
    setText('')
  }

  const send = async (said?: string) => {
    const value = (said ?? text).trim()
    if (!value || value.length > MAX || busy) return
    setBusy(true)
    setError(null)
    try {
      if (editing && said === undefined) {
        const r = await call<{ message: RoomMessage }>('room_edit', { id: editing.id, text: value })
        setMessages((list) => list.map((m) => (m.id === r.message.id ? r.message : m)))
        stopEditing()
      } else {
        await call('room_post', { text: value, replyTo: replyTo?.id ?? null })
        if (said === undefined) setText('')
        setReplyTo(null)
        await poll()
      }
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  const remove = (id: string) =>
    void call('room_delete', { id })
      .then(() => {
        setMessages((list) => list.filter((m) => m.id !== id))
        if (editing?.id === id) stopEditing()
      })
      .catch((e) => setError(errorText(e)))

  const react = (m: RoomMessage, emoji: string) => {
    const mine = m.reactions?.find((r) => r.emoji === emoji)?.mine ?? false
    setReactingTo(null)
    void call<{ reactions: RoomReaction[] }>('room_react', { id: m.id, emoji, on: !mine })
      .then((r) => setMessages((list) => list.map((x) => (x.id === m.id ? { ...x, reactions: r.reactions ?? [] } : x))))
      .catch((e) => setError(errorText(e)))
  }

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
          const reactions = m.reactions ?? []
          const actions = 'size-5 opacity-0 group-hover:opacity-100 focus:opacity-100 pointer-coarse:opacity-100'
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
                <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px]">
                  <button type="button" onClick={() => onProfile(m.user.username)} className={cn('font-bold hover:underline', mine ? 'text-primary' : 'text-foreground')}>
                    <TeamName username={m.user.username}>{name}</TeamName>
                  </button>
                  <span className="text-muted-foreground">
                    {new Date(m.createdAt).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}
                  </span>
                  {m.editedAt ? <span className="text-muted-foreground">(edited)</span> : null}
                  {m.origin === 'discord' ? <span className="text-muted-foreground">from Discord</span> : null}
                  <IconButton label="Reply" className={actions} onClick={() => { setEditing(null); setReplyTo(m) }}>
                    <Reply className="size-3" />
                  </IconButton>
                  <IconButton label="React" className={actions} onClick={() => setReactingTo(reactingTo === m.id ? null : m.id)}>
                    <SmilePlus className="size-3" />
                  </IconButton>
                  {mine && canEditRoomMessage(m, Date.now()) ? (
                    <IconButton
                      label="Edit"
                      className={actions}
                      onClick={() => {
                        setReplyTo(null)
                        setEditing(m)
                        setText(editableText(m.body).slice(0, MAX))
                      }}
                    >
                      <Pencil className="size-3" />
                    </IconButton>
                  ) : null}
                  {mine || canModerate ? (
                    <IconButton label="Remove message" className={actions} onClick={() => remove(m.id)}>
                      <Trash2 className="size-3" />
                    </IconButton>
                  ) : null}
                </span>
                {/* The room is not encrypted: kryo.to stored its mentions as ids,
                    unwrapped effects the writer does not own, and resolved the names. */}
                <p className="whitespace-pre-wrap break-words text-sm text-foreground">
                  <RichText text={m.body} />
                </p>
                {reactions.length > 0 ? (
                  <span className="mt-0.5 flex flex-wrap gap-2" role="group" aria-label="Reactions">
                    {reactions.map((r) => (
                      <button
                        key={r.emoji}
                        type="button"
                        aria-pressed={r.mine}
                        title={`${r.people.map((p) => p.name).join(', ') || 'Somebody'} reacted with ${isEmojiName(r.emoji) ? `:${r.emoji}:` : r.emoji}`}
                        onClick={() => react(m, r.emoji)}
                        className={cn(
                          'kryo-pill inline-flex h-8 items-center gap-1 border px-2 text-xs tabular-nums transition-colors',
                          r.mine ? 'border-primary bg-primary/15 text-foreground' : 'border-border text-muted-foreground hover:text-foreground',
                        )}
                      >
                        <EmojiGlyph emoji={r.emoji} size={20} />
                        {r.count}
                      </button>
                    ))}
                  </span>
                ) : null}
                {reactingTo === m.id ? (
                  <span className="kryo-pill mt-0.5 flex w-fit max-w-full flex-wrap gap-1 border border-border bg-card px-2 py-1">
                    {QUICK_REACTIONS.map((e) => (
                      <button key={e} type="button" className="kryo-radius inline-flex size-8 items-center justify-center text-lg hover:bg-secondary" onClick={() => react(m, e)} aria-label={`React ${e}`}>
                        <span className="kryo-emoji-char">{e}</span>
                      </button>
                    ))}
                    {/* Everything else, from the same picker as the message box. */}
                    <EmojiButton standard stickers={false} rank={meRank} align="left" onPick={(token, kind) => react(m, kind === 'char' ? token : token.slice(1, -1))} />
                  </span>
                ) : null}
              </div>
            </div>
          )
        })}
        <div ref={bottom} />
      </div>
      <footer className="grid gap-2 border-t border-border px-5 py-3">
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
        {replyTo || editing ? (
          <div className="flex items-center justify-between gap-2 text-[11px] text-muted-foreground">
            <span className="min-w-0 truncate">
              {editing ? (
                'Editing your message'
              ) : (
                <>
                  Replying to <b className="text-foreground">{replyTo?.user.displayName || replyTo?.user.username}</b>: {plainRich(replyTo?.body ?? '').slice(0, 80)}
                </>
              )}
            </span>
            <IconButton label="Cancel" className="size-6" onClick={() => (editing ? stopEditing() : setReplyTo(null))}>
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
              } else if (e.key === 'Escape' && editing) {
                e.preventDefault()
                stopEditing()
              }
            }}
            rows={1}
            placeholder="Message everyone"
            aria-label={editing ? 'Edit your message' : 'Message the public room'}
            className="kryo-radius block max-h-32 min-h-9 w-full resize-none border border-border bg-background px-3 py-2 text-sm text-foreground outline-none placeholder:text-muted-foreground focus:border-foreground"
          />
          <EmojiButton rank={meRank} target={box} value={text} onValueChange={(v) => setText(v.slice(0, MAX))} onPick={editing ? undefined : (token) => void send(token)} />
          <Button variant="primary" size="sm" disabled={!text.trim() || busy} onClick={() => send()} aria-label={editing ? 'Save' : 'Send'}>
            <Send className="size-3" />
          </Button>
        </div>
      </footer>
    </section>
  )
}
