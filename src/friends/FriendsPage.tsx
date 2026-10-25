import { ProfileLink, TeamName } from '@/ui/TeamName'
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import {
  Ban,
  BellOff,
  Bell,
  Check,
  Ellipsis,
  Globe,
  MessageSquare,
  MessageSquarePlus,
  Phone,
  Pin,
  PinOff,
  Plus,
  Search,
  Settings2,
  Star,
  User,
  UserMinus,
  UserPlus,
  Users,
  X,
} from 'lucide-react'
import { artSrc } from '@/lib/art'
import { Button, Caption, ContextMenu, IconButton, inputCls, Label, Modal, type MenuEntry } from '@/ui'
import type { Account } from '@/hooks/useAccount'
import { friendName, useFriends, type Friend, type FriendFace } from '@/hooks/useFriends'
import { AsciiArt } from '@/ui/ascii/AsciiArt'
import { CHAT } from '@/ui/ascii/scenes'
import { ChatSetup } from './ChatSetup'
import { ChatPane } from './ChatPane'
import { AddFriendDialog, AddFriendForm } from './AddFriendDialog'
import { ChatSettingsDialog } from './ChatSettings'
import {
  chatConversations,
  chatGroups,
  chatMessageRequest,
  chatMessageRequestRespond,
  chatPeople,
  chatSetContext,
  groupConversationId,
  groupTarget,
  useChatStatus,
  parseFile,
  parseInvite,
  type ChatMessage,
  type ChatPersonInfo,
  type ChatStatus,
  type Conversation,
  type GroupView,
  type Invite,
} from '@/lib/chat'
import { startCall } from '@/lib/calls'
import { friendAction } from '@/lib/friends'
import { GroupCreateDialog, GroupMembersDialog } from './GroupDialogs'
import { RoomPane } from './RoomPane'
import type { InviteGame } from './InvitePicker'
import { call, errorText, on } from '@/lib/bridge'
import { cn } from '@/lib/utils'

/** Below this width the list and the conversation take turns (the web chat in a side panel, a phone). */
const NARROW = 720

type Menu = { x: number; y: number; items: MenuEntry[] }
type Asking = { title: string; body: string; confirm: string; run: () => Promise<unknown> }

/**
 * Friends & chat: everything social in one place, the same in Kryoto Desktop,
 * the web chat and the chat on kryo.to. Your friends by what they are doing,
 * requests to answer, people and groups to talk to, and the conversation.
 * Adding, answering, removing and blocking happen here too.
 */
export function FriendsPage({
  account,
  onProfile,
  onWeb,
  chatWith,
  games = [],
  onJoinInvite,
  list = 'friends',
}: {
  account: Account
  onProfile: () => void
  /** Open a kryo.to page (a profile, privacy settings). */
  onWeb: (path: string) => void
  /** A `kryoto://chat/<username>` link to open (`at` makes each one new). */
  chatWith?: { username: string; at: number } | null
  /** Library games that can be invited to. */
  games?: InviteGame[]
  onJoinInvite?: (invite: Invite) => void
  /**
   * What the side list shows: your friends by what they are doing (the app,
   * the web chat), or your conversations, newest first (the chat on kryo.to,
   * where the site's own Friends tab lists the friends).
   */
  list?: 'friends' | 'chats'
}) {
  const state = useFriends()
  const [search, setSearch] = useState('')
  const live = account.friends && state
  const needle = search.trim().toLowerCase()
  const matches = (f: FriendFace & { nickname?: string | null }) =>
    !needle || [f.nickname, f.displayName, f.username].some((v) => v?.toLowerCase().includes(needle))
  const favourites = live ? state.friends.filter((f) => f.favourite && matches(f)) : []
  const others = live ? state.friends.filter((f) => !f.favourite && matches(f)) : []
  const inGame = others.filter((f) => f.playing)
  const online = others.filter((f) => !f.playing && f.online)
  const offline = others.filter((f) => !f.playing && !f.online)

  // The list and the conversation side by side, or one at a time when narrow.
  const root = useRef<HTMLDivElement | null>(null)
  const [narrow, setNarrow] = useState(false)
  useLayoutEffect(() => {
    const el = root.current
    if (!el) return
    const measure = () => setNarrow(el.getBoundingClientRect().width < NARROW)
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  // Chat: who this device is in chat (kept through a short disconnect), which
  // conversation is open, and unread counts per friend.
  const chat = useChatStatus()
  const myIdRef = useRef<string | null>(null)
  if (chat?.state === 'online') myIdRef.current = chat.userId
  const chatOn = chat != null && ['online', 'connecting', 'offline'].includes(chat.state) && myIdRef.current != null
  const [selected, setSelected] = useState<string | null>(null)
  const [unread, setUnread] = useState<Record<string, number>>({})
  const [conversations, setConversations] = useState<Conversation[]>([])
  // Group chats: the list, names of members who are not friends, dialogs.
  const groupsOn = !!account.groups
  const [groups, setGroups] = useState<GroupView[]>([])
  const [extraPeople, setExtraPeople] = useState<Record<string, ChatPersonInfo>>({})
  const [creatingGroup, setCreatingGroup] = useState(false)
  const [membersOf, setMembersOf] = useState<string | null>(null)

  // Friend management: the add dialog, a row's menu, a confirmation, and a
  // short line saying what just happened.
  const [adding, setAdding] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [menu, setMenu] = useState<Menu | null>(null)
  const [asking, setAsking] = useState<Asking | null>(null)
  const [flash, setFlash] = useState<{ ok: boolean; text: string } | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  useEffect(() => {
    if (!flash) return
    const t = window.setTimeout(() => setFlash(null), 4500)
    return () => window.clearTimeout(t)
  }, [flash])

  // People who are not friends: their requests, ours, and accepted chats.
  // `opened` and `answered` cover the moments before the next report arrives.
  const [opened, setOpened] = useState<FriendFace[]>([])
  const [answered, setAnswered] = useState<Record<string, 'accepted' | 'declined'>>({})
  const [composing, setComposing] = useState(false)
  const [to, setTo] = useState('')
  const [toError, setToError] = useState<string | null>(null)
  const [openingChat, setOpeningChat] = useState(false)
  const strangers = useMemo(() => {
    if (!live) return { requests: [] as FriendFace[], chats: [] as FriendFace[], outgoing: new Set<string>() }
    const friendIds = new Set(state.friends.map((f) => f.id))
    const hidden = new Set(state.hidden)
    const keep = (p: FriendFace) => !friendIds.has(p.id) && !hidden.has(p.username.toLowerCase()) && answered[p.id] !== 'declined'
    const mr = state.messageRequests
    const requests = mr.incoming.filter((p) => keep(p) && answered[p.id] !== 'accepted')
    const seen = new Set(requests.map((p) => p.id))
    const chats: FriendFace[] = []
    for (const p of [...mr.incoming.filter((p) => answered[p.id] === 'accepted'), ...mr.accepted, ...mr.outgoing, ...opened]) {
      if (keep(p) && !seen.has(p.id)) {
        seen.add(p.id)
        chats.push(p)
      }
    }
    const accepted = new Set(mr.accepted.map((p) => p.id))
    const outgoing = new Set([...mr.outgoing, ...opened].map((p) => p.id).filter((id) => !accepted.has(id)))
    return { requests, chats, outgoing }
  }, [live, state, opened, answered])
  const requestOf = (id: string): 'incoming' | 'outgoing' | undefined =>
    strangers.requests.some((p) => p.id === id) ? 'incoming' : strangers.outgoing.has(id) ? 'outgoing' : undefined
  const peer: (FriendFace & { nickname?: string | null }) | undefined =
    live && selected
      ? (state.friends.find((f) => f.id === selected) ?? [...strangers.requests, ...strangers.chats].find((p) => p.id === selected))
      : undefined
  const openGroup = selected?.startsWith('g:') ? groups.find((g) => groupTarget(g.id) === selected) : undefined

  // A conversation that no longer exists (a friend removed, a group left) closes.
  useEffect(() => {
    if (!selected || !live || selected === 'room') return
    if (selected.startsWith('g:') ? !openGroup && groups.length > 0 : !peer) setSelected(null)
  }, [selected, live, peer, openGroup, groups.length])

  // Load groups while chat is on, and again whenever one changes.
  useEffect(() => {
    if (!chatOn || !groupsOn) return
    let alive = true
    const load = () => void chatGroups().then((g) => alive && setGroups(g)).catch(() => {})
    load()
    const stop = on('chat-group-changed', load)
    return () => {
      alive = false
      void stop.then((f) => f())
    }
  }, [chatOn, groupsOn])

  // Names for group members who are not friends (kryo.to answers only for
  // people you share a group with).
  useEffect(() => {
    if (!live || groups.length === 0) return
    const known = new Set([...state.friends.map((f) => f.id), ...Object.keys(extraPeople), myIdRef.current ?? ''])
    const missing = [...new Set(groups.flatMap((g) => g.members.map((m) => m.userId)))].filter((id) => !known.has(id))
    if (missing.length === 0) return
    void chatPeople(missing)
      .then((r) => setExtraPeople((m) => ({ ...m, ...Object.fromEntries(r.people.map((p) => [p.id, p])) })))
      .catch(() => {})
  }, [groups, live, state, extraPeople])

  const nameOf = (id: string): string => {
    const f = live ? state.friends.find((x) => x.id === id) : undefined
    if (f) return friendName(f)
    const e = extraPeople[id]
    return e ? e.displayName || e.username : 'Someone'
  }
  const usernameOf = (id: string): string | null => (live ? state.friends.find(f => f.id === id)?.username : null) ?? extraPeople[id]?.username ?? null

  // The engine learns names, supporter status and mutes from the lists.
  useEffect(() => {
    if (!live) return
    const people = [
      ...state.friends.map((f) => ({ id: f.id, name: friendName(f), supporter: f.supporter, muted: f.muted })),
      ...strangers.requests.map((p) => ({ id: p.id, name: friendName(p), supporter: p.supporter, muted: false, pending: true })),
      ...strangers.chats.map((p) => ({ id: p.id, name: friendName(p), supporter: p.supporter, muted: false })),
    ]
    const listed = new Set(people.map((p) => p.id))
    for (const e of Object.values(extraPeople)) {
      if (!listed.has(e.id)) people.push({ id: e.id, name: e.displayName || e.username, supporter: e.supporter, muted: false })
    }
    void chatSetContext(people, !!account.supporter).catch(() => {})
  }, [live, state, strangers, extraPeople, account.supporter])

  const refreshLists = () => void call('store_refresh_account').catch(() => {})

  const startChat = async () => {
    const username = to.trim().replace(/^@/, '')
    if (!username || openingChat) return
    setOpeningChat(true)
    setToError(null)
    try {
      const r = await chatMessageRequest(username)
      const face: FriendFace = { id: r.userId, username: r.username, displayName: r.name, avatarUrl: null, supporter: false }
      if (r.status !== 'friends') setOpened((list) => [face, ...list.filter((p) => p.id !== face.id)])
      setSelected(face.id)
      setComposing(false)
      setTo('')
      refreshLists()
    } catch (e) {
      setToError(errorText(e))
    } finally {
      setOpeningChat(false)
    }
  }

  // A chat link: a friend opens straight away; anyone else goes through the
  // message request (which also answers whether they can be messaged).
  const handledLink = useRef(0)
  useEffect(() => {
    if (!chatWith || handledLink.current === chatWith.at || !live || !chatOn) return
    handledLink.current = chatWith.at
    const name = chatWith.username.toLowerCase()
    const known = [...state.friends, ...strangers.requests, ...strangers.chats].find((p) => p.username.toLowerCase() === name)
    if (known) {
      setSelected(known.id)
      return
    }
    setComposing(true)
    setTo(chatWith.username)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chatWith, live, chatOn])

  const respond = async (id: string, accept: boolean) => {
    await chatMessageRequestRespond(id, accept)
    setAnswered((a) => ({ ...a, [id]: accept ? 'accepted' : 'declined' }))
    if (!accept) setSelected(null)
    refreshLists()
  }

  useEffect(() => {
    if (!chatOn) return
    let alive = true
    const refresh = () =>
      void chatConversations()
        .then((list) => {
          if (!alive) return
          // People by their id; groups by their conversation id.
          setUnread(Object.fromEntries(list.map((c) => [c.peerUserId ?? c.id, c.unread])))
          setConversations(list)
        })
        .catch(() => {})
    refresh()
    const stops = [on('chat-message', refresh), on('chat-updated', refresh), on('chat-read', refresh)]
    return () => {
      alive = false
      stops.forEach((p) => void p.then((f) => f()))
    }
  }, [chatOn, selected])

  /** Run a friends action and say how it went. */
  const act = useCallback(async (key: string, work: () => Promise<unknown>, done?: string) => {
    setBusy(key)
    try {
      await work()
      if (done) setFlash({ ok: true, text: done })
    } catch (e) {
      setFlash({ ok: false, text: errorText(e) })
    } finally {
      setBusy(null)
    }
  }, [])

  const openFriend = (f: FriendFace) => {
    if (chatOn) setSelected(f.id)
    else onWeb(`/user/${encodeURIComponent(f.username)}`)
  }
  const profileOf = (f: FriendFace) => onWeb(`/user/${encodeURIComponent(f.username)}`)

  const friendMenu = (f: Friend): MenuEntry[] => {
    const name = friendName(f)
    return [
      ...(chatOn ? [{ label: 'Message', icon: <MessageSquare />, onSelect: () => setSelected(f.id) }] : []),
      ...(chatOn && account.voice
        ? [{ label: 'Call', icon: <Phone />, onSelect: () => void startCall(f.id).catch((e) => setFlash({ ok: false, text: errorText(e) })) }]
        : []),
      { label: 'View profile', icon: <User />, onSelect: () => profileOf(f) },
      { separator: true },
      f.favourite
        ? { label: 'Unpin', icon: <PinOff />, onSelect: () => void act(f.id, () => friendAction({ kind: 'favourite', userId: f.id, on: false })) }
        : { label: 'Pin to the top', icon: <Pin />, onSelect: () => void act(f.id, () => friendAction({ kind: 'favourite', userId: f.id, on: true })) },
      f.muted
        ? { label: 'Unmute', icon: <Bell />, onSelect: () => void act(f.id, () => friendAction({ kind: 'mute', userId: f.id, on: false }), `${name} is unmuted.`) }
        : { label: 'Mute', icon: <BellOff />, hint: 'No notifications', onSelect: () => void act(f.id, () => friendAction({ kind: 'mute', userId: f.id, on: true }), `${name} is muted.`) },
      { separator: true },
      {
        label: 'Remove friend',
        icon: <UserMinus />,
        danger: true,
        onSelect: () =>
          setAsking({
            title: `Remove ${name}?`,
            body: 'You stop being friends. They are not told, and your chat history stays on this device.',
            confirm: 'Remove',
            run: () => friendAction({ kind: 'remove', userId: f.id }),
          }),
      },
      {
        label: 'Block',
        icon: <Ban />,
        danger: true,
        onSelect: () =>
          setAsking({
            title: `Block ${name}?`,
            body: 'You stop being friends, and neither of you can message or call the other. Your profile shows as "Account not found" to them. They are not told.',
            confirm: 'Block',
            run: () => friendAction({ kind: 'block', userId: f.id }),
          }),
      },
    ]
  }
  const showMenu = (f: Friend, x: number, y: number) => setMenu({ x, y, items: friendMenu(f) })

  const showList = !narrow || !selected
  const showPane = !narrow || !!selected
  const back = narrow ? () => setSelected(null) : undefined
  const noFriendsYet = live && state.friends.length === 0 && state.incoming.length === 0 && strangers.requests.length === 0
  const chatReady = chat?.state === 'online'

  return (
    <div ref={root} className={cn('grid min-h-0 min-w-0 grow', narrow ? 'grid-cols-1' : 'grid-cols-[300px_1fr]')}>
      {showList ? (
        <aside className={cn('grid min-h-0', list === 'chats' ? 'grid-rows-[auto_auto_1fr]' : 'grid-rows-[auto_auto_auto_1fr]', narrow ? 'bg-background' : 'border-r border-border bg-card/40')}>
          {list === 'friends' ? <SelfCard account={account} chat={chat} onProfile={onProfile} /> : null}
          <div className="flex gap-2 border-b border-border p-3">
            <label className={`kryo-pill flex h-8 min-w-0 grow items-center gap-2 border border-border px-3 text-[11px] text-muted-foreground focus-within:border-foreground${live ? '' : ' opacity-60'}`}>
              <Search className="size-3 shrink-0" aria-hidden />
              <input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                disabled={!live}
                placeholder={list === 'chats' ? 'Search conversations' : 'Search friends'}
                aria-label={list === 'chats' ? 'Search conversations' : 'Search friends'}
                className="min-w-0 grow bg-transparent text-foreground outline-none placeholder:text-muted-foreground"
              />
            </label>
            {live && chatOn ? (
              <IconButton label="New message" aria-expanded={composing} onClick={() => setComposing((v) => !v)}>
                <MessageSquarePlus className="size-3.5" />
              </IconButton>
            ) : null}
            {list === 'friends' ? (
              <IconButton label="Add a friend" disabled={!live} onClick={() => setAdding(true)}>
                <UserPlus className="size-3.5" />
              </IconButton>
            ) : null}
            {account.chat ? (
              <IconButton label="Chat & calls settings" onClick={() => setSettingsOpen(true)}>
                <Settings2 className="size-3.5" />
              </IconButton>
            ) : null}
          </div>
          <div>
            {composing && live && chatOn ? (
              <form
                className="grid gap-2 border-b border-border p-3"
                onSubmit={(e) => {
                  e.preventDefault()
                  void startChat()
                }}
              >
                <div className="flex items-center justify-between">
                  <Caption>New message</Caption>
                  <IconButton label="Close" className="size-6" onClick={() => setComposing(false)}>
                    <X className="size-3" />
                  </IconButton>
                </div>
                <div className="flex gap-2">
                  <input
                    autoFocus
                    value={to}
                    onChange={(e) => setTo(e.target.value)}
                    placeholder="Their username"
                    aria-label="Username"
                    className={inputCls}
                  />
                  <Button type="submit" size="sm" variant="primary" disabled={!to.trim() || openingChat}>
                    Open
                  </Button>
                </div>
                {toError ? <p className="text-[11px] text-destructive">{toError}</p> : null}
                <p className="text-[11px] text-muted-foreground">Not a friend? They get a message request, if they accept messages from everyone.</p>
              </form>
            ) : null}
            {narrow && live && !chatReady && account.chat ? (
              <div className="border-b border-border p-3">
                <ChatSetup available />
              </div>
            ) : null}
          </div>
          {live && list === 'chats' ? (
            <ChatsList
              conversations={conversations}
              needle={needle}
              faceOf={(id) => state.friends.find((f) => f.id === id) ?? [...strangers.requests, ...strangers.chats].find((p) => p.id === id) ?? extraPeople[id]}
              requests={strangers.requests}
              groups={groupsOn && chatOn ? groups : null}
              room={!!account.room && chatOn}
              chatOn={chatOn}
              unread={unread}
              selected={selected}
              myId={myIdRef.current}
              nameOf={nameOf}
              onOpen={setSelected}
              onNewGroup={() => setCreatingGroup(true)}
              onCompose={() => setComposing(true)}
            />
          ) : live ? (
            <div className="grid content-start gap-5 overflow-auto p-3">
              {state.incoming.length > 0 ? (
                <div className="grid gap-1">
                  <GroupTitle title="Friend requests" count={state.incoming.length} />
                  {state.incoming.map((r) => (
                    <div key={r.requestId} className="kryo-radius flex items-center gap-2 border border-border bg-card px-2 py-1.5">
                      <button type="button" className="min-w-0 grow text-left text-xs text-foreground hover:underline" onClick={() => profileOf(r)} title={`@${r.username}`}>
                        <FriendRow f={r} label={friendName(r)} sub="Wants to be friends" />
                      </button>
                      <IconButton
                        label={`Accept ${friendName(r)}`}
                        className="size-7 border-primary bg-primary text-primary-foreground hover:border-primary hover:text-primary-foreground hover:opacity-90"
                        disabled={busy === r.requestId}
                        onClick={() => void act(r.requestId, () => friendAction({ kind: 'respond', requestId: r.requestId, answer: 'accept' }), `You and ${friendName(r)} are friends now.`)}
                      >
                        <Check className="size-3.5" />
                      </IconButton>
                      <IconButton
                        label={`Decline ${friendName(r)}`}
                        className="size-7"
                        disabled={busy === r.requestId}
                        onClick={() => void act(r.requestId, () => friendAction({ kind: 'respond', requestId: r.requestId, answer: 'decline' }))}
                      >
                        <X className="size-3.5" />
                      </IconButton>
                    </div>
                  ))}
                </div>
              ) : null}
              {strangers.requests.length > 0 ? (
                <PeopleGroup title="Message requests" list={strangers.requests} unread={unread} selected={selected} onOpen={openFriend} />
              ) : null}
              {favourites.length > 0 ? (
                <FriendGroup title="Pinned" list={favourites} unread={unread} selected={selected} onOpen={openFriend} onMenu={showMenu} />
              ) : null}
              {inGame.length > 0 ? (
                <FriendGroup title="In game" list={inGame} unread={unread} selected={selected} onOpen={openFriend} onMenu={showMenu} />
              ) : null}
              {online.length > 0 ? (
                <FriendGroup title="Online" list={online} unread={unread} selected={selected} onOpen={openFriend} onMenu={showMenu} />
              ) : null}
              {offline.length > 0 || state.friends.length === 0 || (needle && others.length === 0) ? (
                <FriendGroup
                  title={inGame.length + online.length + favourites.length > 0 ? 'Offline' : 'Friends'}
                  list={offline}
                  empty={
                    needle
                      ? others.length === 0 && favourites.length === 0
                        ? 'Nobody matches.'
                        : undefined
                      : state.friends.length === 0
                        ? 'No friends yet.'
                        : undefined
                  }
                  unread={unread}
                  selected={selected}
                  onOpen={openFriend}
                  onMenu={showMenu}
                  action={state.friends.length === 0 && !needle ? { label: 'Add a friend', onClick: () => setAdding(true) } : undefined}
                />
              ) : null}
              {state.sent.length > 0 && !needle ? (
                <div className="grid gap-1">
                  <GroupTitle title="Sent requests" count={state.sent.length} />
                  {state.sent.map((r) => (
                    <div key={r.requestId} className="kryo-radius flex items-center gap-2 px-2 py-1">
                      <button type="button" className="min-w-0 grow text-left text-xs text-muted-foreground hover:underline" onClick={() => profileOf(r)} title={`@${r.username}`}>
                        <FriendRow f={r} label={friendName(r)} sub="Waiting for them" />
                      </button>
                      <IconButton
                        label={`Take back the request to ${friendName(r)}`}
                        className="size-6"
                        disabled={busy === r.requestId}
                        onClick={() => void act(r.requestId, () => friendAction({ kind: 'respond', requestId: r.requestId, answer: 'cancel' }), 'Request taken back.')}
                      >
                        <X className="size-3" />
                      </IconButton>
                    </div>
                  ))}
                </div>
              ) : null}
              {strangers.chats.length > 0 ? (
                <PeopleGroup title="Other chats" list={strangers.chats} unread={unread} selected={selected} onOpen={openFriend} />
              ) : null}
              {account.room && chatOn ? (
                <button
                  type="button"
                  onClick={() => setSelected('room')}
                  aria-current={selected === 'room' ? 'true' : undefined}
                  className={`kryo-radius flex items-center gap-2.5 px-2 py-1.5 text-left text-xs text-foreground transition-colors hover:bg-secondary${selected === 'room' ? ' bg-secondary' : ''}`}
                >
                  <span className="kryo-pill grid size-7 shrink-0 place-items-center bg-secondary">
                    <Globe className="size-3.5 text-muted-foreground" aria-hidden />
                  </span>
                  <span className="grid min-w-0">
                    <span>Public room</span>
                    <span className="text-[10px] text-muted-foreground">Everyone, not encrypted</span>
                  </span>
                </button>
              ) : null}
              {groupsOn && chatOn ? (
                <div className="grid gap-1">
                  <Caption className="mb-1 flex items-center justify-between">
                    <span>Group chats</span>
                    <button type="button" className="flex items-center gap-1 normal-case tracking-normal hover:text-foreground" onClick={() => setCreatingGroup(true)}>
                      <Plus className="size-3" aria-hidden /> New
                    </button>
                  </Caption>
                  {groups.length === 0 ? (
                    <p className="kryo-radius border border-dashed border-border px-3 py-2.5 text-[11px] text-muted-foreground">Start a group with your friends.</p>
                  ) : null}
                  {groups.map((g) => {
                    const key = groupTarget(g.id)
                    const n = unread[groupConversationId(g.id)] ?? 0
                    return (
                      <button
                        key={g.id}
                        type="button"
                        onClick={() => setSelected(key)}
                        aria-current={selected === key ? 'true' : undefined}
                        className={`kryo-radius flex items-center justify-between gap-2 px-2 py-1.5 text-left text-xs text-foreground transition-colors hover:bg-secondary${selected === key ? ' bg-secondary' : ''}`}
                      >
                        <span className="flex min-w-0 items-center gap-2.5">
                          <span className="kryo-pill grid size-7 shrink-0 place-items-center bg-secondary">
                            <Users className="size-3.5 text-muted-foreground" aria-hidden />
                          </span>
                          <span className="grid min-w-0">
                            <span className="truncate">{g.name}</span>
                            <span className="text-[10px] text-muted-foreground">{g.members.length} people</span>
                          </span>
                        </span>
                        {n ? <Unread n={n} /> : null}
                      </button>
                    )
                  })}
                </div>
              ) : null}
              <button type="button" className="justify-self-start text-[11px] text-muted-foreground underline-offset-2 hover:text-foreground hover:underline" onClick={() => onWeb('/friends')}>
                Privacy and blocked people
              </button>
            </div>
          ) : (
            <ListSkeleton missing={!account.friends} />
          )}
        </aside>
      ) : null}

      {creatingGroup && live ? (
        <GroupCreateDialog
          friends={state.friends}
          onClose={() => setCreatingGroup(false)}
          onCreated={(g) => {
            setCreatingGroup(false)
            setGroups((list) => [g, ...list.filter((x) => x.id !== g.id)])
            setSelected(groupTarget(g.id))
          }}
        />
      ) : null}
      {membersOf && myIdRef.current && live
        ? (() => {
            const g = groups.find((x) => x.id === membersOf)
            return g ? (
              <GroupMembersDialog
                group={g}
                myId={myIdRef.current}
                friends={state.friends}
                nameOf={nameOf}
                usernameOf={usernameOf}
                onClose={() => setMembersOf(null)}
                onLeft={() => {
                  setMembersOf(null)
                  setSelected(null)
                  setGroups((list) => list.filter((x) => x.id !== g.id))
                }}
              />
            ) : null
          })()
        : null}

      {!showPane ? null : selected === 'room' && chatOn && account.room ? (
        <RoomPane myUsername={account.username} onProfile={(u) => onWeb(`/user/${encodeURIComponent(u)}`)} onBack={back} />
      ) : openGroup && chatOn && myIdRef.current ? (
        <ChatPane
          key={selected ?? ''}
          peer={{ id: groupTarget(openGroup.id), name: openGroup.name, supporter: false }}
          myId={myIdRef.current}
          meSupporter={!!account.supporter}
          onProfile={() => {}}
          games={games}
          onJoinInvite={onJoinInvite}
          group={openGroup}
          nameOf={nameOf}
          usernameOf={usernameOf}
          onMembers={() => setMembersOf(openGroup.id)}
          onBack={back}
        />
      ) : peer && chatOn && myIdRef.current ? (
        <ChatPane
          key={peer.id}
          peer={{ id: peer.id, name: friendName(peer), supporter: peer.supporter }}
          face={peer}
          myId={myIdRef.current}
          meSupporter={!!account.supporter}
          onProfile={() => profileOf(peer)}
          request={requestOf(peer.id)}
          onRespond={(accept) => respond(peer.id, accept)}
          canCall={!!account.voice}
          games={games}
          onJoinInvite={onJoinInvite}
          onBack={back}
        />
      ) : narrow ? null : (
        <section className="grid min-h-0 place-content-center justify-items-center gap-6 overflow-auto p-10 text-center">
          <AsciiArt lines={CHAT} mode="reveal" revealMs={800} className="h-24 text-muted-foreground" />
          {!account.friends ? (
            <div className="grid max-w-sm gap-2">
              <Label>Friends &amp; chat</Label>
              <p className="text-sm text-muted-foreground">Friends and chat are not on for your account yet.</p>
            </div>
          ) : noFriendsYet ? (
            <div className="grid w-full max-w-md gap-5 text-left">
              <div className="grid gap-2 text-center">
                <Label>Friends &amp; chat</Label>
                <p className="text-2xl font-bold text-foreground">Add your first friend</p>
                <p className="text-sm text-muted-foreground">See what they are playing, chat end-to-end encrypted, call, and invite them to your games.</p>
              </div>
              <AddFriendForm />
              <Button className="justify-self-center" onClick={() => setAdding(true)}>
                Or share an invite link
              </Button>
            </div>
          ) : (
            <div className="grid gap-2">
              <Label>Friends &amp; chat</Label>
              <p className="text-2xl font-bold text-foreground">{chatOn ? 'Pick someone to talk to' : 'Your friends, in one place'}</p>
            </div>
          )}
          {account.friends ? <ChatSetup available={!!account.chat} /> : null}
        </section>
      )}

      {flash ? (
        <p
          role="status"
          className={cn(
            'kryo-pill kryo-toast pointer-events-none fixed bottom-14 left-1/2 z-[650] max-w-[min(90vw,420px)] -translate-x-1/2 truncate px-4 py-2 text-xs shadow-2xl',
            flash.ok ? 'bg-primary text-primary-foreground' : 'bg-destructive text-destructive-foreground',
          )}
        >
          {flash.text}
        </p>
      ) : null}
      {adding ? <AddFriendDialog onClose={() => setAdding(false)} /> : null}
      {settingsOpen ? <ChatSettingsDialog available={!!account.chat} calls={!!account.voice} onClose={() => setSettingsOpen(false)} /> : null}
      {menu ? <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} /> : null}
      {asking ? (
        <Modal
          title={asking.title}
          onClose={() => setAsking(null)}
          footer={
            <>
              <Button onClick={() => setAsking(null)}>Cancel</Button>
              <Button
                variant="danger"
                onClick={() => {
                  const a = asking
                  setAsking(null)
                  void act('confirm', a.run, a.confirm === 'Block' ? 'Blocked.' : 'Removed.')
                }}
              >
                {asking.confirm}
              </Button>
            </>
          }
        >
          <p className="text-sm text-foreground">{asking.body}</p>
        </Modal>
      ) : null}
    </div>
  )
}

/** You: picture, name, and whether chat is connected on this device. */
function SelfCard({ account, chat, onProfile }: { account: Account; chat: ChatStatus | null; onProfile: () => void }) {
  const name = account.displayName || account.username
  const line: { text: string; tone: 'on' | 'wait' | 'off' } =
    chat?.state === 'online'
      ? { text: 'Online', tone: 'on' }
      : chat?.state === 'connecting'
        ? { text: 'Connecting', tone: 'wait' }
        : chat?.state === 'offline'
          ? { text: 'Reconnecting', tone: 'wait' }
          : chat?.state === 'locked'
            ? { text: 'Chat is locked', tone: 'off' }
            : { text: 'Chat is off', tone: 'off' }
  return (
    <ProfileLink username={account.guest ? null : account.username} onProfile={onProfile} className="kryo-square flex items-center gap-3 border-b border-border p-4 text-left transition-colors hover:bg-secondary focus-visible:outline-2 focus-visible:outline-primary">
      <Face url={account.avatarUrl} name={name} className="size-10 text-sm" />
      <span className="grid min-w-0">
        <b className="truncate text-sm text-foreground"><TeamName username={account.guest ? null : account.username}>{name}</TeamName></b>
        <span
          className={cn(
            'flex items-center gap-1.5 text-[10px] uppercase tracking-wider',
            line.tone === 'on' ? 'text-success' : line.tone === 'wait' ? 'text-warning' : 'text-muted-foreground',
          )}
        >
          <span className={cn('size-1.5 rounded-full', line.tone === 'on' ? 'bg-success' : line.tone === 'wait' ? 'bg-warning' : 'bg-muted-foreground')} />
          {line.text}
        </span>
      </span>
    </ProfileLink>
  )
}

function Face({ url, name, className }: { url: string | null; name: string; className?: string }) {
  return url ? (
    <img decoding="async" src={artSrc(url) ?? undefined} alt="" className={cn('kryo-pill shrink-0 object-cover', className)} />
  ) : (
    <span className={cn('kryo-pill grid shrink-0 place-items-center bg-secondary font-bold text-foreground', className)}>{name.slice(0, 1).toUpperCase()}</span>
  )
}

function Unread({ n }: { n: number }) {
  return (
    <span className="kryo-pill bg-primary px-1.5 text-[10px] font-bold tabular-nums text-primary-foreground" aria-label={`${n} unread`}>
      {n > 99 ? '99+' : n}
    </span>
  )
}

function GroupTitle({ title, count }: { title: string; count?: number }) {
  return (
    <Caption className="mb-1 flex items-center justify-between px-1">
      <span>{title}</span>
      {count !== undefined ? <span className="tabular-nums">{count}</span> : null}
    </Caption>
  )
}

function FriendRow({ f, label, sub }: { f: FriendFace & Partial<Pick<Friend, 'online' | 'playing'>>; label: string; sub?: string }) {
  const dot = f.playing || f.online
  const line = f.playing ? `Playing ${f.playing.title}` : sub ?? (f.online ? 'Online' : null)
  return (
    <span className="flex min-w-0 items-center gap-2.5">
      <span className="relative shrink-0">
        <Face url={f.avatarUrl} name={label} className="size-7 text-[10px]" />
        {dot ? (
          <span
            className={`absolute -bottom-0.5 -right-0.5 size-2.5 rounded-full border-2 border-card ${f.playing ? 'bg-primary' : 'bg-success'}`}
            aria-hidden
          />
        ) : null}
      </span>
      <span className="grid min-w-0">
        <span className="truncate"><TeamName username={f.username}>{label}</TeamName></span>
        {line ? <span className={cn('truncate text-[10px]', f.playing ? 'text-foreground/80' : 'text-muted-foreground')}>{line}</span> : null}
      </span>
    </span>
  )
}

function FriendGroup({
  title,
  list,
  empty,
  unread,
  selected,
  onOpen,
  onMenu,
  action,
}: {
  title: string
  list: Friend[]
  empty?: string
  unread: Record<string, number>
  selected: string | null
  onOpen: (f: FriendFace) => void
  onMenu: (f: Friend, x: number, y: number) => void
  action?: { label: string; onClick: () => void }
}) {
  return (
    <div className="grid gap-0.5">
      <GroupTitle title={title} count={list.length} />
      {list.length === 0 && empty ? (
        <div className="kryo-radius grid justify-items-start gap-2 border border-dashed border-border px-3 py-2.5 text-[11px] text-muted-foreground">
          {empty}
          {action ? (
            <Button size="sm" onClick={action.onClick}>
              <UserPlus className="size-3" aria-hidden />
              {action.label}
            </Button>
          ) : null}
        </div>
      ) : null}
      {list.map((f) => (
        <div
          key={f.id}
          className={cn('kryo-radius group relative flex items-center transition-colors hover:bg-secondary', selected === f.id && 'bg-secondary')}
          onContextMenu={(e) => {
            e.preventDefault()
            onMenu(f, e.clientX, e.clientY)
          }}
        >
          <button
            type="button"
            onClick={() => onOpen(f)}
            aria-current={selected === f.id ? 'true' : undefined}
            title={f.nickname ? `${f.displayName || f.username} (your nickname: ${f.nickname})` : `@${f.username}`}
            className="flex min-w-0 grow items-center justify-between gap-2 px-2 py-1.5 text-left text-xs text-foreground"
          >
            <FriendRow f={f} label={friendName(f)} />
            <span className="flex shrink-0 items-center gap-1 pr-7 text-muted-foreground">
              {unread[f.id] ? <Unread n={unread[f.id]!} /> : null}
              {f.muted ? <BellOff className="size-3" aria-label="Muted" /> : null}
              {f.favourite ? <Star className="size-3 fill-current" aria-label="Pinned" /> : null}
            </span>
          </button>
          <button
            type="button"
            aria-label={`More for ${friendName(f)}`}
            title="More"
            aria-haspopup="menu"
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect()
              onMenu(f, r.left, r.bottom + 4)
            }}
            className="kryo-pill absolute right-1.5 grid size-6 place-items-center text-muted-foreground opacity-0 transition-opacity hover:bg-background hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100"
          >
            <Ellipsis className="size-3.5" />
          </button>
        </div>
      ))}
    </div>
  )
}

/** Non-friends you can chat with: rows like friends, without pins or mutes. */
function PeopleGroup({
  title,
  list,
  unread,
  selected,
  onOpen,
}: {
  title: string
  list: FriendFace[]
  unread: Record<string, number>
  selected: string | null
  onOpen: (f: FriendFace) => void
}) {
  return (
    <div className="grid gap-0.5">
      <GroupTitle title={title} count={list.length} />
      {list.map((p) => (
        <button
          key={p.id}
          type="button"
          onClick={() => onOpen(p)}
          aria-current={selected === p.id ? 'true' : undefined}
          className={`kryo-radius flex items-center justify-between gap-2 px-2 py-1.5 text-left text-xs text-foreground transition-colors hover:bg-secondary${selected === p.id ? ' bg-secondary' : ''}`}
        >
          <FriendRow f={p} label={friendName(p)} />
          {unread[p.id] ? <Unread n={unread[p.id]!} /> : null}
        </button>
      ))}
    </div>
  )
}

/** While the list loads: rows the shape of the real ones. */
function ListSkeleton({ missing }: { missing: boolean }) {
  if (missing) return <p className="p-4 text-[11px] text-muted-foreground">Friends and chat are not on for your account yet.</p>
  return (
    <div className="grid content-start gap-2 p-3" aria-busy="true" aria-label="Loading friends">
      {[0, 1, 2, 3].map((i) => (
        <span key={i} className="flex items-center gap-2.5 px-2 py-1.5">
          <span className="kryo-pill kryo-shimmer size-7 bg-secondary" />
          <span className="kryo-radius kryo-shimmer h-3 w-28 bg-secondary" />
        </span>
      ))}
    </div>
  )
}

/** One line of what was said last, as a list shows it. */
function preview(m: ChatMessage | null, myId: string | null, nameOf: (id: string) => string, group: boolean): string {
  if (!m) return ''
  const who = m.outgoing || m.senderUser === myId ? 'You: ' : group ? `${nameOf(m.senderUser)}: ` : ''
  if (m.deleted) return `${who}Message deleted`
  if (m.kind === 'gif') return `${who}GIF`
  if (m.kind === 'file') return `${who}File: ${parseFile(m.body)?.name ?? ''}`
  if (m.kind === 'invite') return `${who}Game invite: ${parseInvite(m.body)?.title ?? ''}`
  return who + m.body.replace(/\s+/g, ' ')
}

/** When, as short as a list row allows: a time today, else a date. */
function when(ms: number): string {
  const d = new Date(ms)
  return d.toDateString() === new Date().toDateString()
    ? d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString(undefined, { day: 'numeric', month: 'short' })
}

/**
 * Conversations, newest first: people and groups with what was said last,
 * message requests on top, the public room at the end.
 */
function ChatsList({
  conversations,
  needle,
  faceOf,
  requests,
  groups,
  room,
  chatOn,
  unread,
  selected,
  myId,
  nameOf,
  onOpen,
  onNewGroup,
  onCompose,
}: {
  conversations: Conversation[]
  needle: string
  faceOf: (id: string) => (FriendFace & Partial<Pick<Friend, 'online' | 'playing' | 'nickname'>>) | undefined
  requests: FriendFace[]
  /** Group chats, when they are on. */
  groups: GroupView[] | null
  room: boolean
  chatOn: boolean
  unread: Record<string, number>
  selected: string | null
  myId: string | null
  nameOf: (id: string) => string
  onOpen: (key: string) => void
  onNewGroup: () => void
  onCompose: () => void
}) {
  const requestIds = new Set(requests.map((p) => p.id))
  const groupOf = new Map((groups ?? []).map((g) => [groupConversationId(g.id), g]))
  const rows = conversations
    .map((c) => {
      const g = c.peerUserId ? undefined : groupOf.get(c.id)
      const face = c.peerUserId ? faceOf(c.peerUserId) : undefined
      if (c.peerUserId ? !face || requestIds.has(c.peerUserId) : !g) return null
      const name = g ? g.name : friendName(face!)
      return { key: g ? groupTarget(g.id) : c.peerUserId!, name, face, group: g, c, n: unread[g ? c.id : c.peerUserId!] ?? 0 }
    })
    .filter((r): r is NonNullable<typeof r> => r !== null && (!needle || r.name.toLowerCase().includes(needle)))
  // Groups nobody has written in yet still belong in the list.
  const quiet = (groups ?? []).filter((g) => !conversations.some((c) => c.id === groupConversationId(g.id)) && (!needle || g.name.toLowerCase().includes(needle)))
  const row = 'kryo-radius flex w-full items-center gap-2.5 px-2 py-2 text-left text-xs text-foreground transition-colors hover:bg-secondary'
  if (!chatOn) return <div className="p-3" />
  return (
    <div className="grid content-start gap-5 overflow-auto p-3">
      {requests.length > 0 && !needle ? (
        <div className="grid gap-0.5">
          <GroupTitle title="Message requests" count={requests.length} />
          {requests.map((p) => (
            <button key={p.id} type="button" onClick={() => onOpen(p.id)} aria-current={selected === p.id ? 'true' : undefined} className={cn(row, selected === p.id && 'bg-secondary')}>
              <FriendRow f={p} label={friendName(p)} sub="Wants to message you" />
            </button>
          ))}
        </div>
      ) : null}
      <div className="grid gap-0.5">
        <GroupTitle title="Messages" />
        {rows.length === 0 && quiet.length === 0 ? (
          <div className="kryo-radius grid justify-items-start gap-2 border border-dashed border-border px-3 py-2.5 text-[11px] text-muted-foreground">
            {needle ? 'Nothing matches.' : 'No conversations yet. Message a friend to start one.'}
            {needle ? null : (
              <Button size="sm" onClick={onCompose}>
                <MessageSquarePlus className="size-3" aria-hidden />
                New message
              </Button>
            )}
          </div>
        ) : null}
        {rows.map((r) => (
          <button key={r.key} type="button" onClick={() => onOpen(r.key)} aria-current={selected === r.key ? 'true' : undefined} className={cn(row, selected === r.key && 'bg-secondary')}>
            {r.group ? (
              <span className="kryo-pill grid size-7 shrink-0 place-items-center bg-secondary">
                <Users className="size-3.5 text-muted-foreground" aria-hidden />
              </span>
            ) : (
              <span className="relative shrink-0">
                <Face url={r.face!.avatarUrl} name={r.name} className="size-7 text-[10px]" />
                {r.face!.playing || r.face!.online ? (
                  <span className={`absolute -bottom-0.5 -right-0.5 size-2.5 rounded-full border-2 border-card ${r.face!.playing ? 'bg-primary' : 'bg-success'}`} aria-hidden />
                ) : null}
              </span>
            )}
            <span className="grid min-w-0 grow">
              <span className="flex items-baseline justify-between gap-2">
                <span className={cn('truncate', r.n > 0 && 'font-bold')}><TeamName username={r.group ? null : r.face?.username}>{r.name}</TeamName></span>
                <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">{when(r.c.lastAt)}</span>
              </span>
              <span className="flex items-center justify-between gap-2">
                <span className={cn('truncate text-[11px]', r.n > 0 ? 'text-foreground' : 'text-muted-foreground')}>{preview(r.c.last, myId, nameOf, !!r.group)}</span>
                {r.n ? <Unread n={r.n} /> : null}
              </span>
            </span>
          </button>
        ))}
        {quiet.map((g) => (
          <button key={g.id} type="button" onClick={() => onOpen(groupTarget(g.id))} aria-current={selected === groupTarget(g.id) ? 'true' : undefined} className={cn(row, selected === groupTarget(g.id) && 'bg-secondary')}>
            <span className="kryo-pill grid size-7 shrink-0 place-items-center bg-secondary">
              <Users className="size-3.5 text-muted-foreground" aria-hidden />
            </span>
            <span className="grid min-w-0">
              <span className="truncate">{g.name}</span>
              <span className="text-[11px] text-muted-foreground">{g.members.length} people · say hi</span>
            </span>
          </button>
        ))}
      </div>
      {groups ? (
        <button type="button" className="flex items-center gap-1.5 justify-self-start text-[11px] text-muted-foreground hover:text-foreground" onClick={onNewGroup}>
          <Plus className="size-3" aria-hidden /> New group chat
        </button>
      ) : null}
      {room ? (
        <button type="button" onClick={() => onOpen('room')} aria-current={selected === 'room' ? 'true' : undefined} className={cn(row, selected === 'room' && 'bg-secondary')}>
          <span className="kryo-pill grid size-7 shrink-0 place-items-center bg-secondary">
            <Globe className="size-3.5 text-muted-foreground" aria-hidden />
          </span>
          <span className="grid min-w-0">
            <span>Public room</span>
            <span className="text-[10px] text-muted-foreground">Everyone, not encrypted</span>
          </span>
        </button>
      ) : null}
    </div>
  )
}
