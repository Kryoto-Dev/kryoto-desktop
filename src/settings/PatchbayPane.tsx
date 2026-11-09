import { useEffect, useState } from 'react'
import { ArrowUpRight, Check as CheckIcon, Cloud, KeyRound, Trophy, Zap } from '@/ui/pixel-icons'
import { Button, Section, Segmented, inputCls } from '@/ui'
import { errorText } from '@/lib/bridge'
import { settingsApi, type Settings } from '@/lib/settings'
import { patchbay, usePatchbay, type PatchbaySlot } from '@/lib/patchbay'
import { openExternal } from '@/lib/window'
import { cn } from '@/lib/utils'

const DOCS = 'https://github.com/Kryoto-Dev/kryoto-desktop/blob/main/docs/patchbay.md'
const RD_TOKEN = 'https://real-debrid.com/apitoken'
const TORBOX_TOKEN = 'https://torbox.app/settings'

/**
 * Your own services plugged into Kryoto Desktop (src-tauri patchbay/): a cloud
 * saves server and an achievements server instead of kryo.to's, in Settings >
 * Cloud, and Real-Debrid or TorBox in front of downloads, in Settings >
 * Downloads, where a player looks for them. Addresses save when applied;
 * tokens go to the OS secure store and never come back to this page.
 */
export function PatchbayPane({ s, onSaved }: { s: Settings; onSaved: (s: Settings) => void }) {
  const [status, setStatus] = usePatchbay()

  const save = async (p: Partial<Settings>) => {
    const latest = await settingsApi.get()
    const saved = await settingsApi.save({ ...latest, ...p })
    onSaved(saved)
    return saved
  }

  return (
    <>
      <p className="text-xs leading-relaxed text-muted-foreground">
        A server of your own gets your saves or achievements instead of kryo.to, and kryo.to never sees them or your tokens. Switch back to kryo.to any time: each
        place keeps its own.
      </p>

      <Endpoint
        slot="saves"
        icon={<Cloud className="size-3.5" aria-hidden />}
        title="Cloud saves server"
        hint="Games back their saves up here when they close, and newer saves come back before they start. Your Cloud settings (when to back up, how saves are found) still apply."
        value={s.savesEndpoint ?? ''}
        hasToken={!!status?.saves.token}
        onApply={async (endpoint) => (await save({ savesEndpoint: endpoint })).savesEndpoint}
        onToken={async (t) => setStatus(await patchbay.setToken('saves', t))}
      />
      <Endpoint
        slot="achievements"
        icon={<Trophy className="size-3.5" aria-hidden />}
        title="Achievements server"
        hint="Achievements you unlock are sent here every half minute, and ones from your other PCs come back into your games."
        value={s.achievementsEndpoint ?? ''}
        hasToken={!!status?.achievements.token}
        onApply={async (endpoint) => (await save({ achievementsEndpoint: endpoint })).achievementsEndpoint}
        onToken={async (t) => setStatus(await patchbay.setToken('achievements', t))}
      />
      <Button size="sm" variant="ghost" className="justify-self-start" onClick={() => openExternal(DOCS)}>
        How to run your own server
        <ArrowUpRight className="size-3" aria-hidden />
      </Button>
    </>
  )
}

/** Real-Debrid or TorBox in front of downloads, in Settings > Downloads. */
export function DebridSection({ s, onSaved }: { s: Settings; onSaved: (s: Settings) => void }) {
  const [status, setStatus] = usePatchbay()
  const save = async (p: Partial<Settings>) => {
    const latest = await settingsApi.get()
    const saved = await settingsApi.save({ ...latest, ...p })
    onSaved(saved)
    return saved
  }
  return (
      <Section
        title="Debrid"
        hint="Downloads from hosts your debrid service supports go through it first: faster, more connections, no waiting pages. If it can't unlock a link, Kryoto downloads it the usual way."
      >
        <Segmented
          label="Debrid service"
          value={s.debrid || 'off'}
          options={[
            { value: 'off', label: 'Off' },
            { value: 'real-debrid', label: 'Real-Debrid' },
            { value: 'torbox', label: 'TorBox' },
          ]}
          onChange={(v) => void save({ debrid: v === 'off' ? '' : (v as Settings['debrid']) })}
        />
        {s.debrid === 'real-debrid' ? (
          <TokenField
            key="real-debrid"
            slot="real-debrid"
            label="Real-Debrid API token"
            hasToken={!!status?.debridToken}
            onToken={async (t) => setStatus(await patchbay.setToken('real-debrid', t))}
            extra={
              <Button size="sm" variant="ghost" onClick={() => openExternal(RD_TOKEN)}>
                Get your token
                <ArrowUpRight className="size-3" aria-hidden />
              </Button>
            }
          />
        ) : s.debrid === 'torbox' ? (
          <TokenField
            key="torbox"
            slot="torbox"
            label="TorBox API key"
            hasToken={!!status?.torboxToken}
            onToken={async (t) => setStatus(await patchbay.setToken('torbox', t))}
            extra={
              <Button size="sm" variant="ghost" onClick={() => openExternal(TORBOX_TOKEN)}>
                Get your key
                <ArrowUpRight className="size-3" aria-hidden />
              </Button>
            }
          />
        ) : null}
      </Section>
  )
}

function Endpoint({
  slot,
  icon,
  title,
  hint,
  value,
  hasToken,
  onApply,
  onToken,
}: {
  slot: PatchbaySlot
  icon: React.ReactNode
  title: string
  hint: string
  value: string
  hasToken: boolean
  onApply: (endpoint: string) => Promise<string>
  onToken: (token: string | null) => Promise<void>
}) {
  const [draft, setDraft] = useState(value)
  const [note, setNote] = useState<{ ok: boolean; text: string } | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => setDraft(value), [value])

  const apply = () => {
    setBusy(true)
    setNote(null)
    void onApply(draft.trim())
      .then((saved) => {
        setDraft(saved)
        setNote({ ok: true, text: saved ? `Using ${new URL(saved).host}.` : 'Back on kryo.to.' })
      })
      .catch((e: unknown) => setNote({ ok: false, text: errorText(e) }))
      .finally(() => setBusy(false))
  }

  return (
    <div className="kryo-radius grid gap-3 border border-border p-4">
      <span className="flex items-center gap-2 text-xs font-bold text-foreground">
        {icon}
        {title}
        <span className={cn('ml-auto text-[10px] font-normal uppercase tracking-wider', value ? 'text-primary' : 'text-muted-foreground')}>
          {value ? 'Your server' : 'kryo.to'}
        </span>
      </span>
      <p className="text-[11px] leading-relaxed text-muted-foreground">{hint}</p>
      <label className="grid gap-1.5 text-[10px] uppercase tracking-wider text-muted-foreground">
        Address
        <span className="flex gap-2">
          <input className={cn(inputCls, 'grow')} type="url" value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="Blank uses kryo.to" spellCheck={false} />
          <Button size="sm" variant="primary" disabled={busy || draft.trim() === value} onClick={apply}>
            {draft.trim() || !value ? 'Apply' : 'Use kryo.to'}
          </Button>
        </span>
      </label>
      {value ? <TokenField slot={slot} label="Token (optional)" hasToken={hasToken} onToken={onToken} /> : null}
      {note ? <p className={cn('text-[11px]', note.ok ? 'text-muted-foreground' : 'text-destructive')}>{note.text}</p> : null}
    </div>
  )
}

/**
 * A token goes in and never comes back out: once kept, the field only says
 * so, with Replace and Remove.
 */
function TokenField({
  slot,
  label,
  hasToken,
  onToken,
  extra,
}: {
  slot: PatchbaySlot
  label: string
  hasToken: boolean
  onToken: (token: string | null) => Promise<void>
  extra?: React.ReactNode
}) {
  const [draft, setDraft] = useState('')
  const [editing, setEditing] = useState(false)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null)

  const run = (p: Promise<unknown>, after?: () => void) => {
    setBusy(true)
    setResult(null)
    void p
      .then(() => after?.())
      .catch((e: unknown) => setResult({ ok: false, text: errorText(e) }))
      .finally(() => setBusy(false))
  }
  const test = () => {
    setBusy(true)
    setResult(null)
    void patchbay
      .test(slot)
      .then((text) => setResult({ ok: true, text }))
      .catch((e: unknown) => setResult({ ok: false, text: errorText(e) }))
      .finally(() => setBusy(false))
  }

  return (
    <div className="grid gap-2">
      <span className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider text-muted-foreground">
        <KeyRound className="size-3" aria-hidden />
        {label}
      </span>
      {hasToken && !editing ? (
        <span className="flex flex-wrap items-center gap-2">
          <span className="flex items-center gap-1.5 text-[11px] text-foreground">
            <CheckIcon className="size-3 text-primary" aria-hidden />
            Kept in this PC's secure store
          </span>
          <Button size="sm" onClick={() => setEditing(true)}>Replace</Button>
          <Button size="sm" variant="ghost" disabled={busy} onClick={() => run(onToken(null))}>Remove</Button>
        </span>
      ) : (
        <span className="flex gap-2">
          <input
            className={cn(inputCls, 'grow')}
            type="password"
            autoComplete="off"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder="Paste the token"
            spellCheck={false}
          />
          <Button
            size="sm"
            variant="primary"
            disabled={busy || !draft.trim()}
            onClick={() =>
              run(onToken(draft), () => {
                setDraft('')
                setEditing(false)
              })
            }
          >
            Keep
          </Button>
        </span>
      )}
      <span className="flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={busy} onClick={test}>
          <Zap className="size-3" aria-hidden />
          Test
        </Button>
        {extra}
      </span>
      {result ? <p className={cn('text-[11px]', result.ok ? 'text-foreground' : 'text-destructive')}>{result.text}</p> : null}
    </div>
  )
}
