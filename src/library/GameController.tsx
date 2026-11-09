import { useEffect, useRef, useState } from 'react'
import { ExternalLink, RotateCcw, SlidersHorizontal, Trash2, Undo2 } from '@/ui/pixel-icons'
import { Button, Check, ContextMenu, Dropdown, Section, Segmented, type MenuEntry } from '@/ui'
import { PadBadge } from '@/controller/PadArtView'
import { errorText } from '@/lib/bridge'
import { catalogApiUrl } from '@/lib/endpoint'
import { isWindowsHost, type LibraryGame } from '@/lib/library'
import { openExternal } from '@/lib/window'
import { cn } from '@/lib/utils'
import { usePadStore } from '@/hooks/usePads'
import {
  REMAPPABLE,
  VIRTUAL_DRIVER_URL,
  actsAs,
  assign,
  familyOf,
  isIdentity,
  layoutFor,
  padApi,
  padGames,
  padKeys,
  padLabel,
  type GamePadProfile,
  type GamePadView,
  type PadLine,
  type PadMode,
  type PadSeen,
  type PadUse,
  type StickUse,
} from '@/lib/pad'
import type { PadControl } from '@/lib/pad-art'

/** The controls keyboard and mouse can be set on, in the order they are listed. */
const BINDABLE: PadControl[] = [...REMAPPABLE, 'dpup', 'dpdown', 'dpleft', 'dpright']

const STICKS: { value: StickUse; label: string }[] = [
  { value: 'wasd', label: 'W A S D' },
  { value: 'arrows', label: 'Arrow keys' },
  { value: 'steer', label: 'A and D only' },
  { value: 'mouse', label: 'Mouse' },
  { value: 'off', label: 'Nothing' },
]

/**
 * Properties > Controller: what a pad does in this game, saved as it changes.
 * Left alone, played as a controller with a remap of its own, or played as
 * keyboard and mouse for a game with no controller support, from a layout for
 * its kind of game. A game follows the shared default until it is changed.
 * Each connected controller has a row: what the game gets from it, and, where
 * that can really work, taking it out of the game or giving it its own
 * settings, from its buttons or by right-clicking it. It says plainly what
 * cannot reach this game: src-tauri pad_games.rs has why.
 */
export function GameController({ game, onGeneral }: { game: LibraryGame; onGeneral: () => void }) {
  const { pads, config } = usePadStore()
  const [view, setView] = useState<GamePadView | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [suggested, setSuggested] = useState<string | null>(null)
  const [driver, setDriver] = useState<boolean | null>(null)
  const [seen, setSeen] = useState<PadSeen | null>(null)
  /** The controller whose own settings are open below; none is every controller. */
  const [editing, setEditing] = useState<string | null>(null)
  const [menu, setMenu] = useState<{ line: PadLine; x: number; y: number } | null>(null)
  /** What Remove took away, for Undo. */
  const [removed, setRemoved] = useState<GamePadProfile | null>(null)
  const saving = useRef(0)
  const windows = isWindowsHost()
  const shown = view?.profile

  // What the game will see follows the settings and what is plugged in.
  const plugged = pads.map((d) => d.id).join()
  useEffect(() => {
    if (!shown) return
    let cancelled = false
    void padGames
      .seen(shown)
      .then((s) => !cancelled && setSeen(s))
      .catch((e: unknown) => !cancelled && setError(errorText(e)))
    return () => {
      cancelled = true
    }
  }, [shown, plugged, driver])

  useEffect(() => {
    let cancelled = false
    setView(null)
    void padGames
      .get(game.id)
      .then((v) => !cancelled && setView(v))
      .catch((e: unknown) => !cancelled && setError(errorText(e)))
    if (windows) void padApi.virtualDriver().then((d) => !cancelled && setDriver(d)).catch(() => {})
    return () => {
      cancelled = true
    }
  }, [game.id, windows])

  // kryo.to's genres say which layout fits; a game not on kryo.to just has none.
  useEffect(() => {
    if (!game.slug) return
    let cancelled = false
    void catalogApiUrl(`/api/games/${encodeURIComponent(game.slug)}`)
      .then((url) => fetch(url))
      .then((r) => (r.ok ? (r.json() as Promise<{ game?: { genres?: unknown; tags?: unknown } }>) : null))
      .then((b) => {
        const list = (v: unknown) => (Array.isArray(v) ? v.filter((s): s is string => typeof s === 'string') : [])
        if (!cancelled && b?.game) setSuggested(layoutFor([...list(b.game.genres), ...list(b.game.tags)]))
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [game.slug])

  if (error && !view) return <p className="text-xs text-destructive">{error}</p>
  if (!view) return <p className="text-xs text-muted-foreground">Looking at the game&apos;s folder…</p>

  if (view.antiCheat) {
    return (
      <Section title="Controller" hint="What your controller does in this game.">
        <p className="kryo-radius border border-border bg-card px-3 py-2 text-xs leading-relaxed text-muted-foreground">
          This game uses {view.antiCheat}. Kryoto sends it no keyboard or mouse input and plugs in no virtual controller, because anti-cheat can treat those as cheating. A controller the game
          supports by itself still works.
        </p>
      </Section>
    )
  }

  const all = view.profile
  // The controller being edited, while it is plugged in and has settings of its own.
  const target = editing && all.pads[editing]?.own && seen?.pads.some((l) => l.key === editing) ? editing : null
  const p = (target && all.pads[target]?.own) || all
  const keys = padKeys(pads)
  const pad = pads[target ? keys.indexOf(target) : 0] ?? pads[0] ?? null
  const family = familyOf(pad, pad ? config?.pads[pad.guid] : null)
  const targetName = seen?.pads.find((l) => l.key === target)?.name

  /** Save, and show it failing: the page goes back to what is really saved. */
  const save = (profile: GamePadProfile | null) => {
    const before = view
    const ticket = ++saving.current
    setError(null)
    setNote(null)
    setRemoved(null)
    setView({ ...view, profile: profile ?? view.default, own: profile !== null })
    padGames.set(game.id, profile).catch((e: unknown) => {
      if (ticket !== saving.current) return
      setView(before)
      setError(`Not saved: ${errorText(e)}`)
    })
  }
  /** A change to what is open: one controller's own settings, or everyone's. */
  const change = (patch: Partial<GamePadProfile>) => {
    const own = target ? all.pads[target]?.own : null
    if (target && own) save({ ...all, pads: { ...all.pads, [target]: { off: false, own: { ...own, ...patch } } } })
    else save({ ...all, ...patch })
  }

  const layout = (id: string) => {
    const l = view.layouts[id]
    if (l) change({ template: l.template, binds: l.binds, leftStick: l.leftStick, rightStick: l.rightStick, sensitivity: l.sensitivity, deadzone: l.deadzone, invertY: l.invertY })
  }

  const setUse = (key: string, use: PadUse | null) => {
    const next = { ...all.pads }
    if (use) next[key] = use
    else delete next[key]
    save({ ...all, pads: next })
  }
  const takeOut = (key: string) => {
    if (editing === key) setEditing(null)
    setUse(key, { off: true, own: null })
  }
  const useOwn = (key: string) => {
    setEditing(key)
    if (!all.pads[key]?.own) setUse(key, { off: false, own: { ...all, pads: {} } })
  }
  const useShared = (key: string) => {
    if (editing === key) setEditing(null)
    setUse(key, null)
  }

  /** The whole setup goes: the game follows the shared default, as if never set. */
  const removeSetup = () => {
    const before = all
    setEditing(null)
    save(null)
    setRemoved(before)
  }

  const padMenu = (line: PadLine): MenuEntry[] => {
    const items: MenuEntry[] = []
    if (line.removed) items.push({ label: 'Add back to this game', icon: <Undo2 />, onSelect: () => useShared(line.key) })
    else if (line.canRemove) items.push({ label: 'Remove from this game', icon: <Trash2 />, danger: true, onSelect: () => takeOut(line.key) })
    if (line.canOwn) {
      items.push({ label: line.own ? 'Edit its settings' : 'Give it its own settings', icon: <SlidersHorizontal />, onSelect: () => useOwn(line.key) })
      if (line.own) items.push({ label: 'Use the settings below instead', icon: <Undo2 />, onSelect: () => useShared(line.key) })
    }
    if (!items.length) items.push({ label: 'Nothing can change for this one here', disabled: true, onSelect: () => {} })
    if (view.own) items.push({ separator: true }, { label: 'Remove controller setup', icon: <Trash2 />, danger: true, onSelect: removeSetup })
    return items
  }

  const makeDefault = () => {
    setError(null)
    padGames
      .setDefault(all)
      .then(() => {
        setView({ ...view, default: all })
        setNote('Games without their own settings now use these.')
      })
      .catch((e: unknown) => setError(`Not saved: ${errorText(e)}`))
  }

  const keyOptions = [{ value: '', label: 'Nothing' }, ...view.keys.map((k) => ({ value: k.name, label: k.label }))]
  const templateOptions = view.templates.map((t) => ({ value: t.id, label: t.id === suggested ? `${t.label} (suggested)` : t.label, hint: t.hint }))

  return (
    <div className="grid gap-6">
      <Section title="Controller" hint="What your controller does while this game runs. Changes save straight away.">
        <div className="grid gap-3">
          <Check checked={!view.own} onChange={(follow) => save(follow ? null : all)} label="Use the shared default" />
          <Segmented<PadMode>
            label="What the controller does"
            value={all.mode}
            options={[
              { value: 'off', label: 'Off' },
              { value: 'gamepad', label: 'Controller' },
              { value: 'keyboard', label: 'Keyboard and mouse' },
            ]}
            onChange={(mode) => save({ ...all, mode })}
          />
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            {all.mode === 'off'
              ? 'Kryoto leaves this game alone: no remap, no virtual controller, no keys. The game still reads a controller by itself if it can.'
              : all.mode === 'gamepad'
                ? 'For games with controller support. The game gets your controller as you set it up.'
                : 'For games with no controller support. Kryoto presses keys and moves the mouse for you.'}
          </p>
          {view.own ? (
            <Button size="sm" variant="ghost" className="justify-self-start" onClick={removeSetup}>
              <Trash2 className="size-3.5" />
              Remove controller setup
            </Button>
          ) : null}
          {removed ? (
            <p className="flex flex-wrap items-center gap-x-2 text-[11px] leading-relaxed text-muted-foreground">
              Setup removed: this game follows the shared default again.
              <Button size="sm" variant="ghost" onClick={() => save(removed)}>
                <Undo2 className="size-3" />
                Undo
              </Button>
            </p>
          ) : null}
        </div>
      </Section>

      {seen ? (
        <Section title="What the game will see" hint={seen.summary}>
          <div className="grid gap-2">
            {seen.pads.map((line) => (
              <div
                key={line.key}
                onContextMenu={(e) => {
                  e.preventDefault()
                  setMenu({ line, x: e.clientX, y: e.clientY })
                }}
                className={cn('kryo-radius grid gap-1 border p-3', line.key === target ? 'border-foreground' : 'border-border')}
              >
                <div className="flex flex-wrap items-center gap-2">
                  <span className={cn('min-w-0 grow truncate text-xs font-bold', line.removed ? 'text-muted-foreground' : 'text-foreground')}>{line.name}</span>
                  {line.player ? <span className="text-[10px] uppercase tracking-wider text-muted-foreground">Player {line.player}</span> : null}
                  {line.canOwn && !line.removed ? (
                    <Button size="sm" variant="ghost" onClick={() => (line.key === target ? setEditing(null) : useOwn(line.key))}>
                      <SlidersHorizontal className="size-3" />
                      {line.key === target ? 'Done' : line.own ? 'Edit its settings' : 'Own settings'}
                    </Button>
                  ) : null}
                  {line.removed ? (
                    <Button size="sm" variant="ghost" onClick={() => useShared(line.key)}>
                      <Undo2 className="size-3" />
                      Add back
                    </Button>
                  ) : line.canRemove ? (
                    <Button size="sm" variant="ghost" onClick={() => takeOut(line.key)}>
                      <Trash2 className="size-3" />
                      Remove
                    </Button>
                  ) : null}
                </div>
                <p className="text-[11px] leading-relaxed text-muted-foreground">{line.sees}</p>
              </div>
            ))}
          </div>
        </Section>
      ) : null}
      {menu ? <ContextMenu x={menu.x} y={menu.y} items={padMenu(menu.line)} onClose={() => setMenu(null)} /> : null}

      {target ? (
        <div className="flex flex-wrap items-center gap-2">
          <p className="min-w-0 grow text-xs font-bold text-foreground">Settings for {targetName} only</p>
          <Button size="sm" variant="ghost" onClick={() => useShared(target)}>
            <Undo2 className="size-3" />
            Use the settings for every controller
          </Button>
        </div>
      ) : null}

      {all.mode === 'gamepad' ? (
        <Section
          title={target ? 'Its buttons in this game' : 'Buttons in this game'}
          hint={
            windows
              ? 'Give a button another job and the button that had it takes over. It reaches the game through the virtual Xbox controller. With none changed, the controller keeps its own remap.'
              : 'Give a button another job and the button that had it takes over. SDL games, Wine and Proton follow this; a game that reads the controller another way will not. It applies from the next start.'
          }
        >
          <div className="grid gap-3">
            {seen && seen.pads.length > 0 && !seen.pads.some((l) => l.canOwn) ? (
              <p className="kryo-radius border border-border bg-card px-3 py-2 text-[11px] leading-relaxed text-muted-foreground">
                A remap reaches none of the controllers connected now. Keyboard and mouse works for any of them.
              </p>
            ) : null}
            {windows && driver === false ? (
              <p className="kryo-radius flex flex-wrap items-center gap-2 border border-border bg-card px-3 py-2 text-[11px] leading-relaxed text-muted-foreground">
                <span className="min-w-0 grow">Remaps need the free ViGEmBus driver, which is optional. Without it games read the controller as it is.</span>
                <Button size="sm" variant="ghost" onClick={() => void openExternal(VIRTUAL_DRIVER_URL)}>
                  <ExternalLink className="size-3" />
                  Get ViGEmBus
                </Button>
              </p>
            ) : null}
            <div className="grid gap-x-6 gap-y-1 sm:grid-cols-2">
              {REMAPPABLE.map((c) => (
                <div key={c} className="flex items-center gap-3 px-2 py-1">
                  <PadBadge family={family} control={c} label={padLabel(family, c).long} />
                  <span className="min-w-0 grow truncate text-xs text-foreground">{padLabel(family, c).long}</span>
                  <Dropdown<PadControl>
                    size="sm"
                    className="w-32 shrink-0"
                    label={`${padLabel(family, c).long} acts as`}
                    value={actsAs(p.remap, c)}
                    emphasis={actsAs(p.remap, c) !== c}
                    onChange={(v) => change({ remap: assign(p.remap, c, v) })}
                    options={REMAPPABLE.map((j) => ({ value: j, label: j === c ? `${padLabel(family, j).short} (own)` : `as ${padLabel(family, j).short}` }))}
                  />
                </div>
              ))}
            </div>
            {!isIdentity(p.remap) ? (
              <Button size="sm" variant="ghost" className="justify-self-start" onClick={() => change({ remap: {} })}>
                <RotateCcw className="size-3.5" />
                Reset all buttons
              </Button>
            ) : null}
          </div>
        </Section>
      ) : null}

      {all.mode === 'keyboard' ? (
        <>
          <Section title="Layout" hint={view.templates.find((t) => t.id === p.template)?.hint ?? 'A starting point for this kind of game; change anything below.'}>
            <div className="grid gap-3">
              {view.outputProblem ? <p className="text-[11px] leading-relaxed text-destructive">{view.outputProblem}</p> : null}
              {view.runAsAdmin ? (
                <p className="flex flex-wrap items-center gap-x-2 text-[11px] leading-relaxed text-warning">
                  This game runs as administrator, and Windows stops Kryoto sending input to it. Turn that off in General.
                  <Button size="sm" variant="ghost" onClick={onGeneral}>
                    Open General
                  </Button>
                </p>
              ) : null}
              <Dropdown label="Layout" value={p.template} onChange={layout} options={templateOptions} />
              {!windows ? (
                <p className="text-[11px] leading-relaxed text-muted-foreground">
                  On Linux Kryoto cannot tell which window is in front, so input goes out while the game runs and Kryoto is not in front.
                </p>
              ) : null}
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                Hold {padLabel(family, 'back').short} and {padLabel(family, 'start').short} together to pause it, and again to start it. The controller buzzes each time.
              </p>
            </div>
          </Section>
          <Section title="Sticks">
            <div className="grid gap-3">
              <div className="grid gap-3 sm:grid-cols-2">
                <StickRow title="Left stick" value={p.leftStick} onChange={(leftStick) => change({ leftStick })} />
                <StickRow title="Right stick" value={p.rightStick} onChange={(rightStick) => change({ rightStick })} />
              </div>
              {p.leftStick === 'mouse' || p.rightStick === 'mouse' ? (
                <>
                  <Slider label="Mouse speed" min={100} max={4000} step={100} value={p.sensitivity} onChange={(sensitivity) => change({ sensitivity })} />
                  <Check checked={p.invertY} onChange={(invertY) => change({ invertY })} label="Invert up and down" />
                </>
              ) : null}
              <Slider label="Dead zone" min={2} max={60} step={1} value={Math.round(p.deadzone * 100)} onChange={(v) => change({ deadzone: v / 100 })} />
            </div>
          </Section>
          <Section title="Buttons">
            <div className="grid gap-x-6 gap-y-1 sm:grid-cols-2">
              {BINDABLE.map((c) => (
                <div key={c} className="flex items-center gap-3 px-2 py-1">
                  <PadBadge family={family} control={c} label={padLabel(family, c).long} />
                  <span className="min-w-0 grow truncate text-xs text-foreground">{padLabel(family, c).long}</span>
                  <Dropdown
                    size="sm"
                    className="w-32 shrink-0"
                    label={`${padLabel(family, c).long} presses`}
                    value={p.binds[c] ?? ''}
                    onChange={(v) => {
                      const binds = { ...p.binds }
                      if (v) binds[c] = v
                      else delete binds[c]
                      change({ binds })
                    }}
                    options={keyOptions}
                  />
                </div>
              ))}
            </div>
          </Section>
        </>
      ) : null}

      <div className="flex flex-wrap items-center gap-3">
        <Button size="sm" variant="ghost" onClick={makeDefault}>
          Use these for every game without its own
        </Button>
        {note ? <span className="text-[11px] text-muted-foreground">{note}</span> : null}
        {error ? <span className="text-[11px] text-destructive">{error}</span> : null}
      </div>
    </div>
  )
}

function StickRow({ title, value, onChange }: { title: string; value: StickUse; onChange: (v: StickUse) => void }) {
  return (
    <div className="flex items-center gap-3">
      <span className="min-w-0 grow text-xs text-foreground">{title}</span>
      <Dropdown<StickUse> size="sm" className="w-32 shrink-0" label={title} value={value} onChange={onChange} options={STICKS} />
    </div>
  )
}

function Slider({ label, min, max, step, value, onChange }: { label: string; min: number; max: number; step: number; value: number; onChange: (v: number) => void }) {
  // Saved when let go, not on every step of the drag.
  const [draft, setDraft] = useState(value)
  useEffect(() => setDraft(value), [value])
  return (
    <label className="flex items-center gap-4 text-xs text-muted-foreground">
      <span className="w-24 shrink-0">{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={draft}
        aria-label={label}
        className="grow"
        onChange={(e) => setDraft(Number(e.target.value))}
        onPointerUp={() => onChange(draft)}
        onKeyUp={() => onChange(draft)}
      />
      <span className="w-10 text-right">{draft}</span>
    </label>
  )
}
