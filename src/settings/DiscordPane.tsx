import { useEffect, useState } from 'react'
import { ChevronDown } from '@/ui/pixel-icons'
import { Check, Dropdown, Section, Segmented, inputCls } from '@/ui'
import { call, isTauri } from '@/lib/bridge'
import { artSrc } from '@/lib/art'
import type { DiscordPresence, PresenceButton, Settings } from '@/lib/settings'
import { useDiscord } from '@/lib/discord'
import { DiscordChip } from '@/library/DiscordChip'

/**
 * Settings > General > Discord: what shows beside the player's name while a
 * game runs (src-tauri discord_rpc/). One switch and a preview on the surface;
 * the words, picture and buttons fold away under it.
 */

type Status = {
  connected: boolean
  preview: {
    details?: string
    state?: string
    assets?: { large_image?: string | null }
    timestamps?: { start: number }
    buttons?: { label: string; url: string }[]
  } | null
}

const KINDS: { value: PresenceButton['kind']; label: string }[] = [
  { value: 'desktop', label: 'Get Kryoto Desktop' },
  { value: 'game', label: "The game's page" },
  { value: 'donate', label: 'Donate to Kryoto' },
  { value: 'profile', label: 'My Kryoto profile' },
  { value: 'custom', label: 'My own link' },
  { value: 'none', label: 'No button' },
]

/** Only the second button: it stays in the list so it can be chosen again. */
const ROTATE = { value: 'rotate' as const, label: 'Game page and Donate, taking turns' }

const LABELS: Record<PresenceButton['kind'], string> = {
  desktop: 'Get Kryoto Desktop',
  rotate: '',
  game: 'Open in Kryoto',
  donate: 'Donate to Kryoto',
  profile: 'My Kryoto profile',
  custom: 'Words on the button',
  none: '',
}

export function DiscordPane({ s, patch }: { s: Settings; patch: (p: Partial<Settings>) => void }) {
  const d = s.discord
  const set = (p: Partial<DiscordPresence>) => patch({ discord: { ...d, ...p } })
  const [status, setStatus] = useState<Status | null>(null)

  useEffect(() => {
    let off = false
    const look = () => void call<Status>('discord_presence_status').then((v) => !off && setStatus(v)).catch(() => {})
    look()
    if (!isTauri()) return
    const t = window.setInterval(look, 5000)
    return () => {
      off = true
      window.clearInterval(t)
    }
  }, [d])

  return (
    <Section title="Discord" hint="Anyone who can see you on Discord sees what you play here, with the words and links you pick.">
      <Check checked={d.on} onChange={(on) => set({ on })} label="Show what I play on Discord" />
      {d.on ? (
        <>
          <Preview status={status} />
          <SetDifferently />
          <details className="group grid">
            <summary className="flex w-fit cursor-pointer list-none items-center gap-1.5 text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:text-foreground">
              <ChevronDown className="size-3 transition-transform group-open:rotate-180" aria-hidden />
              Words, picture and buttons
            </summary>
            <div className="grid gap-5 pt-4">
              <div className="grid gap-2">
                <Line label="First line" value={d.line1} onSave={(line1) => set({ line1 })} />
                <Line label="Second line" value={d.line2} onSave={(line2) => set({ line2 })} />
                <p className="text-[11px] text-muted-foreground">
                  {'Use {game}, {playtime}, {version} and {developer}. A line that needs something a game does not have is left out.'}
                </p>
              </div>
              <Segmented
                label="Picture"
                value={d.picture}
                options={[
                  { value: 'cover', label: 'Game cover' },
                  { value: 'kryoto', label: 'Kryoto logo' },
                  { value: 'none', label: 'None' },
                ]}
                onChange={(picture) => set({ picture })}
              />
              <div className="grid gap-2">
                <Check checked={d.elapsed} onChange={(elapsed) => set({ elapsed })} label="Show how long I've been playing" />
                <Check checked={d.playtime} onChange={(playtime) => set({ playtime })} label="Show my total play time" />
                <Check checked={d.browsing} onChange={(browsing) => set({ browsing })} label="Show Kryoto when no game is running" />
                <Check checked={d.adultNames} onChange={(adultNames) => set({ adultNames })} label="Show the name and art of adult games" />
                <p className="pl-6.5 text-[11px] text-muted-foreground">Anyone who can see you on Discord will see them. Off, it just says Playing a game.</p>
              </div>
              <div className="grid gap-3">
                <ButtonRow name="First button" value={d.button1} kinds={KINDS} onChange={(button1) => set({ button1 })} />
                <ButtonRow name="Second button" value={d.button2} kinds={[ROTATE, ...KINDS]} onChange={(button2) => set({ button2 })} />
                {d.button2.kind === 'rotate' ? <p className="text-[11px] text-muted-foreground">They swap every 5 minutes. Pick one to keep it there.</p> : null}
              </div>
            </div>
          </details>
        </>
      ) : null}
    </Section>
  )
}

/** What other people would see, on the game last played. */
function Preview({ status }: { status: Status | null }) {
  const p = status?.preview
  return (
    <div className="grid gap-1.5">
      {p ? (
        <div className="kryo-radius flex w-fit max-w-md items-center gap-3 border border-border p-2.5">
          {p.assets?.large_image ? (
            <img
              src={artSrc(p.assets.large_image) ?? undefined}
              alt=""
              className="size-14 shrink-0 bg-secondary object-cover"
              style={{ borderRadius: 'min(var(--kryo-radius), 4px)' }}
              onError={(e) => (e.currentTarget.style.visibility = 'hidden')}
            />
          ) : null}
          <span className="grid min-w-0 gap-0.5">
            <span className="truncate text-sm font-bold text-foreground">{p.details}</span>
            {p.state ? <span className="truncate text-xs text-muted-foreground">{p.state}</span> : null}
            {p.timestamps ? <span className="text-[11px] text-muted-foreground">00:12 elapsed</span> : null}
            {p.buttons?.length ? <span className="truncate text-[11px] text-muted-foreground">{p.buttons.map((b) => b.label).join(' · ')}</span> : null}
          </span>
        </div>
      ) : null}
      <Standing />
    </div>
  )
}

/** Where it stands right now, in the same words as a game's page. */
function Standing() {
  const d = useDiscord()
  const live = d?.games.find((g) => g.now)
  const text = live ? `Showing ${live.title} on Discord now.` : d?.open ? 'Discord is open. Shows while a game runs.' : "Discord isn't open. Shows while a game runs and it is."
  return <span className="text-[11px] text-muted-foreground">{text}</span>
}

/** The games that do not follow the switches above, to change in one place. */
function SetDifferently() {
  const games = useDiscord()?.games.filter((g) => g.set) ?? []
  if (!games.length) return null
  return (
    <div className="grid gap-1.5">
      <span className="text-[10px] uppercase tracking-wider text-muted-foreground">Set differently</span>
      {games.map((g) => (
        <DiscordChip key={g.id} id={g.id} named />
      ))}
    </div>
  )
}

/** Saved when the box is left, not at every letter. */
function Line({ label, value, placeholder, onSave }: { label: string; value: string; placeholder?: string; onSave: (v: string) => void }) {
  const [text, setText] = useState(value)
  useEffect(() => setText(value), [value])
  return (
    <label className="grid gap-1 text-[11px] text-muted-foreground">
      {label}
      <input className={inputCls} value={text} placeholder={placeholder} onChange={(e) => setText(e.target.value)} onBlur={() => text !== value && onSave(text)} />
    </label>
  )
}

function ButtonRow({ name, value, kinds, onChange }: { name: string; value: PresenceButton; kinds: { value: PresenceButton['kind']; label: string }[]; onChange: (b: PresenceButton) => void }) {
  return (
    <div className="grid gap-1.5">
      <span className="flex items-center gap-3 text-[11px] text-muted-foreground">
        {name}
        <Dropdown size="sm" label={name} value={value.kind} options={kinds} onChange={(kind) => onChange({ ...value, kind })} className="w-52" />
      </span>
      {value.kind !== 'none' && value.kind !== 'rotate' ? <Line label="Words on the button" value={value.label} placeholder={LABELS[value.kind]} onSave={(label) => onChange({ ...value, label })} /> : null}
      {value.kind === 'custom' ? <Line label="Link" value={value.url} placeholder="https://" onSave={(url) => onChange({ ...value, url })} /> : null}
    </div>
  )
}
