import { useCallback, useEffect, useState } from 'react'
import { ArrowLeft, ChevronRight, Copy, Smartphone } from 'lucide-react'
import { Button, Caption, Check, Section } from '@/ui'
import { call, errorText } from '@/lib/bridge'
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
            No phones yet. Pair one on this network above, or sign in to the same kryo.to account in Kryoto Mobile.
          </span>
        )}
      </Section>
    </>
  )
}
