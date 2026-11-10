import { useCallback, useEffect, useState } from 'react'
import { Copy, KeyRound, Laptop, Globe, Trash2 } from '@/ui/pixel-icons'
import { errorText } from '@/lib/bridge'
import {
  chatBackupCreate,
  chatBackupDelete,
  chatBackupStatus,
  chatDeviceRevoke,
  chatDevices,
  chatExportHistory,
  here,
  Here,
  type BackupStatus,
  type MyDevice,
} from '@/lib/chat'
import { Busy, Button, Caption, Check, Modal } from '@/ui'

function day(ms: number): string {
  return ms ? new Date(ms).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' }) : 'never'
}

/**
 * The account's key backup and chat devices. The recovery code is shown
 * once, here, right after it is made; nothing else ever displays it.
 */
export function ChatSecurity({ onClose }: { onClose: () => void }) {
  const [backup, setBackup] = useState<BackupStatus | null>(null)
  const [devices, setDevices] = useState<MyDevice[] | null>(null)
  const [code, setCode] = useState<string | null>(null)
  const [wroteDown, setWroteDown] = useState(false)
  const [confirm, setConfirm] = useState<'delete-backup' | 'new-code' | MyDevice | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState<string | null>(null)

  const load = useCallback(() => {
    void chatBackupStatus().then(setBackup).catch((e) => setError(errorText(e)))
    void chatDevices().then(setDevices).catch((e) => setError(errorText(e)))
  }, [])
  useEffect(load, [load])

  const act = async (work: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    setConfirm(null)
    try {
      await work()
      load()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  const create = () =>
    act(async () => {
      setWroteDown(false)
      setCode(await chatBackupCreate())
    })

  if (code) {
    return (
      <Modal
        title="Your recovery code"
        onClose={() => wroteDown && setCode(null)}
        footer={
          <Button variant="primary" disabled={!wroteDown} onClick={() => setCode(null)}>
            Done
          </Button>
        }
      >
        <p className="text-sm text-foreground">
          Write this down and keep it somewhere safe. With it, a new device can take over your chat identity, so your
          friends keep trusting you. It is shown only now.
        </p>
        <code className="kryo-radius block select-all border border-border bg-background p-3 font-mono text-sm leading-relaxed tracking-wider text-foreground">
          {code}
        </code>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <Button size="sm" onClick={() => navigator.clipboard.writeText(code).catch(() => {})}>
            <Copy className="size-3" aria-hidden /> Copy
          </Button>
          <Check checked={wroteDown} onChange={setWroteDown} label="I saved my recovery code" />
        </div>
        <p className="text-xs text-muted-foreground">
          Nobody, Kryoto staff included, can open your backup without this code, or give it back to you if you lose it.
        </p>
      </Modal>
    )
  }

  return (
    <Modal title="Chat security" onClose={onClose}>
      {error ? <p className="text-xs text-destructive">{error}</p> : null}

      <section className="grid gap-2">
        <Caption className="flex items-center gap-2">
          <KeyRound className="size-3" aria-hidden /> Key backup
        </Caption>
        <p className="text-xs text-muted-foreground">
          An encrypted copy of your chat identity, locked with a recovery code only you have. Without it, a new device or a
          reinstall means a new identity, and your friends see a security warning.
        </p>
        {backup == null ? (
          <p className="text-xs text-muted-foreground">Checking...</p>
        ) : backup.exists ? (
          <p className="text-xs text-foreground">
            Backed up, last updated {day(backup.updatedAtMs)}.
            {backup.keptHere ? ` ${Here()} keeps it up to date.` : ''}
          </p>
        ) : (
          <p className="text-xs text-foreground">No backup yet.</p>
        )}
        <div className="flex flex-wrap gap-2">
          <Button variant="primary" size="sm" disabled={busy || backup == null} onClick={() => (backup?.exists ? setConfirm('new-code') : void create())}>
            {backup?.exists ? 'Make a new recovery code' : 'Turn on backup'}
          </Button>
          {backup?.exists ? (
            <Button variant="danger" size="sm" disabled={busy} onClick={() => setConfirm('delete-backup')}>
              <Trash2 className="size-3" aria-hidden /> Delete backup
            </Button>
          ) : null}
        </div>
      </section>

      <section className="grid gap-2">
        <Caption>Devices with chat</Caption>
        {devices == null && !error ? <Busy className="size-3.5 text-muted-foreground" label="Loading devices" /> : null}
        <ul className="grid gap-1.5">
          {devices?.map((d) => (
            <li key={d.deviceId} className="kryo-radius flex items-center justify-between gap-3 border border-border px-3 py-2">
              <span className="flex min-w-0 items-center gap-2.5">
                {d.kind === 'web' ? <Globe className="size-4 shrink-0 text-muted-foreground" aria-hidden /> : <Laptop className="size-4 shrink-0 text-muted-foreground" aria-hidden />}
                <span className="grid min-w-0">
                  <b className="truncate text-xs text-foreground">
                    {d.name || (d.kind === 'web' ? 'Web browser' : 'PC')}
                    {d.current ? ` (${here()})` : ''}
                  </b>
                  <span className="text-[11px] text-muted-foreground">
                    Added {day(d.createdAtMs)}, last used {day(d.lastSeenDayMs)}
                    {d.certified ? '' : ' - not set up with your current identity'}
                  </span>
                </span>
              </span>
              {!d.current ? (
                <Button size="sm" variant="danger" disabled={busy} onClick={() => setConfirm(d)}>
                  Remove
                </Button>
              ) : null}
            </li>
          ))}
        </ul>
      </section>

      <section className="grid gap-2">
        <Caption>Your messages</Caption>
        <p className="text-xs text-muted-foreground">
          Your chat history lives only on your devices. Save a copy of everything on {here()} as a file. The file is not
          encrypted: anyone who opens it can read it.
        </p>
        <div>
          <Button
            size="sm"
            disabled={busy}
            onClick={() =>
              void act(async () => {
                const where = await chatExportHistory()
                if (where) setSaved(where)
              })
            }
          >
            Save chat history
          </Button>
        </div>
        {saved ? <p className="text-[11px] text-muted-foreground">Saved to {saved}</p> : null}
      </section>

      {confirm === 'delete-backup' ? (
        <Modal
          title="Delete your key backup?"
          onClose={() => setConfirm(null)}
          footer={
            <>
              <Button onClick={() => setConfirm(null)}>Cancel</Button>
              <Button variant="danger" onClick={() => act(chatBackupDelete)}>
                Delete
              </Button>
            </>
          }
        >
          <p className="text-sm text-foreground">Your recovery code stops working. Chat on {here()} keeps working.</p>
        </Modal>
      ) : null}
      {confirm === 'new-code' ? (
        <Modal
          title="Make a new recovery code?"
          onClose={() => setConfirm(null)}
          footer={
            <>
              <Button onClick={() => setConfirm(null)}>Cancel</Button>
              <Button variant="primary" onClick={() => create()}>
                Make a new code
              </Button>
            </>
          }
        >
          <p className="text-sm text-foreground">The old recovery code stops working as soon as the new one is made.</p>
        </Modal>
      ) : null}
      {confirm && typeof confirm === 'object' ? (
        <Modal
          title={`Remove ${confirm.name || 'this device'}?`}
          onClose={() => setConfirm(null)}
          footer={
            <>
              <Button onClick={() => setConfirm(null)}>Cancel</Button>
              <Button variant="danger" onClick={() => act(() => chatDeviceRevoke(confirm.deviceId))}>
                Remove
              </Button>
            </>
          }
        >
          <p className="text-sm text-foreground">
            It is signed out of chat at once and stops receiving messages. Use this for a lost or old device.
          </p>
        </Modal>
      ) : null}
    </Modal>
  )
}
