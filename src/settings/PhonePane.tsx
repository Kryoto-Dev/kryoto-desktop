import { useCallback, useEffect, useState } from 'react'
import { ArrowLeft, ChevronRight, Copy, Smartphone } from 'lucide-react'
import { Button, Caption, Check, Section } from '@/ui'
import { call, errorText, on } from '@/lib/bridge'
import { formatLastPlayed } from '@/lib/format'
import { QrCode } from '@/settings/QrCode'
import { openExternal } from '@/lib/window'

const MOBILE_URL = 'https://kryo.to/mobile'

type PhoneStatus = {
  lan: boolean
  addresses: string[]
  code: string | null
  codeExpiresIn: number | null
  /** What the QR code holds; null without a live code. */
  pairUri: string | null
  devices: { id: string; name: string; pairedAt: number; lastSeen: number }[]
  /** When Kryoto Mobile last reached this PC through kryo.to (Unix ms). */
  remoteSeenAt?: number | null
}

/** A phone reaching this PC through kryo.to counts as connected for this long after it last did. */
const REMOTE_LIVE_MS = 2 * 60_000

/** Remote for the signed-in kryo.to account (remote.rs, kryo.to lib/remote-consent.ts). */
type RemoteConsent = { signedIn: boolean; enabled: boolean; decided: boolean }

/**
 * Remote, away from home: off until the account turns it on, here or on
 * kryo.to/remote. The answer is the account's, so turning it on here turns it
 * on in Kryoto Mobile, Kryoto Forge and on kryo.to too, and off stops this PC
 * at once. Separate from pairing on this network, which never goes through
 * kryo.to.
 */
function RemoteSection() {
  const [consent, setConsent] = useState<RemoteConsent | null>(null)
  const [saving, setSaving] = useState(false)
  const [note, setNote] = useState<string | null>(null)

  useEffect(() => {
    let alive = true
    void call<RemoteConsent>('remote_status').then((c) => alive && setConsent(c)).catch(() => {})
    const offs = [
      on<RemoteConsent>('remote-consent', (c) => setConsent(c)),
      on<{ ok: boolean; error: string | null }>('remote-consent-done', (r) => {
        setSaving(false)
        setNote(r.ok ? null : (r.error ?? 'kryo.to did not save that.'))
      }),
    ]
    return () => {
      alive = false
      for (const off of offs) void off.then((f) => f())
    }
  }, [])

  const set = (enabled: boolean) => {
    setSaving(true)
    setNote(null)
    void call('remote_set_consent', { enabled }).catch((e: unknown) => {
      setSaving(false)
      setNote(errorText(e))
    })
  }

  if (!consent) return null
  return (
    <Section
      title="Remote, away from home"
      hint={
        consent.enabled
          ? 'On for your kryo.to account. While Kryoto Desktop is open, this PC tells kryo.to its name, system and version, its games and what is running, and its downloads. From kryo.to/remote and Kryoto Mobile away from home you can send it games, pause, resume, cancel or clear downloads, start and stop games, and take games off it.'
          : 'Off. Nothing about this PC goes to kryo.to for Remote, and nothing can be started on it from there. Turn it on to see this PC, its games and downloads on kryo.to/remote and in Kryoto Mobile away from home, and to send it games, start and stop games, pause, resume, cancel or clear downloads, and take games off it from there. It is for your whole kryo.to account, and you can turn it off again here or on kryo.to.'
      }
    >
      {!consent.signedIn ? (
        <span className="text-[11px] text-muted-foreground">Sign in to kryo.to in the Store to choose.</span>
      ) : (
        <div className="flex flex-wrap items-center gap-3">
          <Button size="sm" variant={consent.enabled ? 'outline' : 'primary'} disabled={saving} onClick={() => set(!consent.enabled)}>
            {saving ? 'Saving' : consent.enabled ? 'Turn off Remote' : 'Turn on Remote'}
          </Button>
          <Caption>{consent.enabled ? 'Remote is on' : consent.decided ? 'Remote is off' : 'Not turned on yet'}</Caption>
        </div>
      )}
      {note ? <span className="text-[11px] text-destructive">{note}</span> : null}
      <span className="text-[11px] text-muted-foreground">A phone paired on this network below works either way.</span>
    </Section>
  )
}

/**
 * Settings > Phone: let Kryoto Mobile see the library and start games, on this
 * network (a paired phone talks to this PC directly) or through kryo.to.
 */
export function PhonePane() {
  const [status, setStatus] = useState<PhoneStatus | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [manual, setManual] = useState(false)

  const refresh = useCallback(async () => {
    await call<PhoneStatus>('phone_status')
      .then(setStatus)
      .catch((e: unknown) => setNote(errorText(e)))
  }, [])
  useEffect(() => {
    void refresh()
    const t = window.setInterval(() => void refresh(), 2000)
    return () => window.clearInterval(t)
  }, [refresh])

  const run = (command: string, args?: Record<string, unknown>) => {
    setNote(null)
    void call(command, args)
      .catch((e: unknown) => setNote(errorText(e)))
      .then(refresh)
  }

  if (!status) return <Caption>{note ?? 'Loading...'}</Caption>
  return (
    <>
      <Section title="Kryoto Mobile" hint="The Android app for this PC: its games, Play and Stop, and downloads, at home or away.">
        <Button size="sm" className="w-fit" onClick={() => void openExternal(MOBILE_URL)}>
          <Smartphone className="size-3" />
          Get Kryoto Mobile
        </Button>
      </Section>
      <RemoteSection />
      <Section
        title="On this network"
        hint="Windows may ask to allow Kryoto on private networks. Anyone on your network with the code can pair, so only use a network you trust."
      >
        <Check checked={status.lan} onChange={(on) => run('phone_set_lan', { on })} label="Let phones on this network see my games and start them" />
        {status.lan ? (
          <div className="grid gap-3">
            {!manual ? <div className="grid justify-items-center gap-3 py-3 text-center">
              <Caption>Scan with Kryoto Mobile</Caption>
              {status.pairUri ? (
                <div className="grid justify-items-center gap-3">
                  <QrCode text={status.pairUri} label="QR code to pair a phone" className="size-64 max-w-full shrink-0 rounded-md" />
                  <span className="max-w-xs text-[11px] leading-relaxed text-muted-foreground">
                    Open Library in Kryoto Mobile and tap Scan QR code. Connect on your Wi-Fi, or sign in to the same kryo.to account on both devices.
                  </span>
                </div>
              ) : (
                <span className="text-[11px] text-muted-foreground">
                  {status.addresses.length ? 'Make a new code to show the QR code.' : 'No private network address found. Connect this PC to Wi-Fi or Ethernet.'}
                </span>
              )}
            </div> : null}
            <Button size="sm" className="w-fit" onClick={() => setManual(!manual)} aria-expanded={manual}>
              {manual ? <ArrowLeft className="size-3" /> : <ChevronRight className="size-3" />}
              {manual ? 'Back to QR code' : 'Enter details manually'}
            </Button>
            {manual ? <div className="phone-panel-in grid gap-3">
            <div className="grid gap-1.5">
              <Caption>Or type them in: address (the first one is usually right)</Caption>
              {status.addresses.map((a) => (
                <div key={a} className="flex items-center gap-2">
                  <code className="select-text text-sm text-foreground">{a}</code>
                  <Button
                    size="sm"
                    onClick={() =>
                      void navigator.clipboard
                        .writeText(a)
                        .then(() => setNote('Copied.'))
                        .catch(() => setNote('Could not copy.'))
                    }
                  >
                    <Copy className="size-3" />
                    Copy
                  </Button>
                </div>
              ))}
            </div>
            <div className="grid gap-1.5">
              <Caption>Code</Caption>
              <div className="flex flex-wrap items-center gap-3">
                {status.code ? (
                  <>
                    <span className="select-text text-2xl tracking-[0.3em] text-foreground">{status.code}</span>
                    <span className="text-[11px] text-muted-foreground">expires in {Math.max(1, Math.ceil((status.codeExpiresIn ?? 0) / 60))} min</span>
                  </>
                ) : (
                  <span className="text-[11px] text-muted-foreground">No code. Make one, then scan or type it on the phone.</span>
                )}
              </div>
            </div>
            </div> : null}
            <div className="flex items-center gap-3">
              <Button size="sm" onClick={() => run('phone_new_code')}>New code</Button>
              <Caption>{status.code ? `Expires in ${Math.max(1, Math.ceil((status.codeExpiresIn ?? 0) / 60))} min` : 'Make a new code to connect.'}</Caption>
            </div>
          </div>
        ) : null}
        {note ? <span className="text-[11px] text-muted-foreground">{note}</span> : null}
      </Section>
      <Section title="Phones">
        {status.remoteSeenAt ? (
          <div className="flex items-center justify-between gap-3">
            <div className="grid min-w-0">
              <span className="truncate text-xs text-foreground">Kryoto Mobile, away from home</span>
              <Caption>
                {Date.now() - status.remoteSeenAt < REMOTE_LIVE_MS
                  ? 'Connected now, through kryo.to'
                  : `Through kryo.to · last seen ${formatLastPlayed(Math.floor(status.remoteSeenAt / 1000))}`}
              </Caption>
            </div>
          </div>
        ) : null}
        {status.devices.length ? (
          status.devices.map((d) => (
            <div key={d.id} className="flex items-center justify-between gap-3">
              <div className="grid min-w-0">
                <span className="truncate text-xs text-foreground">{d.name}, on this network</span>
                <Caption>Last seen {formatLastPlayed(d.lastSeen)}</Caption>
              </div>
              <Button size="sm" onClick={() => run('phone_revoke', { id: d.id })}>
                Remove
              </Button>
            </div>
          ))
        ) : status.remoteSeenAt ? null : (
          <span className="text-[11px] text-muted-foreground">
            No phones yet. Pair one on this network above, or turn on Remote and sign in to the same kryo.to account in Kryoto Mobile.
          </span>
        )}
      </Section>
    </>
  )
}
