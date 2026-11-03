import { StrictMode, useEffect, useMemo, useRef, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { ArrowUpRight, Lock, LogOut, MonitorSmartphone } from 'lucide-react'
import '../styles.css'
import { on, useWebBackend } from '@/lib/bridge'
import type { Account } from '@/hooks/useAccount'
import { FriendsPage } from '@/friends/FriendsPage'
import { CallOverlay } from '@/friends/CallOverlay'
import { friendName, useFriends } from '@/hooks/useFriends'
import { useSocialBadge } from '@/hooks/useSocialBadge'
import { accept, decline, hangUp, subscribe as onCall, toggleDeafen, toggleMute } from '@/lib/calls'
import { startActiveBeacon, type ChatMessage } from '@/lib/chat'
import { applyLook } from '@/lib/settings'
import { Button, Label } from '@/ui'
import { API_BASE, api, startSignIn, waitForToken, type DeviceStart } from './api'
import { createBackend } from './backend'
import { callElsewhere, EMBEDDED, EXPECTED_USER, fromSite, holdEngineLock, setEngineInCall, toSite } from './embed'
import { Store } from './store'

/**
 * chat.kryo.to: Kryoto's end-to-end encrypted chat in a browser. The same
 * chat screens as Kryoto Desktop, with the engine in WebAssembly. It has its
 * own origin (no ads, no analytics, nothing third-party) and its own sign-in:
 * a code approved on kryo.to, never the kryo.to cookie.
 *
 * kryo.to also shows it in its Friends & chat panel, framed (`embed.ts`):
 * there the site approves the code when the reader presses Turn on chat, and
 * the frame shows only the conversations, beside the site's own friends list.
 */

type Me = Account & { id: string; anonymous: boolean; web: boolean }

async function loadMe(token: string): Promise<Me | null> {
  try {
    const r = await api<{ user: Record<string, any> | null }>(token, 'GET', '/api/auth/me')
    const u = r.user
    if (!u) return null
    const f = (u.features ?? {}) as Record<string, boolean>
    return {
      id: String(u.id),
      anonymous: !!u.isAnonymous,
      web: !!f.chat_web,
      username: u.username,
      displayName: u.displayName ?? null,
      avatarUrl: u.avatarUrl ?? null,
      supporter: !!(u.isSupporter || (Array.isArray(u.perks) && u.perks.includes('ad_free'))),
      chat: !!f.chat,
      friends: !!f.friends,
      groups: !!f.chat_groups,
      room: !!f.chat_room,
      voice: !!f.voice,
    }
  } catch (e) {
    if (e && typeof e === 'object' && 'status' in e && (e as { status: number }).status === 401) return null
    throw e
  }
}

function Frame({ children }: { children: React.ReactNode }) {
  return <div className="grid min-h-svh place-items-center bg-background p-6 text-center text-foreground">{children}</div>
}

function SignIn({ onToken }: { onToken: (t: string) => void }) {
  const [started, setStarted] = useState<DeviceStart | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!started) return
    let cancelled = false
    void waitForToken(started, () => cancelled)
      .then(onToken)
      .catch((e: unknown) => !cancelled && (setError(e instanceof Error ? e.message : String(e)), setStarted(null)))
    return () => {
      cancelled = true
    }
  }, [started, onToken])
  return (
    <Frame>
      <div className="kryo-radius grid w-full max-w-md gap-4 border border-border bg-card p-6 text-left">
        <div className="grid gap-1">
          <Label>Kryoto chat</Label>
          <p className="text-xl font-bold">Chat in your browser</p>
        </div>
        <p className="text-sm text-muted-foreground">
          End-to-end encrypted, like Kryoto Desktop: only the people in a conversation can read it, not Kryoto. A browser is
          a little less safe than the app - a compromised website could target it - so use Kryoto Desktop for anything
          sensitive.
        </p>
        {started ? (
          <div className="grid gap-3">
            <p className="text-sm">Approve this sign-in on kryo.to (you need to be signed in there):</p>
            <code className="kryo-radius border border-border bg-background p-3 text-center font-mono text-2xl tracking-[0.3em]">
              {started.user_code}
            </code>
            <a href={started.verification_uri_complete} target="_blank" rel="noreferrer" className="inline-flex">
              <Button variant="primary">
                Open kryo.to to approve <ArrowUpRight className="size-3" aria-hidden />
              </Button>
            </a>
            <p className="text-xs text-muted-foreground">Waiting for you to approve...</p>
          </div>
        ) : (
          <Button
            variant="primary"
            onClick={() => {
              setError(null)
              void startSignIn()
                .then(setStarted)
                .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
            }}
          >
            <Lock className="size-3" aria-hidden /> Sign in with kryo.to
          </Button>
        )}
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
      </div>
    </Frame>
  )
}

/**
 * Signing in inside kryo.to's panel: the site shows its own Turn on chat
 * button and approves the code itself, so here there is only the wait.
 */
function EmbeddedSignIn({ onToken }: { onToken: (t: string) => void }) {
  const [error, setError] = useState<string | null>(null)
  const busy = useRef(false)
  useEffect(() => {
    toSite({ type: 'state', state: 'signin' })
    let cancelled = false
    const stop = fromSite((m) => {
      if (m.type !== 'signin-start' || busy.current) return
      busy.current = true
      setError(null)
      toSite({ type: 'state', state: 'waiting' })
      void startSignIn()
        .then((s) => {
          toSite({ type: 'signin-code', code: s.user_code })
          return waitForToken(s, () => cancelled)
        })
        .then(onToken)
        .catch((e: unknown) => {
          const text = e instanceof Error ? e.message : String(e)
          setError(text)
          toSite({ type: 'state', state: 'signin', detail: text })
        })
        .finally(() => (busy.current = false))
    })
    return () => {
      cancelled = true
      stop()
    }
  }, [onToken])
  return (
    <Frame>
      <p className="text-xs text-muted-foreground">{error ?? 'Waiting for kryo.to...'}</p>
    </Frame>
  )
}

/** Another tab already runs chat in this browser. */
function Elsewhere({ onTake, inCall }: { onTake: () => void; inCall: boolean }) {
  const moved = (() => {
    try {
      return sessionStorage.getItem('kryo.chat.moved') === '1'
    } catch {
      return false
    }
  })()
  return (
    <Frame>
      <div className="grid max-w-sm justify-items-center gap-3">
        <MonitorSmartphone className="size-6 text-muted-foreground" aria-hidden />
        <p className="text-sm font-bold">{inCall ? 'You are in a call in another tab' : moved ? 'Chat moved to another tab' : 'Chat is open in another tab'}</p>
        <p className="text-xs text-muted-foreground">
          {inCall ? 'Using chat here ends that call.' : 'One tab at a time keeps your messages in order. Use it here, and the other tab lets go.'}
        </p>
        <Button variant="primary" onClick={onTake}>
          {inCall ? 'Use chat here anyway' : 'Use chat here'}
        </Button>
      </div>
    </Frame>
  )
}

/** One backend (one gateway connection) per sign-in, StrictMode or not. */
let installed: string | null = null
function install(token: string, userId: string, onSignedOut: () => void) {
  if (installed === `${userId}:${token}`) return
  installed = `${userId}:${token}`
  useWebBackend(createBackend(token, userId, onSignedOut))
}

function Chat({ me, token, onSignedOut }: { me: Me; token: string; onSignedOut: () => void }) {
  install(token, me.id, onSignedOut)
  const friends = useFriends()
  const person = (id: string) => friends?.friends.find((x) => x.id === id) ?? friends?.messageRequests.accepted.find((x) => x.id === id) ?? friends?.messageRequests.incoming.find((x) => x.id === id)
  const nameOf = (id: string) => {
    const f = person(id)
    return f ? friendName(f) : 'Someone'
  }
  const avatarOf = (id: string) => person(id)?.avatarUrl ?? null
  const [chatWith, setChatWith] = useState<{ username: string; at: number } | null>(null)
  const open = (path: string) => (EMBEDDED ? toSite({ type: 'navigate', path }) : window.open(`${API_BASE}${path}`, '_blank', 'noopener'))

  useEffect(() => (EMBEDDED ? fromSite((m) => m.type === 'open' && setChatWith({ username: m.username, at: Date.now() })) : undefined), [])

  const page = (
    <FriendsPage
      account={me}
      onProfile={() => open(`/user/${encodeURIComponent(me.username)}`)}
      onWeb={open}
      chatWith={chatWith}
      onJoinInvite={(inv) => open(`/game/${encodeURIComponent(inv.slug)}`)}
      list={EMBEDDED ? 'chats' : 'friends'}
    />
  )
  if (EMBEDDED) {
    return (
      <div className="flex h-svh flex-col bg-background text-foreground">
        <SiteBridge nameOf={nameOf} />
        {page}
        {me.voice ? <CallOverlay nameOf={nameOf} avatarOf={avatarOf} /> : null}
      </div>
    )
  }
  return (
    <div className="flex h-svh flex-col bg-background text-foreground">
      <header className="flex items-center justify-between gap-3 border-b border-border px-4 py-2">
        <span className="flex items-center gap-2 text-xs uppercase tracking-[0.2em] text-muted-foreground">
          <Lock className="size-3" aria-hidden /> Kryoto chat
        </span>
        <div className="flex items-center gap-2">
          <a href={API_BASE} className="inline-flex">
            <Button size="sm" variant="ghost">
              kryo.to <ArrowUpRight className="size-3" aria-hidden />
            </Button>
          </a>
          <Button
            size="sm"
            onClick={async () => {
              const store = await Store.open()
              await store.del('token')
              onSignedOut()
            }}
            title="Sign out of chat in this browser (your keys stay here until you remove chat)"
          >
            <LogOut className="size-3" aria-hidden /> Sign out
          </Button>
        </div>
      </header>
      {page}
      {me.voice ? <CallOverlay nameOf={nameOf} avatarOf={avatarOf} /> : null}
    </div>
  )
}

/**
 * What kryo.to's panel needs from the chat: the counts for its badges, who
 * just wrote (for its notice, never what), the call for its call bar, and the
 * call buttons pressed there.
 */
function SiteBridge({ nameOf }: { nameOf: (id: string) => string }) {
  const badge = useSocialBadge()
  useEffect(() => toSite({ type: 'badge', unread: badge.unread, messageRequests: badge.messageRequests }), [badge.unread, badge.messageRequests])
  const names = useRef(nameOf)
  names.current = nameOf
  useEffect(() => {
    toSite({ type: 'state', state: 'ready' })
    const stops = [
      on<ChatMessage>('chat-message', (m) => {
        if (!m.outgoing && m.senderUser) toSite({ type: 'message', from: names.current(m.senderUser), peer: m.senderUser })
      }),
    ]
    const offCall = onCall((c) =>
      toSite({
        type: 'call',
        phase: c.phase,
        peer: 'peer' in c ? c.peer : '',
        name: 'peer' in c ? names.current(c.peer) : '',
        since: c.phase === 'active' ? c.since : null,
        muted: c.muted,
        deafened: c.deafened,
        reason: c.phase === 'ended' ? c.reason : null,
        reconnecting: c.phase === 'active' && c.reconnecting,
      }),
    )
    const offSite = fromSite((m) => {
      if (m.type !== 'call') return
      if (m.action === 'accept') void accept()
      else if (m.action === 'decline') decline()
      else if (m.action === 'hangup') hangUp()
      else if (m.action === 'mute') toggleMute()
      else if (m.action === 'deafen') toggleDeafen()
    })
    return () => {
      stops.forEach((p) => void p.then((f) => f()))
      offCall()
      offSite()
    }
  }, [])
  return null
}

function App() {
  const [token, setToken] = useState<string | null | undefined>(undefined)
  const [me, setMe] = useState<Me | null | undefined>(undefined)
  const [error, setError] = useState<string | null>(null)
  const [lock, setLock] = useState<'pending' | 'held' | 'elsewhere'>('pending')
  const [busyElsewhere, setBusyElsewhere] = useState(false)
  // Other tabs ask before taking chat over; they must not end a call here.
  useEffect(() => onCall((c) => setEngineInCall(c.phase !== 'idle' && c.phase !== 'ended')), [])
  // In use: the tab is in front and someone touched chat in the last two
  // minutes (inside kryo.to the frame only sees its own clicks and keys).
  useEffect(() => {
    let touched = 0
    const touch = () => (touched = Date.now())
    const events = ['pointerdown', 'keydown', 'wheel'] as const
    events.forEach((e) => window.addEventListener(e, touch, { passive: true }))
    const stop = startActiveBeacon(() => document.visibilityState === 'visible' && (EMBEDDED || document.hasFocus()) && Date.now() - touched < 120_000)
    return () => {
      stop()
      events.forEach((e) => window.removeEventListener(e, touch))
    }
  }, [])
  useEffect(() => {
    void Store.open()
      .then((s) => s.get<string>('token'))
      .then((t) => setToken(t ?? null))
      .catch(() => setToken(null))
  }, [])
  useEffect(() => {
    if (!token) return
    void loadMe(token)
      .then((m) => {
        if (!m) setToken(null)
        else setMe(m)
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
  }, [token])
  const ready = !!(token && me && !me.anonymous && me.web && me.chat && me.friends && !(EXPECTED_USER && me.id !== EXPECTED_USER))
  useEffect(() => {
    if (!ready) return
    let alive = true
    const got = () => {
      if (!alive) return
      setLock('held')
      try {
        sessionStorage.removeItem('kryo.chat.moved')
      } catch {
        /* only changes the wording */
      }
    }
    void holdEngineLock('try').then((held) => {
      if (held) return got()
      if (alive) setLock('elsewhere')
      // In kryo.to's panel: take over by itself when the tab with chat closes.
      if (EMBEDDED) void holdEngineLock('wait').then((h) => h && got())
    })
    return () => {
      alive = false
    }
  }, [ready])
  const take = () => void holdEngineLock('steal').then((held) => held && setLock('held'))
  // In kryo.to's panel: opening it here means using chat here.
  const takeRef = useRef(take)
  takeRef.current = take
  const lockRef = useRef(lock)
  lockRef.current = lock
  useEffect(
    () =>
      fromSite((m) => {
        if (m.type === 'look') applyLook({ theme: m.look.theme, palette: m.look.palette as never, radius: m.look.radius as never, font: m.look.font === 'mono' ? 'mono' : 'teletext', season: m.look.season, reducedMotion: m.look.motion === 'reduced' })
        // Never by itself while the other tab is in a call: that would cut it off.
        if (m.type === 'focus' && lockRef.current === 'elsewhere') void callElsewhere().then((busy) => (busy ? setBusyElsewhere(true) : takeRef.current()))
      }),
    [],
  )
  const signedOut = useMemo(() => () => window.location.reload(), [])
  const save = useMemo(
    () => (t: string) => {
      void Store.open()
        .then((s) => s.set('token', t))
        .then(() => setToken(t))
    },
    [],
  )

  const state =
    error ? 'unavailable'
    : token === undefined || (token && me === undefined) ? 'loading'
    : !token || !me ? 'signin'
    : me.anonymous ? 'anonymous'
    : EXPECTED_USER && me.id !== EXPECTED_USER ? 'other-account'
    : !me.web || !me.chat || !me.friends ? 'unavailable'
    : lock === 'elsewhere' ? 'elsewhere'
    : lock === 'pending' ? 'loading'
    : 'ready'
  useEffect(() => {
    if (state !== 'signin' && state !== 'ready') toSite({ type: 'state', state, detail: state === 'other-account' ? me?.username : (error ?? undefined) })
  }, [state, me, error])

  if (state === 'unavailable') return <Frame><p className="max-w-sm text-sm text-muted-foreground">{error ?? 'Chat is not available for your account yet.'}</p></Frame>
  if (state === 'loading') return <Frame><p className="text-xs text-muted-foreground">Loading...</p></Frame>
  if (state === 'signin') return EMBEDDED ? <EmbeddedSignIn onToken={save} /> : <SignIn onToken={save} />
  if (state === 'anonymous') return <Frame><p className="max-w-sm text-sm">You need to turn off anonymous mode to use this feature.</p></Frame>
  if (state === 'other-account')
    return (
      <Frame>
        <div className="grid max-w-sm justify-items-center gap-3">
          <p className="text-sm">Chat in this browser is signed in as @{me!.username}.</p>
          <Button
            onClick={async () => {
              const store = await Store.open()
              await store.del('token')
              signedOut()
            }}
          >
            <LogOut className="size-3" aria-hidden /> Sign out of chat here
          </Button>
        </div>
      </Frame>
    )
  if (state === 'elsewhere') return <Elsewhere onTake={take} inCall={busyElsewhere} />
  return <Chat me={me!} token={token!} onSignedOut={signedOut} />
}

document.documentElement.dataset.window = EMBEDDED ? 'embed' : 'web'
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
