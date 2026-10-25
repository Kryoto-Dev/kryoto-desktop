import { useEffect, useState } from 'react'
import { ProfileLink, TeamName } from '@/ui/TeamName'
import { Headphones, HeadphoneOff, Mic, MicOff, Phone, PhoneOff, Settings2, WifiOff } from 'lucide-react'
import {
  accept,
  audioDevices,
  callState,
  chooseInput,
  chooseOutput,
  clock,
  decline,
  hangUp,
  levels,
  startCallListener,
  subscribe,
  toggleDeafen,
  toggleMute,
  type AudioDevice,
  type AudioDevices,
  type CallView,
} from '@/lib/calls'
import { artSrc } from '@/lib/art'
import { asciiTrack, Button, Caption, Dropdown, IconButton } from '@/ui'
import { cn } from '@/lib/utils'

/** "The system's" device, as a choice in the lists. */
const SYSTEM = '__system__'

/**
 * The call card, over everything while a call is ringing or on. Lives in the
 * shell (not a page), so a call goes on while you browse: who, how long, who
 * is talking, mute, deafen, the devices, hang up.
 */
export function CallOverlay({ nameOf, avatarOf, usernameOf }: { nameOf: (id: string) => string; avatarOf?: (id: string) => string | null; usernameOf?: (id: string) => string | null }) {
  const [s, setS] = useState<CallView>(callState())
  const [now, setNow] = useState(Date.now())
  const [level, setLevel] = useState({ local: 0, remote: 0 })
  const [devicesOpen, setDevicesOpen] = useState(false)
  useEffect(() => {
    startCallListener()
    return subscribe(setS)
  }, [])
  const live = s.phase === 'active' || s.phase === 'connecting'
  useEffect(() => {
    if (!live) return
    const t = window.setInterval(() => {
      setNow(Date.now())
      setLevel(levels())
    }, 120)
    return () => window.clearInterval(t)
  }, [live])
  useEffect(() => {
    if (s.phase === 'idle' || s.phase === 'ended') setDevicesOpen(false)
  }, [s.phase])
  if (s.phase === 'idle') return null
  const name = nameOf(s.peer)
  const username = usernameOf?.(s.peer) ?? null
  const avatar = avatarOf?.(s.peer) ?? null
  const talking = s.phase === 'active' && level.remote > 0.12
  const reconnecting = s.phase === 'active' && s.reconnecting
  const line =
    s.phase === 'outgoing'
      ? 'Calling...'
      : s.phase === 'incoming'
        ? 'is calling you'
        : s.phase === 'connecting'
          ? 'Connecting...'
          : s.phase === 'active'
            ? s.reconnecting
              ? 'Reconnecting...'
              : clock(now - s.since)
            : s.reason
  const off = 'border-destructive text-destructive hover:border-destructive hover:text-destructive'
  return (
    <div
      role="dialog"
      aria-label={`Call with ${name}`}
      className="kryo-radius kryo-pop fixed bottom-16 right-4 z-[600] grid w-80 max-w-[calc(100vw-2rem)] gap-3 border border-border bg-card p-4 shadow-2xl shadow-black/60"
    >
      <div className="flex items-center gap-3">
        {/* Who is talking: the picture's border lights while they speak. */}
        <ProfileLink username={username} className={cn('kryo-pill grid size-11 shrink-0 place-items-center overflow-hidden border-2 bg-secondary transition-colors duration-100 focus-visible:outline-2 focus-visible:outline-primary', talking ? 'border-success' : 'border-transparent')}>
          {avatar ? (
            <img decoding="async" src={artSrc(avatar) ?? undefined} alt="" className="size-full object-cover" />
          ) : (
            <span className="text-sm font-bold text-foreground">{name.slice(0, 1).toUpperCase()}</span>
          )}
        </ProfileLink>
        <div className="grid min-w-0 grow gap-0.5">
          <ProfileLink username={username} className="truncate text-sm font-bold text-foreground hover:underline focus-visible:outline-2 focus-visible:outline-primary"><TeamName username={username}>{name}</TeamName></ProfileLink>
          <span className={cn('flex items-center gap-1.5 text-xs tabular-nums', reconnecting ? 'text-warning' : 'text-muted-foreground')} aria-live="polite">
            {reconnecting ? <WifiOff className="size-3" aria-hidden /> : null}
            {line}
            {s.peerMuted && live ? <MicOff className="size-3" aria-label={`${name} is muted`} /> : null}
          </span>
        </div>
      </div>

      {s.phase === 'incoming' ? (
        <div className="flex gap-2">
          <Button variant="primary" size="sm" className="grow" onClick={() => void accept()}>
            <Phone className="size-3" aria-hidden /> Answer
          </Button>
          <Button variant="danger" size="sm" className="grow" onClick={decline}>
            <PhoneOff className="size-3" aria-hidden /> Decline
          </Button>
        </div>
      ) : s.phase === 'ended' ? null : (
        <div className="flex items-center gap-1.5">
          <IconButton label={s.muted ? 'Unmute' : 'Mute'} aria-pressed={s.muted} onClick={toggleMute} className={cn(s.muted && off)}>
            {s.muted ? <MicOff className="size-4" /> : <Mic className="size-4" />}
          </IconButton>
          <IconButton label={s.deafened ? 'Hear the call again' : 'Deafen'} aria-pressed={s.deafened} onClick={toggleDeafen} className={cn(s.deafened && off)}>
            {s.deafened ? <HeadphoneOff className="size-4" /> : <Headphones className="size-4" />}
          </IconButton>
          <IconButton label="Microphone and speaker" aria-expanded={devicesOpen} onClick={() => setDevicesOpen((v) => !v)}>
            <Settings2 className="size-4" />
          </IconButton>
          {/* Your microphone's level: proof you are being heard. */}
          <span className="kryo-ascii-art grow overflow-hidden whitespace-pre text-center text-[11px] leading-none text-muted-foreground" aria-hidden>
            {live ? asciiTrack(level.local, 8) : ''}
          </span>
          <Button variant="danger" size="sm" onClick={hangUp}>
            <PhoneOff className="size-3" aria-hidden /> {s.phase === 'outgoing' ? 'Cancel' : 'Hang up'}
          </Button>
        </div>
      )}

      {devicesOpen ? <DevicePicker level={level.local} /> : null}

      <span className="text-[10px] text-muted-foreground">End-to-end encrypted</span>
    </div>
  )
}

function choices(list: AudioDevice[]): { value: string; label: string }[] {
  return [{ value: SYSTEM, label: 'Same as the system' }, ...list.map((x) => ({ value: x.id, label: x.label }))]
}

function chosen(list: AudioDevice[], id: string | null): string {
  return id && list.some((x) => x.id === id) ? id : SYSTEM
}

/** Which microphone and speaker the call uses, with the microphone's level to check it by. */
function DevicePicker({ level }: { level: number }) {
  const [d, setD] = useState<AudioDevices | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    const load = () => void audioDevices().then(setD).catch(() => setD(null))
    load()
    navigator.mediaDevices?.addEventListener?.('devicechange', load)
    return () => navigator.mediaDevices?.removeEventListener?.('devicechange', load)
  }, [])
  if (!d) return null
  const change = (work: Promise<void>) =>
    void work
      .then(() => audioDevices().then(setD))
      .then(() => setError(null))
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
  return (
    <div className="grid gap-2 border-t border-border pt-3">
      <Caption>Microphone</Caption>
      <Dropdown size="sm" label="Microphone" value={chosen(d.inputs, d.input)} options={choices(d.inputs)} onChange={(v) => change(chooseInput(v === SYSTEM ? null : v))} />
      <span className="kryo-ascii-art whitespace-pre text-[11px] leading-none text-muted-foreground" aria-hidden>
        {asciiTrack(level, 28)}
      </span>
      {d.canPickOutput ? (
        <>
          <Caption>Speaker</Caption>
          <Dropdown size="sm" label="Speaker" value={chosen(d.outputs, d.output)} options={choices(d.outputs)} onChange={(v) => change(chooseOutput(v === SYSTEM ? null : v))} />
        </>
      ) : null}
      {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
    </div>
  )
}
