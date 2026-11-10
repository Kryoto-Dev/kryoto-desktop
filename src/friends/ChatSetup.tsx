import { useEffect, useState } from 'react'
import { Bell, BellOff, KeyRound, Lock, Monitor, Power, RotateCcw, ShieldCheck, Smartphone, Trash2 } from '@/ui/pixel-icons'
import { errorText, isWeb } from '@/lib/bridge'
import {
  chatAccountDeviceRemove,
  chatAccountDevices,
  chatEnable,
  chatPushDisable,
  chatPushEnable,
  chatPushState,
  chatRemoveDevice,
  chatResetIdentity,
  chatRestore,
  chatStatusText,
  chatUnlock,
  Here,
  here,
  useChatStatus,
  type AccountDevice,
  type AccountDevices,
  type PushState,
} from '@/lib/chat'
import { Button, Caption, inputCls, Modal } from '@/ui'
import { ChatSecurity } from './ChatSecurity'

type Asking = 'enable' | 'remove' | 'reset' | 'restore' | null

/**
 * Turning chat on for this PC, and taking it off again; unlocking it where
 * a passphrase protects it; and, when the account already has chat on
 * another device, restoring its identity from the key backup or starting a
 * new one.
 *
 * Every step that signs in, deletes or replaces something asks first.
 */
/** "Today", "Yesterday", "12 days ago": when a device last connected. */
function lastUsed(d: AccountDevice): string {
  if (d.online) return 'Connected now'
  if (!d.lastSeenMs) return 'Never connected'
  const days = Math.floor((Date.now() - d.lastSeenMs) / 86_400_000)
  return days <= 0 ? 'Used today' : days === 1 ? 'Used yesterday' : `Used ${days} days ago`
}

/**
 * The account's chat devices, listed and removed with no chat on this device:
 * how a device turned away at the limit makes room. Removing one is what
 * Remove in that device's own settings does.
 */
function YourChatDevices({ onRemoved }: { onRemoved: () => void }) {
  const [list, setList] = useState<AccountDevices | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [removing, setRemoving] = useState<AccountDevice | null>(null)
  const load = () => {
    setError(null)
    chatAccountDevices().then(setList).catch((e: unknown) => setError(errorText(e)))
  }
  useEffect(load, [])
  const name = (d: AccountDevice) => d.name || (d.kind === 'desktop' ? 'A PC' : 'A phone or browser')
  return (
    <div className="grid gap-2 border-t border-border pt-3">
      <Caption>Your chat devices</Caption>
      {error ? (
        <div className="flex flex-wrap items-center gap-2">
          <p className="text-xs text-destructive">{error}</p>
          <Button size="sm" onClick={load}>
            Retry
          </Button>
        </div>
      ) : !list ? (
        <p className="text-xs text-muted-foreground">Loading...</p>
      ) : (
        <ul className="grid gap-1.5">
          {list.devices.map((d) => (
              <li key={d.id} className="flex items-center gap-2.5">
                {d.kind === 'desktop' ? <Monitor className="size-3.5 shrink-0 text-muted-foreground" aria-hidden /> : <Smartphone className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />}
                <span className="grid min-w-0 grow">
                  <span className="truncate text-xs text-foreground">{name(d)}</span>
                  <span className="text-[11px] text-muted-foreground">{lastUsed(d)}</span>
                </span>
                <Button size="sm" variant="danger" onClick={() => setRemoving(d)}>
                  <Trash2 className="size-3" aria-hidden />
                  Remove
                </Button>
              </li>
            ))}
        </ul>
      )}
      {removing ? (
        <Modal
          title={`Remove ${name(removing)}?`}
          onClose={() => setRemoving(null)}
          footer={
            <>
              <Button onClick={() => setRemoving(null)}>Cancel</Button>
              <Button
                variant="danger"
                onClick={() =>
                  chatAccountDeviceRemove(removing.id).then(() => {
                    setRemoving(null)
                    load()
                    onRemoved()
                  })
                }
              >
                Remove
              </Button>
            </>
          }
        >
          <p className="text-sm text-foreground">
            It leaves chat: the messages kept on it can no longer be read there, and it has to be set up again to chat.
            Messages on your other devices stay.
          </p>
        </Modal>
      ) : null}
    </div>
  )
}

export function ChatSetup({ available }: { available: boolean }) {
  const status = useChatStatus()
  const [asking, setAsking] = useState<Asking>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [security, setSecurity] = useState(false)
  const [passphrase, setPassphrase] = useState('')
  const [repeat, setRepeat] = useState('')
  const [code, setCode] = useState('')

  if (!status) return null
  // Ships dark: shown only to accounts chat is rolled out to, or where chat
  // was already turned on (so it can still be removed).
  if (!available && status.state === 'off') return null
  const on = status.state !== 'off'
  const canEnable = ['off', 'signInNeeded', 'unavailable', 'disabled', 'anonymousMode'].includes(status.state)
  const online = status.state === 'online'

  const run = async (work: () => Promise<unknown>) => {
    setAsking(null)
    setBusy(true)
    setError(null)
    try {
      await work()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  const unlock = (creating: boolean) =>
    run(async () => {
      if (creating && passphrase !== repeat) throw new Error('The two passphrases are not the same.')
      const next = await chatUnlock(passphrase)
      setPassphrase('')
      setRepeat('')
      // A new passphrase is the first step of turning chat on.
      if (next.state === 'off') await chatEnable()
    })

  return (
    <div className="kryo-radius grid w-full max-w-md gap-3 border border-border p-4 text-left">
      <div className="flex items-center gap-2">
        <Lock className="size-3.5 text-muted-foreground" aria-hidden />
        <Caption>Chat on {here()}</Caption>
      </div>
      <p className="text-xs text-foreground" aria-live="polite">
        {busy ? 'Working...' : chatStatusText(status)}
      </p>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}

      {status.state === 'locked' ? (
        <form
          className="grid gap-2"
          onSubmit={(e) => {
            e.preventDefault()
            void unlock(status.creating)
          }}
        >
          <input
            type="password"
            autoFocus
            value={passphrase}
            onChange={(e) => setPassphrase(e.target.value)}
            placeholder={status.creating ? 'Choose a passphrase (8+ characters)' : 'Chat passphrase'}
            aria-label="Chat passphrase"
            className={inputCls}
          />
          {status.creating ? (
            <input
              type="password"
              value={repeat}
              onChange={(e) => setRepeat(e.target.value)}
              placeholder="Type it again"
              aria-label="Repeat the chat passphrase"
              className={inputCls}
            />
          ) : null}
          <div className="flex items-center gap-2">
            <Button type="submit" variant="primary" size="sm" disabled={busy || !passphrase}>
              {status.creating ? 'Set passphrase' : 'Unlock'}
            </Button>
            {status.creating ? (
              <span className="text-[11px] text-muted-foreground">Forget it and chat on {here()} has to start over.</span>
            ) : null}
          </div>
        </form>
      ) : null}

      {status.state === 'unavailable' ? <YourChatDevices onRemoved={() => void run(chatEnable)} /> : null}

      {status.state === 'needsLink' ? (
        <div className="flex flex-wrap gap-2">
          <Button variant="primary" size="sm" disabled={busy} onClick={() => setAsking('restore')}>
            <KeyRound className="size-3" aria-hidden />
            Use recovery code
          </Button>
          <Button size="sm" disabled={busy} onClick={() => setAsking('reset')}>
            <RotateCcw className="size-3" aria-hidden />
            Start a new identity
          </Button>
        </div>
      ) : null}

      <div className="flex flex-wrap gap-2">
        {canEnable ? (
          <Button variant="primary" size="sm" disabled={busy} onClick={() => setAsking('enable')}>
            <Power className="size-3" aria-hidden />
            Turn on chat
          </Button>
        ) : null}
        {online ? (
          <Button size="sm" disabled={busy} onClick={() => setSecurity(true)}>
            <ShieldCheck className="size-3" aria-hidden />
            Backup and devices
          </Button>
        ) : null}
        {on && status.state !== 'connecting' ? (
          <Button variant="danger" size="sm" disabled={busy} onClick={() => setAsking('remove')}>
            <Trash2 className="size-3" aria-hidden />
            Remove from {here()}
          </Button>
        ) : null}
      </div>

      {/* Browser pushes: a phone has its own, under Profile > Notifications. */}
      {online && isWeb() && here() !== 'this phone' ? <BrowserNotifications /> : null}

      {security ? <ChatSecurity onClose={() => setSecurity(false)} /> : null}

      {asking === 'enable' ? (
        <Modal
          title="Turn on chat on this PC?"
          onClose={() => setAsking(null)}
          footer={
            <>
              <Button onClick={() => setAsking(null)}>Cancel</Button>
              <Button variant="primary" onClick={() => run(chatEnable)}>
                Turn on
              </Button>
            </>
          }
        >
          <p className="text-sm text-foreground">
            This PC signs in to chat with the account you are signed in to in the Store, and makes its own
            encryption keys. Your messages are end-to-end encrypted: only the people in a conversation can read
            them, not Kryoto.
          </p>
          <p className="text-xs text-muted-foreground">
            The keys stay on this PC, protected by your Windows or Linux account (or a passphrase, where neither keeps
            secrets).
          </p>
        </Modal>
      ) : null}

      {asking === 'restore' ? (
        <Modal
          title="Use your recovery code"
          onClose={() => setAsking(null)}
          footer={
            <>
              <Button onClick={() => setAsking(null)}>Cancel</Button>
              <Button
                variant="primary"
                disabled={code.replace(/[^0-9a-z]/gi, '').length < 52}
                onClick={() =>
                  void run(async () => {
                    await chatRestore(code)
                    setCode('')
                  })
                }
              >
                Restore
              </Button>
            </>
          }
        >
          <p className="text-sm text-foreground">
            Type the recovery code you saved when you turned on your key backup. {Here()} then uses the same chat
            identity as your other devices, and your friends see no warning.
          </p>
          <textarea
            autoFocus
            value={code}
            onChange={(e) => setCode(e.target.value)}
            rows={3}
            spellCheck={false}
            placeholder="XXXX-XXXX-XXXX-..."
            aria-label="Recovery code"
            className="kryo-radius w-full resize-none border border-border bg-background px-3 py-2 font-mono text-sm uppercase tracking-wider text-foreground outline-none focus:border-foreground"
          />
          <p className="text-xs text-muted-foreground">Messages from before are not on {here()}: history stays on each device.</p>
        </Modal>
      ) : null}

      {asking === 'reset' ? (
        <Modal
          title="Start a new chat identity?"
          onClose={() => setAsking(null)}
          footer={
            <>
              <Button onClick={() => setAsking(null)}>Cancel</Button>
              <Button variant="danger" onClick={() => run(chatResetIdentity)}>
                Start over
              </Button>
            </>
          }
        >
          <p className="text-sm text-foreground">
            Use this only if you no longer have your other device or your recovery code. Your friends will see that
            your security key changed, anyone who verified you has to verify you again, and chat on your other devices
            stops until you set them up again. Your key backup is deleted.
          </p>
        </Modal>
      ) : null}

      {asking === 'remove' ? (
        <Modal
          title={`Remove chat from ${here()}?`}
          onClose={() => setAsking(null)}
          footer={
            <>
              <Button onClick={() => setAsking(null)}>Cancel</Button>
              <Button variant="danger" onClick={() => run(chatRemoveDevice)}>
                Remove
              </Button>
            </>
          }
        >
          <p className="text-sm text-foreground">
            This deletes the chat keys and message history on {here()}, and signs it out of chat. It cannot be undone.
          </p>
          <p className="text-xs text-muted-foreground">Your account and your friends are not affected.</p>
        </Modal>
      ) : null}
    </div>
  )
}

/**
 * The web chat's notifications while its tab is closed. The push carries no
 * message (it is end-to-end encrypted and stays on the server), so the notice
 * only says that something is waiting.
 */
function BrowserNotifications() {
  const [state, setState] = useState<PushState | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    void chatPushState()
      .then(setState)
      .catch(() => setState(null))
  }, [])
  if (!state?.supported) return null
  if (state.embedded) {
    return (
      <div className="grid gap-1.5 border-t border-border pt-3">
        <div className="flex items-center justify-between gap-3">
          <span className="text-xs text-foreground">{state.on ? "Notifications are on, through kryo.to's." : 'Get a notification when a message arrives and kryo.to is in the background.'}</span>
          {state.on ? null : (
            <Button
              size="sm"
              disabled={busy || state.permission === 'denied'}
              onClick={() => {
                setBusy(true)
                setError(null)
                void chatPushEnable()
                  .then(() => chatPushState().then(setState))
                  .catch((e: unknown) => setError(errorText(e)))
                  .finally(() => setBusy(false))
              }}
            >
              <Bell className="size-3" aria-hidden />
              Turn on
            </Button>
          )}
        </div>
        <p className="text-[11px] text-muted-foreground">
          {state.permission === 'denied'
            ? "Notifications are blocked for kryo.to. Allow them in the browser's site settings first."
            : "Shown while a kryo.to tab is open, using kryo.to's own notifications."}
        </p>
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
      </div>
    )
  }
  const flip = () => {
    setBusy(true)
    setError(null)
    // No await before this: the permission prompt must open inside the click.
    void (state.on ? chatPushDisable() : chatPushEnable())
      .then(() => chatPushState().then(setState))
      .catch((e: unknown) => setError(errorText(e)))
      .finally(() => setBusy(false))
  }
  return (
    <div className="grid gap-1.5 border-t border-border pt-3">
      <div className="flex items-center justify-between gap-3">
        <span className="text-xs text-foreground">{state.on ? 'Notifications are on in this browser.' : 'Get a notification when a message arrives and this tab is closed.'}</span>
        <Button size="sm" disabled={busy || (!state.on && state.permission === 'denied')} onClick={flip}>
          {state.on ? <BellOff className="size-3" aria-hidden /> : <Bell className="size-3" aria-hidden />}
          {state.on ? 'Turn off' : 'Turn on'}
        </Button>
      </div>
      {state.permission === 'denied' && !state.on ? (
        <p className="text-[11px] text-muted-foreground">Notifications are blocked for this site. Allow them in the browser's site settings first.</p>
      ) : (
        <p className="text-[11px] text-muted-foreground">The notice never shows who wrote or what: only that something is waiting.</p>
      )}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
  )
}
