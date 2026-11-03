import { ProfileLink, TeamName } from '@/ui/TeamName'
import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  AlertCircle,
  ArrowLeft,
  Check,
  CheckCheck,
  Clock,
  Copy,
  Gamepad2,
  ImagePlay,
  Paperclip,
  Phone,
  PhoneMissed,
  Download,
  Flag,
  Lock,
  Pencil,
  Reply,
  Send,
  Settings2,
  ShieldAlert,
  ShieldCheck,
  Shield,
  SmilePlus,
  Trash2,
  Users,
  X,
} from 'lucide-react'
import { errorText, on } from '@/lib/bridge'
import {
  chatDelete,
  chatEdit,
  chatGifSearch,
  chatIdentityAck,
  chatMarkRead,
  chatMessages,
  chatReact,
  chatSend,
  chatSendGif,
  chatSettingsGet,
  chatTyping,
  chatVerifyInfo,
  chatVerifyMark,
  parseGif,
  parseInvite,
  parseFile,
  parseCall,
  callLine,
  chatSendFile,
  chatFileSave,
  chatFilePreview,
  type FileInfo,
  QUICK_REACTIONS,
  textLimit,
  type ChatMessage,
  type ChatSettings,
  type Gif,
  type GroupView,
  type Invite,
  type VerifyInfo,
  groupConversationId,
} from '@/lib/chat'
import type { Friend } from '@/hooks/useFriends'
import { InvitePicker, type InviteGame } from './InvitePicker'
import { ChatSettingsDialog } from './ChatSettings'
import { ReportDialog } from './ReportDialog'
import { startCall } from '@/lib/calls'
import { Button, Caption, ContextMenu, IconButton, Modal, type MenuEntry } from '@/ui'
import { cn } from '@/lib/utils'
import { artSrc } from '@/lib/art'

const EDIT_WINDOW_MS = 24 * 60 * 60 * 1000
const TYPING_REPEAT_MS = 4000

/** The 1:1 conversation id, as km-core builds it: "dm:" + both ids, smaller first. */
export function dmConversationId(a: string, b: string): string {
  const [lo, hi] = BigInt(a) <= BigInt(b) ? [BigInt(a), BigInt(b)] : [BigInt(b), BigInt(a)]
  const hex = (n: bigint) => n.toString(16).padStart(16, '0')
  return `646d3a${hex(lo)}${hex(hi)}`
}

function time(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
}

/** The divider over a new day's messages: Today, Yesterday, or the date. */
function dayLabel(ms: number): string {
  const d = new Date(ms)
  const start = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime()
  const days = Math.round((start(new Date()) - start(d)) / 86_400_000)
  if (days === 0) return 'Today'
  if (days === 1) return 'Yesterday'
  return d.toLocaleDateString(undefined, { weekday: days < 7 ? 'long' : undefined, day: 'numeric', month: 'long', year: d.getFullYear() === new Date().getFullYear() ? undefined : 'numeric' })
}

function StatusIcon({ status }: { status: ChatMessage['status'] }) {
  const cls = 'size-3'
  switch (status) {
    case 'sending':
      return <Clock className={cls} aria-label="Sending" />
    case 'sent':
      return <Check className={cls} aria-label="Sent" />
    case 'delivered':
      return <CheckCheck className={cls} aria-label="Delivered" />
    case 'read':
      return <CheckCheck className={cn(cls, 'text-primary')} aria-label="Read" />
    case 'failed':
      return <AlertCircle className={cn(cls, 'text-destructive')} aria-label="Not sent" />
    default:
      return null
  }
}

function GifView({ gif, auto }: { gif: Gif; auto: boolean }) {
  const [load, setLoad] = useState(auto)
  const w = Math.min(gif.width || 240, 260)
  const h = gif.width ? Math.round((gif.height / gif.width) * w) : 180
  if (!load) {
    return (
      <button
        type="button"
        onClick={() => setLoad(true)}
        className="kryo-radius grid place-items-center border border-dashed border-border text-[11px] text-muted-foreground"
        style={{ width: w, height: h }}
        title="Load it from the GIF provider (they see your IP address)"
      >
        GIF: {gif.title || 'tap to load'}
      </button>
    )
  }
  return <img decoding="async" src={artSrc(gif.url) ?? undefined} alt={gif.title || 'GIF'} width={w} height={h} className="kryo-radius object-cover" loading="lazy" />
}

function size(n: number): string {
  if (n < 1000) return `${n} B`
  if (n < 1000 * 1000) return `${(n / 1000).toFixed(0)} KB`
  return `${(n / 1000 / 1000).toFixed(1)} MB`
}

/** A file in a message: images show inline (decrypted here), anything can be saved. */
function FileCard({ msgId, file }: { msgId: string; file: FileInfo }) {
  const image = file.mime.startsWith('image/') && file.size <= 10 * 1024 * 1024
  const [src, setSrc] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  useEffect(() => {
    if (!image) return
    let alive = true
    void chatFilePreview(msgId)
      .then((s) => alive && setSrc(s))
      .catch((e) => alive && setError(errorText(e)))
    return () => {
      alive = false
    }
  }, [msgId, image])
  return (
    <span className="grid gap-2">
      {src ? <img decoding="async" src={src} alt={file.name} className="kryo-radius max-h-72 max-w-full object-contain" /> : null}
      <span className="flex items-center gap-3">
        <span className="grid min-w-0">
          <b className="truncate text-xs">{file.name}</b>
          <span className="text-[10px] text-muted-foreground">{size(file.size)}{saved ? ' - saved' : ''}</span>
        </span>
        <Button
          size="sm"
          onClick={() =>
            void chatFileSave(msgId)
              .then((w) => w && setSaved(true))
              .catch((e) => setError(errorText(e)))
          }
        >
          <Download className="size-3" aria-hidden /> Save
        </Button>
      </span>
      {error ? <span className="text-[11px] text-destructive">{error}</span> : null}
    </span>
  )
}

function InviteCard({
  invite,
  outgoing,
  name,
  cover,
  onJoin,
}: {
  invite: Invite
  outgoing: boolean
  name: string
  cover: string | null
  onJoin?: (invite: Invite) => void
}) {
  const expired = invite.expiresAt > 0 && Date.now() > invite.expiresAt
  return (
    <span className="grid min-w-52 gap-2">
      <span className="flex items-center gap-3">
        {cover ? (
          <img decoding="async" src={artSrc(cover) ?? undefined} alt="" className="kryo-radius h-14 w-10 object-cover" />
        ) : (
          <span className="kryo-radius grid h-14 w-10 place-items-center bg-secondary">
            <Gamepad2 className="size-4 text-muted-foreground" aria-hidden />
          </span>
        )}
        <span className="grid min-w-0">
          <span className="text-[10px] uppercase tracking-wider text-muted-foreground">
            {outgoing ? `You invited ${name}` : 'Invites you to play'}
          </span>
          <b className="truncate text-sm">{invite.title}</b>
          {invite.steamLobby ? <span className="text-[10px] text-muted-foreground">Straight into their lobby</span> : null}
        </span>
      </span>
      {outgoing ? null : expired ? (
        <span className="text-[11px] text-muted-foreground">This invite expired.</span>
      ) : (
        <Button variant="primary" size="sm" onClick={() => onJoin?.(invite)}>
          Join
        </Button>
      )}
    </span>
  )
}

/**
 * One end-to-end encrypted conversation. Everything shown here came out of
 * the Rust engine as plaintext; keys never reach this view.
 */
export function ChatPane({
  peer,
  myId,
  meSupporter,
  onProfile,
  request,
  onRespond,
  games = [],
  onJoinInvite,
  group,
  nameOf = () => 'Someone',
  usernameOf = () => null,
  onMembers,
  canCall = false,
  face,
  onBack,
}: {
  peer: { id: string; name: string; supporter: boolean }
  /** Their picture and what they are doing, for the header (people, not groups). */
  face?: Pick<Friend, 'username' | 'avatarUrl' | 'online' | 'playing'>
  /** Back to the list, where the list and the conversation take turns. */
  onBack?: () => void
  myId: string
  meSupporter: boolean
  onProfile: () => void
  /**
   * Not a friend: `incoming` is their message request waiting for your
   * answer, `outgoing` yours waiting for theirs.
   */
  request?: 'incoming' | 'outgoing'
  onRespond?: (accept: boolean) => Promise<void>
  /** Library games to invite to, and what "Join" on an invite does. */
  games?: InviteGame[]
  onJoinInvite?: (invite: Invite) => void
  /** A group chat: `peer.id` is then "g:<id>" and `peer.name` the group's name. */
  group?: GroupView
  /** Names of the people writing in a group. */
  nameOf?: (userId: string) => string
  usernameOf?: (userId: string) => string | null
  onMembers?: () => void
  /** Voice calls are rolled out to this account. */
  canCall?: boolean
}) {
  const conversationId = useMemo(() => (group ? groupConversationId(group.id) : dmConversationId(myId, peer.id)), [group, myId, peer.id])
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [text, setText] = useState('')
  const [replyTo, setReplyTo] = useState<ChatMessage | null>(null)
  const [editing, setEditing] = useState<ChatMessage | null>(null)
  const [deleting, setDeleting] = useState<ChatMessage | null>(null)
  const [reactingTo, setReactingTo] = useState<string | null>(null)
  const [typing, setTyping] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [settings, setSettings] = useState<ChatSettings | null>(null)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [gifOpen, setGifOpen] = useState(false)
  const [answering, setAnswering] = useState(false)
  const [verify, setVerify] = useState<VerifyInfo | null>(null)
  const [verifyOpen, setVerifyOpen] = useState(false)
  const [inviting, setInviting] = useState(false)
  const [reporting, setReporting] = useState<string | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  // Right-click on a message: what its hover buttons do, and copying it.
  const messageMenu = (m: ChatMessage, canChange: boolean): MenuEntry[] => [
    { label: 'Reply', icon: <Reply />, onSelect: () => setReplyTo(m) },
    { label: 'React', icon: <SmilePlus />, onSelect: () => setReactingTo(m.msgId) },
    ...(m.kind === 'text' ? [{ label: 'Copy text', icon: <Copy />, onSelect: () => void navigator.clipboard?.writeText(m.body).catch(() => {}) }] : []),
    ...(canChange && m.kind === 'text'
      ? [{ label: 'Edit', icon: <Pencil />, onSelect: () => {
          setEditing(m)
          setReplyTo(null)
          setText(m.body)
        } }]
      : []),
    ...(canChange || !m.outgoing ? [{ separator: true } as const] : []),
    ...(canChange ? [{ label: 'Delete for everyone', icon: <Trash2 />, danger: true, onSelect: () => setDeleting(m) }] : []),
    ...(!m.outgoing ? [{ label: 'Report to Kryoto staff', icon: <Flag />, danger: true, onSelect: () => setReporting(m.msgId) }] : []),
  ]
  const waiting = request === 'incoming'
  const bottom = useRef<HTMLDivElement>(null)
  const box = useRef<HTMLTextAreaElement>(null)
  const typingSent = useRef(0)
  const typingStop = useRef<number | undefined>(undefined)
  const limit = textLimit(meSupporter)

  const markRead = useCallback(() => {
    if (document.hasFocus()) void chatMarkRead(peer.id).catch(() => {})
  }, [peer.id])

  // Their security key: verified or not, and whether it just changed.
  const loadVerify = useCallback(() => {
    // In a group each member is verified in your one-to-one chat with them.
    if (group) return
    void chatVerifyInfo(peer.id).then(setVerify).catch(() => {})
  }, [peer.id, group])
  useEffect(() => {
    setVerify(null)
    loadVerify()
    const stop = on<{ userId: string }>('chat-identity-changed', (e) => e.userId === peer.id && loadVerify())
    return () => void stop.then((f) => f())
  }, [peer.id, loadVerify])
  const keyChanged = verify?.pendingChange ?? false

  // Load, and follow along.
  useEffect(() => {
    let alive = true
    setMessages([])
    setReplyTo(null)
    setEditing(null)
    setTyping(false)
    void chatMessages(peer.id)
      .then((m) => {
        if (!alive) return
        setMessages(m)
        markRead()
      })
      .catch((e) => alive && setError(errorText(e)))
    void chatSettingsGet().then((s) => alive && setSettings(s)).catch(() => {})
    let typingTimer: number | undefined
    const stops = [
      on<ChatMessage>('chat-message', (m) => {
        if (m.conversationId !== conversationId) return
        setMessages((list) => (list.some((x) => x.msgId === m.msgId) ? list : [...list, m]))
        if (!m.outgoing) {
          setTyping(false)
          markRead()
        }
      }),
      on<ChatMessage>('chat-updated', (m) => {
        if (m.conversationId !== conversationId) return
        setMessages((list) => list.map((x) => (x.msgId === m.msgId ? m : x)))
      }),
      on<{ conversationId: string; active: boolean }>('chat-typing', (t) => {
        if (t.conversationId !== conversationId) return
        setTyping(t.active)
        window.clearTimeout(typingTimer)
        if (t.active) typingTimer = window.setTimeout(() => setTyping(false), 6000)
      }),
    ]
    const onFocus = () => markRead()
    window.addEventListener('focus', onFocus)
    return () => {
      alive = false
      window.clearTimeout(typingTimer)
      window.removeEventListener('focus', onFocus)
      stops.forEach((p) => void p.then((f) => f()))
    }
  }, [peer.id, conversationId, markRead])

  useEffect(() => {
    bottom.current?.scrollIntoView({ block: 'end' })
  }, [messages.length, typing])

  // Starting a reply or an edit puts you in the message box, at the end.
  useEffect(() => {
    if (!replyTo && !editing) return
    const el = box.current
    if (!el) return
    el.focus()
    el.setSelectionRange(el.value.length, el.value.length)
  }, [replyTo, editing])

  const stopTyping = useCallback(() => {
    if (typingSent.current) {
      typingSent.current = 0
      void chatTyping(peer.id, false).catch(() => {})
    }
  }, [peer.id])

  const onType = (value: string) => {
    setText(value)
    if (editing) return
    const now = Date.now()
    if (value.trim() && now - typingSent.current > TYPING_REPEAT_MS) {
      typingSent.current = now
      void chatTyping(peer.id, true).catch(() => {})
    }
    window.clearTimeout(typingStop.current)
    typingStop.current = window.setTimeout(stopTyping, TYPING_REPEAT_MS)
  }

  const submit = async () => {
    const value = text.trim()
    if (!value || value.length > limit) return
    setError(null)
    window.clearTimeout(typingStop.current)
    stopTyping()
    try {
      if (editing) {
        const m = await chatEdit(editing.msgId, value)
        setMessages((list) => list.map((x) => (x.msgId === m.msgId ? m : x)))
        setEditing(null)
      } else {
        const m = await chatSend(peer.id, value, replyTo?.msgId)
        setMessages((list) => (list.some((x) => x.msgId === m.msgId) ? list.map((x) => (x.msgId === m.msgId ? m : x)) : [...list, m]))
        setReplyTo(null)
      }
      setText('')
    } catch (e) {
      setError(errorText(e))
    }
  }

  const toggleReaction = (m: ChatMessage, emoji: string) => {
    const mine = m.reactions.find((r) => r.emoji === emoji)?.userIds.includes(myId) ?? false
    setReactingTo(null)
    void chatReact(m.msgId, emoji, mine).catch((e) => setError(errorText(e)))
  }


  const byId = useMemo(() => new Map(messages.map((m) => [m.msgId, m])), [messages])
  const counter = limit - text.length

  return (
    <section className="grid min-h-0 grid-rows-[auto_1fr_auto]">
      <header className={cn('flex items-center justify-between gap-3 border-b border-border py-3', onBack ? 'px-3' : 'px-5')}>
        <div className="flex min-w-0 items-center gap-3">
          {onBack ? (
            <IconButton label="Back to the list" onClick={onBack}>
              <ArrowLeft className="size-4" />
            </IconButton>
          ) : null}
          {group ? (
            <span className="kryo-pill grid size-8 shrink-0 place-items-center bg-secondary">
              <Users className="size-4 text-muted-foreground" aria-hidden />
            </span>
          ) : (
            <button type="button" onClick={onProfile} className="kryo-pill relative shrink-0" title={`${peer.name}'s profile`} aria-label={`${peer.name}'s profile`}>
              {face?.avatarUrl ? (
                <img decoding="async" src={artSrc(face.avatarUrl) ?? undefined} alt="" className="kryo-pill size-8 object-cover" />
              ) : (
                <span className="kryo-pill grid size-8 place-items-center bg-secondary text-xs font-bold text-foreground">{peer.name.slice(0, 1).toUpperCase()}</span>
              )}
              {face?.playing || face?.online ? (
                <span className={cn('absolute -bottom-0.5 -right-0.5 size-2.5 rounded-full border-2 border-background', face.playing ? 'bg-primary' : 'bg-success')} aria-hidden />
              ) : null}
            </button>
          )}
          <div className="grid min-w-0">
            <ProfileLink username={group ? null : face?.username} onProfile={onProfile} className="truncate text-sm font-bold text-foreground hover:underline focus-visible:outline-2 focus-visible:outline-primary"><TeamName username={group ? null : face?.username}>{peer.name}</TeamName></ProfileLink>
            <span className="flex min-w-0 items-center gap-1 text-[10px] uppercase tracking-wider text-muted-foreground">
              {face?.playing ? (
                <span className="truncate normal-case tracking-normal text-foreground/80">Playing {face.playing.title}</span>
              ) : (
                <>
                  <Lock className="size-2.5 shrink-0" aria-hidden />
                  <span className="truncate">{group ? `${group.members.length} people · end-to-end encrypted` : 'End-to-end encrypted'}</span>
                </>
              )}
            </span>
          </div>
        </div>
        <div className="flex gap-1">
          {group ? (
            <IconButton label={`Members (${group.members.length})`} onClick={() => onMembers?.()}>
              <Users className="size-4" />
            </IconButton>
          ) : (
            <>
              {canCall && !request ? (
                <IconButton label={`Call ${peer.name}`} onClick={() => void startCall(peer.id).catch((e) => setError(errorText(e)))}>
                  <Phone className="size-4" />
                </IconButton>
              ) : null}
              <IconButton label={verify?.verified ? `${peer.name} is verified` : `Verify ${peer.name}`} onClick={() => setVerifyOpen(true)}>
                {keyChanged ? (
                  <ShieldAlert className="size-4 text-destructive" />
                ) : verify?.verified ? (
                  <ShieldCheck className="size-4 text-success" />
                ) : (
                  <Shield className="size-4" />
                )}
              </IconButton>
            </>
          )}
          <IconButton label="Chat settings" onClick={() => setSettingsOpen(true)}>
            <Settings2 className="size-4" />
          </IconButton>
        </div>
      </header>

      <div className="grid min-h-0 content-start gap-2 overflow-auto px-5 py-4" aria-live="polite">
        {messages.length === 0 ? (
          <p className="py-10 text-center text-xs text-muted-foreground">
            {group ? 'Say hi. Only the people in this group can read it.' : `Say hi. Only you and ${peer.name} can read this conversation.`}
          </p>
        ) : null}
        {messages.map((m, i) => {
          const newDay = i === 0 || dayLabel(messages[i - 1]!.sentAt) !== dayLabel(m.sentAt)
          const quoted = m.replyTo ? byId.get(m.replyTo) : null
          const gif = m.kind === 'gif' && !m.deleted ? parseGif(m.body) : null
          const invite = m.kind === 'invite' && !m.deleted ? parseInvite(m.body) : null
          const file = m.kind === 'file' && !m.deleted ? parseFile(m.body) : null
          const canChange = m.outgoing && !m.deleted && Date.now() - m.sentAt < EDIT_WINDOW_MS
          return (
            <Fragment key={m.msgId}>
            {newDay ? (
              <div className="flex items-center gap-3 py-1 text-[10px] uppercase tracking-wider text-muted-foreground" role="separator" aria-label={dayLabel(m.sentAt)}>
                <span className="h-px grow bg-border" aria-hidden />
                {dayLabel(m.sentAt)}
                <span className="h-px grow bg-border" aria-hidden />
              </div>
            ) : null}
            {m.kind === 'call' ? (
              // A finished call: who called and how it went, between the messages.
              <div className="flex items-center justify-center gap-1.5 py-1 text-[11px] text-muted-foreground">
                {parseCall(m.body)?.answered ? <Phone className="size-3" aria-hidden /> : <PhoneMissed className="size-3 text-destructive" aria-hidden />}
                <span>{(() => { const c = parseCall(m.body); return c ? callLine(c, myId, peer.name) : 'Call' })()}</span>
                <span aria-hidden>·</span>
                <span>{time(m.sentAt)}</span>
              </div>
            ) : (
            <div
              className={cn('group flex', m.outgoing ? 'justify-end' : 'justify-start')}
              onContextMenu={(e) => {
                // A link or a picture keeps the system's own menu (open, save).
                if (m.deleted || waiting || (e.target as HTMLElement).closest('a,img,video')) return
                e.preventDefault()
                setMenu({ x: e.clientX, y: e.clientY, items: messageMenu(m, canChange) })
              }}
            >
              <div className={cn('grid max-w-[75%] gap-1', m.outgoing ? 'justify-items-end' : 'justify-items-start')}>
                {group && !m.outgoing ? <ProfileLink username={usernameOf(m.senderUser)} className="text-[10px] font-bold text-muted-foreground hover:underline focus-visible:outline-2 focus-visible:outline-primary"><TeamName username={usernameOf(m.senderUser)}>{nameOf(m.senderUser)}</TeamName></ProfileLink> : null}
                {quoted ? (
                  <span className="truncate border-l-2 border-border pl-2 text-[11px] text-muted-foreground">
                    {quoted.deleted ? 'Message deleted' : quoted.kind === 'gif' ? 'GIF' : quoted.kind === 'file' ? `File: ${parseFile(quoted.body)?.name ?? ''}` : quoted.kind === 'invite' ? `Game invite: ${parseInvite(quoted.body)?.title ?? ''}` : quoted.body.slice(0, 80)}
                  </span>
                ) : null}
                <div
                  className={cn(
                    'kryo-radius whitespace-pre-wrap break-words px-3 py-2 text-sm',
                    m.outgoing ? 'bg-secondary text-foreground' : 'border border-border bg-card text-foreground',
                    m.deleted && 'italic text-muted-foreground',
                  )}
                >
                  {m.deleted ? (
                    'Message deleted'
                  ) : gif ? (
                    <GifView gif={gif} auto={settings?.gifsAuto ?? true} />
                  ) : file ? (
                    <FileCard msgId={m.msgId} file={file} />
                  ) : invite ? (
                    <InviteCard invite={invite} outgoing={m.outgoing} name={peer.name} cover={games.find((g) => g.slug === invite.slug)?.cover ?? null} onJoin={onJoinInvite} />
                  ) : (
                    m.body
                  )}
                </div>
                <span className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
                  {time(m.sentAt)}
                  {m.editedAt && !m.deleted ? <span>(edited)</span> : null}
                  {m.outgoing && !(group && ['sent', 'delivered', 'read'].includes(m.status)) ? <StatusIcon status={m.status} /> : null}
                </span>
                {m.reactions.length > 0 ? (
                  <span className="flex flex-wrap gap-1">
                    {m.reactions.map((r) => (
                      <button
                        key={r.emoji}
                        type="button"
                        onClick={() => toggleReaction(m, r.emoji)}
                        className={cn(
                          'kryo-pill border px-1.5 text-xs',
                          r.userIds.includes(myId) ? 'border-primary' : 'border-border',
                        )}
                        aria-label={`${r.emoji} ${r.userIds.length}`}
                      >
                        {r.emoji} {r.userIds.length > 1 ? r.userIds.length : ''}
                      </button>
                    ))}
                  </span>
                ) : null}
                {!m.deleted && !waiting ? (
                  <span className="flex gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
                    <IconButton label="Reply" className="size-6" onClick={() => setReplyTo(m)}>
                      <Reply className="size-3" />
                    </IconButton>
                    <IconButton label="React" className="size-6" onClick={() => setReactingTo(reactingTo === m.msgId ? null : m.msgId)}>
                      <SmilePlus className="size-3" />
                    </IconButton>
                    {canChange && m.kind === 'text' ? (
                      <IconButton
                        label="Edit"
                        className="size-6"
                        onClick={() => {
                          setEditing(m)
                          setReplyTo(null)
                          setText(m.body)
                        }}
                      >
                        <Pencil className="size-3" />
                      </IconButton>
                    ) : null}
                    {canChange ? (
                      <IconButton label="Delete for everyone" className="size-6" onClick={() => setDeleting(m)}>
                        <Trash2 className="size-3" />
                      </IconButton>
                    ) : null}
                    {!m.outgoing ? (
                      <IconButton label="Report to Kryoto staff" className="size-6" onClick={() => setReporting(m.msgId)}>
                        <Flag className="size-3" />
                      </IconButton>
                    ) : null}
                  </span>
                ) : null}
                {reactingTo === m.msgId ? (
                  <span className="kryo-pill flex gap-1 border border-border bg-card px-2 py-1">
                    {QUICK_REACTIONS.map((e) => (
                      <button key={e} type="button" className="text-base" onClick={() => toggleReaction(m, e)} aria-label={`React ${e}`}>
                        {e}
                      </button>
                    ))}
                  </span>
                ) : null}
              </div>
            </div>
            )}
            </Fragment>
          )
        })}
        {typing ? <p className="text-[11px] text-muted-foreground">{group ? 'Someone' : peer.name} is typing...</p> : null}
        {notice ? <p className="text-center text-[11px] text-muted-foreground">{notice}</p> : null}
        <div ref={bottom} />
      </div>

      {waiting ? (
        <footer className="grid gap-3 border-t border-border px-5 py-4">
          {error ? <p className="text-xs text-destructive">{error}</p> : null}
          <div className="grid gap-1">
            <b className="text-sm text-foreground">{peer.name} wants to message you</b>
            <p className="text-xs text-muted-foreground">
              You are not friends. They will not see that you read anything, and cannot see you typing, unless you accept.
            </p>
          </div>
          <div className="flex gap-2">
            <Button
              variant="primary"
              size="sm"
              disabled={answering}
              onClick={() => {
                setAnswering(true)
                setError(null)
                void onRespond?.(true)
                  .catch((e) => setError(errorText(e)))
                  .finally(() => setAnswering(false))
              }}
            >
              Accept
            </Button>
            <Button
              size="sm"
              disabled={answering}
              onClick={() => {
                setAnswering(true)
                setError(null)
                void onRespond?.(false)
                  .catch((e) => setError(errorText(e)))
                  .finally(() => setAnswering(false))
              }}
            >
              Decline
            </Button>
          </div>
        </footer>
      ) : keyChanged ? (
        <footer className="grid gap-3 border-t border-destructive/60 bg-destructive/10 px-5 py-4">
          <div className="grid gap-1">
            <b className="flex items-center gap-2 text-sm text-foreground">
              <ShieldAlert className="size-4 text-destructive" aria-hidden /> {peer.name}'s security key changed
            </b>
            <p className="text-xs text-muted-foreground">
              They may have reinstalled or started a new chat identity. If you did not expect this, check with them
              another way before you go on. Nothing is sent until you choose.
            </p>
          </div>
          {error ? <p className="text-xs text-destructive">{error}</p> : null}
          <div className="flex gap-2">
            <Button
              variant="primary"
              size="sm"
              onClick={() =>
                void chatIdentityAck(peer.id)
                  .then(loadVerify)
                  .catch((e) => setError(errorText(e)))
              }
            >
              I understand, continue
            </Button>
            <Button size="sm" onClick={() => setVerifyOpen(true)}>
              About verifying
            </Button>
          </div>
        </footer>
      ) : (
      <footer className="grid gap-2 border-t border-border px-5 py-3">
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
        {request === 'outgoing' ? (
          <p className="text-[11px] text-muted-foreground">
            You are not friends: {peer.name} sees this as a message request until they accept.
          </p>
        ) : null}
        {replyTo || editing ? (
          <div className="flex items-center justify-between gap-2 text-[11px] text-muted-foreground">
            <span className="truncate">
              {editing ? 'Editing your message' : `Replying to: ${replyTo?.kind === 'gif' ? 'GIF' : replyTo?.kind === 'invite' ? 'a game invite' : replyTo?.body.slice(0, 80)}`}
            </span>
            <IconButton
              label="Cancel"
              className="size-6"
              onClick={() => {
                setReplyTo(null)
                if (editing) setText('')
                setEditing(null)
              }}
            >
              <X className="size-3" />
            </IconButton>
          </div>
        ) : null}
        <div className="flex items-end gap-2">
          <textarea
            ref={box}
            value={text}
            onChange={(e) => onType(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                void submit()
              }
            }}
            rows={Math.min(6, Math.max(1, text.split('\n').length))}
            placeholder={`Message ${peer.name}`}
            aria-label={`Message ${peer.name}`}
            className="kryo-radius max-h-40 min-h-9 grow resize-none border border-border bg-background px-3 py-2 text-sm text-foreground outline-none placeholder:text-muted-foreground focus:border-foreground"
          />
          <IconButton
            label="Send a file (up to 25 MB)"
            onClick={() =>
              void chatSendFile(peer.id)
                .then((m) => m && setMessages((list) => (list.some((x) => x.msgId === m.msgId) ? list : [...list, m])))
                .catch((e) => setError(errorText(e)))
            }
          >
            <Paperclip className="size-4" />
          </IconButton>
          <IconButton label="Invite to a game" onClick={() => setInviting(true)}>
            <Gamepad2 className="size-4" />
          </IconButton>
          {meSupporter ? (
            <IconButton label="Send a GIF" onClick={() => setGifOpen(true)}>
              <ImagePlay className="size-4" />
            </IconButton>
          ) : null}
          <Button variant="primary" size="sm" disabled={!text.trim() || counter < 0} onClick={() => void submit()} aria-label="Send">
            <Send className="size-3" />
          </Button>
        </div>
        {counter < 200 ? (
          <Caption className={cn('text-right', counter < 0 && 'text-destructive')}>
            {counter} characters left{meSupporter ? '' : ' (supporters get 8,000)'}
          </Caption>
        ) : null}
      </footer>
      )}

      {menu ? <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} /> : null}
      {reporting ? (
        <ReportDialog
          peer={
            group
              ? (() => {
                  const sender = messages.find((x) => x.msgId === reporting)?.senderUser ?? ''
                  return { id: sender, name: nameOf(sender) }
                })()
              : peer
          }
          conversation={group ? peer.id : undefined}
          messages={group ? messages.filter((x) => x.outgoing || x.senderUser === messages.find((y) => y.msgId === reporting)?.senderUser) : messages}
          around={reporting}
          onClose={() => setReporting(null)}
          onDone={(blocked) => {
            setReporting(null)
            setNotice(blocked ? `Report sent and ${peer.name} is blocked.` : 'Report sent. Staff will reply in your notifications.')
          }}
        />
      ) : null}

      {inviting ? (
        <InvitePicker
          peer={peer}
          games={games}
          onClose={() => setInviting(false)}
          onSent={() => setInviting(false)}
        />
      ) : null}

      {verifyOpen ? (
        <Modal
          title={`Verify ${peer.name}`}
          onClose={() => setVerifyOpen(false)}
          footer={
            verify && verify.safetyNumber.length > 0 && !keyChanged ? (
              <Button
                variant={verify.verified ? 'outline' : 'primary'}
                onClick={() =>
                  void chatVerifyMark(peer.id, !verify.verified)
                    .then(loadVerify)
                    .catch((e) => setError(errorText(e)))
                }
              >
                {verify.verified ? 'Remove verification' : 'Mark as verified'}
              </Button>
            ) : (
              <Button onClick={() => setVerifyOpen(false)}>Close</Button>
            )
          }
        >
          {keyChanged ? (
            <p className="text-sm text-foreground">
              Accept the new key first (in the conversation), then compare the new safety number with {peer.name}.
            </p>
          ) : verify && verify.safetyNumber.length > 0 ? (
            <>
              <p className="text-sm text-foreground">
                Compare these numbers with {peer.name}, in person or on a call. If they match on both screens, nobody
                is listening in between.
              </p>
              <div className="kryo-radius grid grid-cols-4 gap-x-4 gap-y-2 border border-border bg-background p-4 font-mono text-base tracking-wider text-foreground">
                {verify.safetyNumber.map((g, i) => (
                  <span key={i}>{g}</span>
                ))}
              </div>
              <p className="flex items-center gap-2 text-xs text-muted-foreground">
                {verify.verified ? (
                  <>
                    <ShieldCheck className="size-3.5 text-success" aria-hidden /> You verified {peer.name}. If their key
                    ever changes, you will be told and this is cleared.
                  </>
                ) : (
                  'Not verified yet. Messages are encrypted either way; verifying proves it is really them.'
                )}
              </p>
            </>
          ) : (
            <p className="text-sm text-foreground">
              Their key is not known yet. It appears once {peer.name} has chat turned on and you have exchanged a message.
            </p>
          )}
        </Modal>
      ) : null}

      {deleting ? (
        <Modal
          title="Delete for everyone?"
          onClose={() => setDeleting(null)}
          footer={
            <>
              <Button onClick={() => setDeleting(null)}>Cancel</Button>
              <Button
                variant="danger"
                onClick={() => {
                  const m = deleting
                  setDeleting(null)
                  void chatDelete(m.msgId).catch((e) => setError(errorText(e)))
                }}
              >
                Delete
              </Button>
            </>
          }
        >
          <p className="text-sm text-foreground">It disappears for {peer.name} too, on every device.</p>
        </Modal>
      ) : null}

      {settingsOpen ? (
        <ChatSettingsDialog
          available
          calls={canCall}
          onClose={() => {
            setSettingsOpen(false)
            // GIFs-straight-away may have changed under this conversation.
            void chatSettingsGet().then(setSettings).catch(() => {})
          }}
        />
      ) : null}

      {gifOpen ? (
        <GifPicker
          onClose={() => setGifOpen(false)}
          onPick={(g) => {
            setGifOpen(false)
            void chatSendGif(peer.id, g).catch((e) => setError(errorText(e)))
          }}
        />
      ) : null}
    </section>
  )
}

/** GIF search through kryo.to (supporters only, decided there). */
function GifPicker({ onClose, onPick }: { onClose: () => void; onPick: (g: Gif) => void }) {
  const [query, setQuery] = useState('')
  const [gifs, setGifs] = useState<Gif[]>([])
  const [attribution, setAttribution] = useState<string | undefined>()
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    const q = query.trim()
    const t = window.setTimeout(() => {
      setError(null)
      void chatGifSearch(q)
        .then((r) => {
          setGifs(r.gifs)
          setAttribution(r.attribution)
        })
        .catch((e) => setError(errorText(e)))
    }, q ? 350 : 0)
    return () => window.clearTimeout(t)
  }, [query])
  return (
    <Modal title="Send a GIF" onClose={onClose} wide>
      <input
        autoFocus
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="Search GIFs"
        aria-label="Search GIFs"
        className="kryo-pill h-9 w-full border border-border bg-background px-4 text-xs text-foreground outline-none placeholder:text-muted-foreground focus:border-foreground"
      />
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
      <div className="grid grid-cols-3 gap-2 sm:grid-cols-4">
        {gifs.map((g) => (
          <button key={g.id} type="button" onClick={() => onPick(g)} className="kryo-radius overflow-hidden border border-border" title={g.title}>
            <img decoding="async" src={artSrc(g.url) ?? undefined} alt={g.title || 'GIF'} loading="lazy" className="aspect-square w-full object-cover" />
          </button>
        ))}
      </div>
      {attribution ? <Caption className="text-right">{attribution}</Caption> : null}
    </Modal>
  )
}
