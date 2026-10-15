import { useCallback, useEffect, useState } from 'react'
import { Copy } from 'lucide-react'
import { Button, Caption, Check, Section } from '@/ui'
import { call, errorText } from '@/lib/bridge'
import { formatLastPlayed } from '@/lib/format'
import { QrCode } from '@/settings/QrCode'

type PhoneStatus = {
  lan: boolean
  addresses: string[]
  code: string | null
  codeExpiresIn: number | null
  /** What the QR code holds; null without a live code or an address. */
  pairUri: string | null
  devices: { id: string; name: string; pairedAt: number; lastSeen: number }[]
}

/**
 * Settings > Phone: let Kryoto Mobile see the library and start games, on this
 * network (a paired phone talks to this PC directly) or through kryo.to.
 */
export function PhonePane() {
  const [status, setStatus] = useState<PhoneStatus | null>(null)
  const [note, setNote] = useState<string | null>(null)

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
      <Section
        title="On this network"
        hint="Windows may ask to allow Kryoto on private networks. Anyone on your network with the code can pair, so only use a network you trust."
      >
        <Check checked={status.lan} onChange={(on) => run('phone_set_lan', { on })} label="Let phones on this network see my games and start them" />
        {status.lan ? (
          <div className="grid gap-3">
            <div className="grid gap-1.5">
              <Caption>Scan with Kryoto Mobile</Caption>
              {status.pairUri ? (
                <div className="flex flex-wrap items-center gap-4">
                  <QrCode text={status.pairUri} label="QR code to pair a phone" className="size-40 shrink-0 rounded-md" />
                  <span className="max-w-[16rem] text-[11px] leading-relaxed text-muted-foreground">
                    In Kryoto Mobile, open Library and tap Scan QR code. The phone must be on the same network.
                  </span>
                </div>
              ) : (
                <span className="text-[11px] text-muted-foreground">
                  {status.addresses.length ? 'Make a new code to show the QR code.' : 'No private network address found. Connect this PC to Wi-Fi or Ethernet.'}
                </span>
              )}
            </div>
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
                <Button size="sm" onClick={() => run('phone_new_code')}>
                  New code
                </Button>
              </div>
            </div>
          </div>
        ) : null}
        {note ? <span className="text-[11px] text-muted-foreground">{note}</span> : null}
      </Section>
      <Section title="Paired phones">
        {status.devices.length ? (
          status.devices.map((d) => (
            <div key={d.id} className="flex items-center justify-between gap-3">
              <div className="grid min-w-0">
                <span className="truncate text-xs text-foreground">{d.name}</span>
                <Caption>Last seen {formatLastPlayed(d.lastSeen)}</Caption>
              </div>
              <Button size="sm" onClick={() => run('phone_revoke', { id: d.id })}>
                Remove
              </Button>
            </div>
          ))
        ) : (
          <span className="text-[11px] text-muted-foreground">No phones paired.</span>
        )}
      </Section>
    </>
  )
}
