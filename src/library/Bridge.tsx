import { useCallback, useEffect, useMemo, useState } from 'react'
import { Download, FolderOpen, Undo2, Upload } from '@/ui/pixel-icons'
import { Button, Check, Dropdown, IconButton, inputCls, Matrix, Modal } from '@/ui'
import { PixelSplash } from '@/ui/pixel-icons-motion'
import { SceneMotion } from '@/ui/ascii/SceneMotion'
import { SHELF_MOVE, SHELF_SEND } from '@/ui/ascii/motion'
import { errorText } from '@/lib/bridge'
import { pickPath } from '@/lib/pick'
import { artSrc } from '@/lib/art'
import { adultBlur, useShowAdult } from '@/lib/adult'
import { formatLastPlayed, formatPlaytime } from '@/lib/format'
import { bridge, type BridgeImported, type BridgeRow, type BridgeScan, type ExportList, type Launcher, type Sent } from '@/lib/launchers'
import { cn } from '@/lib/utils'

/**
 * Bridge: Kryoto and the player's other launchers, on one screen, one step at
 * a time like the phone and controller setups. Pick a launcher and a way, look
 * at every game and what happens to it, then go.
 *
 * Nothing is written before the player presses the last button. A launcher's
 * files are only read, unless games are sent out to it (src-tauri bridge/).
 */

type Phase = 'pick' | 'reading' | 'review' | 'working' | 'done'
type Way = 'in' | 'out'

/** A row's answer: a kryo.to game's slug, `own`, `skip`, or '' while unanswered. */
type Pick = string

const HOW: Record<NonNullable<BridgeRow['how']>, string> = {
  steamAppid: 'Same Steam game',
  executable: 'Same game file',
  folder: 'Same game folder',
  title: 'Same name',
}

function defaultPick(r: BridgeRow): Pick {
  if (r.kind === 'matched') return r.options[0]?.slug ?? 'own'
  if (r.kind === 'own') return 'own'
  // Already in the library under that name: not twice, unless the player says.
  if (r.kind === 'stays') return r.joins ? 'skip' : 'own'
  return ''
}

const count = (n: number, one = 'game', many = 'games') => `${n} ${n === 1 ? one : many}`

export function Bridge({ onClose }: { onClose: () => void }) {
  const showAdult = useShowAdult()
  const [phase, setPhase] = useState<Phase>('pick')
  const [launchers, setLaunchers] = useState<Launcher[] | null>(null)
  /** Folders the player chose, by launcher. */
  const [dirs, setDirs] = useState<Record<string, string>>({})
  const [at, setAt] = useState<{ id: string; way: Way } | null>(null)
  const [scan, setScan] = useState<BridgeScan | null>(null)
  const [picks, setPicks] = useState<Record<string, Pick>>({})
  const [out, setOut] = useState<ExportList | null>(null)
  const [user, setUser] = useState('')
  const [chosen, setChosen] = useState<Set<string>>(new Set())
  const [imported, setImported] = useState<BridgeImported | null>(null)
  const [sent, setSent] = useState<Sent | null>(null)
  const [undone, setUndone] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)

  const look = useCallback(async () => {
    try {
      const all = await bridge.launchers()
      // A folder the player chose answers for that launcher.
      const picked = await Promise.all(all.map((l) => (dirs[l.id] ? bridge.detect(l.id, dirs[l.id]!) : l)))
      setLaunchers(picked)
    } catch (e) {
      setError(errorText(e))
    }
  }, [dirs])
  useEffect(() => void look(), [look])

  const launcher = launchers?.find((l) => l.id === at?.id) ?? null
  const name = launcher?.name ?? ''
  const dir = launcher?.dir ?? null
  const way = at?.way ?? 'in'

  // While a launcher that has to close is open, keep looking: things move on
  // by themselves once it has.
  const waiting = phase === 'pick' ? !!launchers?.some((l) => l.running) : phase === 'review' && way === 'out'
  useEffect(() => {
    if (!waiting) return
    const t = window.setInterval(() => void look(), 2000)
    return () => window.clearInterval(t)
  }, [waiting, look])

  const start = async (l: Launcher, next: Way) => {
    setAt({ id: l.id, way: next })
    setPhase('reading')
    setError(null)
    setUndone(null)
    try {
      if (next === 'in') {
        const s = await bridge.scan(l.id, l.dir)
        if (s.running) {
          void look()
          return setPhase('pick')
        }
        setScan(s)
        setPicks(Object.fromEntries(s.rows.map((r) => [r.key, defaultPick(r)])))
      } else {
        const list = await bridge.exportList(l.id, l.dir)
        setOut(list)
        setUser(list.users[0]?.id ?? '')
        setChosen(new Set(list.games.filter((g) => !g.sent).map((g) => g.id)))
      }
      setPhase('review')
    } catch (e) {
      setError(errorText(e))
      setPhase('pick')
    }
  }

  const choose = async (l: Launcher) => {
    const picked = await pickPath({ title: `${l.name}'s folder`, directory: true })
    if (!picked) return
    setError(null)
    try {
      const found = await bridge.detect(l.id, picked)
      if (!found.dir) return setError(`That is not ${l.name}'s folder.`)
      setDirs((d) => ({ ...d, [l.id]: picked }))
    } catch (e) {
      setError(errorText(e))
    }
  }

  const rows = useMemo(() => scan?.rows ?? [], [scan])
  const open = rows.filter((r) => r.kind === 'matched' || r.kind === 'own' || r.kind === 'ask' || r.kind === 'stays')
  const unanswered = open.filter((r) => picks[r.key] === '').length
  const coming = open.filter((r) => picks[r.key] && picks[r.key] !== 'skip')
  const hours = coming.reduce((s, r) => s + r.playtimeSeconds, 0)

  /** The scene gets a moment even for a short list, so the move is seen. */
  const withScene = async <T,>(work: Promise<T>): Promise<PromiseSettledResult<T>> => {
    const [result] = await Promise.allSettled([work, new Promise((r) => setTimeout(r, 2400))])
    return result
  }

  const bring = async () => {
    if (!at) return
    setPhase('working')
    setError(null)
    const decisions = coming.map((r) => ({ key: r.key, slug: picks[r.key] === 'own' ? null : picks[r.key]! }))
    const result = await withScene(bridge.import(at.id, scan?.dir ?? dir, decisions))
    if (result.status === 'fulfilled') {
      setImported(result.value)
      setPhase('done')
    } else {
      setError(errorText(result.reason))
      setPhase('review')
    }
  }

  const send = async () => {
    if (!at) return
    setPhase('working')
    setError(null)
    const result = await withScene(bridge.export(at.id, dir, user, [...chosen], showAdult))
    if (result.status === 'fulfilled') {
      setSent(result.value)
      setPhase('done')
    } else {
      setError(errorText(result.reason))
      setPhase('review')
    }
  }

  const takeBack = async () => {
    if (!at) return
    setError(null)
    try {
      const n = await bridge.undo(at.id, dir)
      setUndone(n)
      setSent(null)
      setOut(await bridge.exportList(at.id, dir))
      setPhase('done')
    } catch (e) {
      setError(errorText(e))
    }
  }

  const back = () => {
    setPhase('pick')
    setError(null)
    void look()
  }

  const steps = ['Pick a launcher', 'Check the list', way === 'out' ? 'Send them' : 'Bring them in']
  const step = phase === 'pick' || phase === 'reading' ? 0 : phase === 'review' ? 1 : phase === 'working' ? 2 : 3
  const closed = !launcher?.running

  const footer =
    phase === 'review' ? (
      <div className="flex w-full flex-wrap items-center justify-between gap-3">
        <span className="min-w-0 text-[11px] text-muted-foreground">
          {way === 'in'
            ? unanswered
              ? `Answer ${count(unanswered)} more first.`
              : `Nothing changes until you press the button. ${name}'s files are only read.`
            : closed
              ? `A copy of ${name}'s list is kept first, and you can undo this.`
              : `Close ${name} first, from the tray too: it would write over the change when it closes. This moves on by itself.`}
        </span>
        <div className="flex gap-2">
          <Button variant="ghost" onClick={back}>
            Back
          </Button>
          {way === 'in' ? (
            <Button variant="primary" disabled={!!unanswered || coming.length === 0} onClick={() => bring()}>
              {`Bring ${count(coming.length)} in`}
            </Button>
          ) : (
            <Button variant="primary" disabled={!closed || !user || chosen.size === 0} onClick={() => send()}>
              {`Send ${count(chosen.size)} to ${name}`}
            </Button>
          )}
        </div>
      </div>
    ) : phase === 'done' ? (
      <div className="flex w-full justify-end gap-2">
        {sent ? (
          <Button variant="ghost" onClick={() => takeBack()}>
            <Undo2 className="size-3.5" />
            Undo
          </Button>
        ) : null}
        <Button variant="ghost" onClick={back}>
          Another launcher
        </Button>
        <Button variant="primary" onClick={onClose}>
          Done
        </Button>
      </div>
    ) : undefined

  return (
    <Modal title="Bridge" onClose={onClose} wide fill footer={footer}>
      <div className="grid h-full min-h-0 grid-rows-[auto_1fr] gap-4">
        <Progress steps={steps} step={step} />
        {phase === 'pick' ? (
          <div className="grid min-h-0 content-center justify-items-center gap-4 overflow-y-auto">
            <Words title="One library for all your launchers" text="Bring games in from another launcher, or send your Kryoto games out to one. You see the whole list before anything changes." />
            {!launchers ? <Matrix state="busy" className="size-16 text-muted-foreground" announce /> : null}
            <ul className="grid w-full max-w-xl gap-1.5">
              {launchers?.map((l) => (
                <LauncherRow key={l.id} launcher={l} onGo={(next) => void start(l, next)} onChoose={() => void choose(l)} />
              ))}
            </ul>
            {error ? <p className="text-xs text-destructive">{error}</p> : null}
          </div>
        ) : phase === 'review' && way === 'in' && scan ? (
          <ReviewIn scan={scan} open={open} rows={rows} picks={picks} setPicks={setPicks} name={name} keeps={!!launcher?.keeps} showAdult={showAdult} error={error} onRetry={launcher ? () => void start(launcher, 'in') : undefined} />
        ) : phase === 'review' && way === 'out' && out ? (
          <ReviewOut
            list={out}
            name={name}
            user={user}
            setUser={setUser}
            chosen={chosen}
            setChosen={setChosen}
            showAdult={showAdult}
            error={error}
            canUndo={closed}
            onUndo={() => void takeBack()}
          />
        ) : (
          <div className="grid content-center justify-items-center gap-4 text-center" aria-live="polite">
            <Stage phase={phase} way={way} name={name} keeps={!!launcher?.keeps} imported={imported} sent={sent} undone={undone} hours={hours} error={error} />
          </div>
        )}
      </div>
    </Modal>
  )
}

/** A launcher: how it stands, and what can be done with it. */
function LauncherRow({ launcher: l, onGo, onChoose }: { launcher: Launcher; onGo: (way: Way) => void; onChoose: () => void }) {
  const closeFirst = l.running && !l.keeps
  return (
    <li className="kryo-radius flex items-center gap-3 border border-border p-3">
      <span className="grid min-w-0 grow gap-0.5">
        <span className="text-sm font-bold text-foreground">{l.name}</span>
        <span className="truncate text-[11px] text-muted-foreground" title={l.dir ?? undefined}>
          {!l.dir ? 'Not on this PC' : closeFirst ? `Close ${l.name} first, from the tray too. This moves on by itself.` : l.dir}
        </span>
      </span>
      <IconButton label={`Choose ${l.name}'s folder`} onClick={onChoose}>
        <FolderOpen className="size-4" />
      </IconButton>
      {l.exports ? (
        <Button size="sm" disabled={!l.dir} onClick={() => onGo('out')}>
          <Upload className="size-3" />
          Send out
        </Button>
      ) : null}
      <Button size="sm" variant="primary" disabled={!l.dir || closeFirst} onClick={() => onGo('in')}>
        <Download className="size-3" />
        Bring in
      </Button>
    </li>
  )
}

function ReviewIn({
  scan,
  open,
  rows,
  picks,
  setPicks,
  name,
  keeps,
  showAdult,
  error,
  onRetry,
}: {
  scan: BridgeScan
  open: BridgeRow[]
  rows: BridgeRow[]
  picks: Record<string, Pick>
  setPicks: React.Dispatch<React.SetStateAction<Record<string, Pick>>>
  name: string
  keeps: boolean
  showAdult: boolean
  error: string | null
  onRetry?: () => void
}) {
  const groups = [
    { kind: 'ask', title: 'Ask me', hint: 'Kryoto is not sure which game these are. Pick one, or keep it as your own.' },
    { kind: 'matched', title: 'On Kryoto', hint: "These become Kryoto games, with Kryoto's art and details. Your files stay where they are." },
    { kind: 'own', title: 'Your own games', hint: `Kryoto does not list these. They keep ${name}'s name, pictures and play time, and stay on this PC.` },
    { kind: 'stays', title: `These stay in ${name}`, hint: `They show in your Library and Play starts them through ${name}. Nothing about them goes to kryo.to.` },
  ] as const
  const missing = rows.filter((r) => r.kind === 'missing')
  const before = rows.filter((r) => r.kind === 'imported')
  const setAll = (list: BridgeRow[], pick: Pick) => setPicks((cur) => ({ ...cur, ...Object.fromEntries(list.map((r) => [r.key, pick])) }))
  const [find, setFind] = useState('')
  return (
    <div className="grid min-h-0 content-start gap-5 overflow-y-auto pr-1" aria-label="What each game becomes">
      {open.length > SEARCH_FROM ? <FindGame value={find} onChange={setFind} total={open.length} /> : null}
      {!scan.catalog ? (
        <div className="kryo-radius grid gap-2 border border-warning/40 bg-warning/10 p-3 text-xs text-warning">
          <p>Kryoto could not read its game list just now, so these come over as your own games. When Kryoto finds one of them later, it asks you once whether to link it.</p>
          {onRetry ? (
            <Button size="sm" className="w-fit" onClick={onRetry}>
              Try again
            </Button>
          ) : null}
        </div>
      ) : null}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
      {open.length === 0 ? <p className="text-xs text-muted-foreground">Nothing new to bring in.</p> : null}
      {groups.map((g) => {
        const list = open.filter((r) => r.kind === g.kind && named(r.title, find))
        if (!list.length) return null
        return (
          <section key={g.kind} className="grid gap-2">
            <header className="flex items-end justify-between gap-3">
              <span className="grid gap-0.5">
                <h3 className="text-xs font-bold uppercase tracking-wider text-foreground">
                  {g.title} <span className="text-muted-foreground">{list.length}</span>
                </h3>
                <p className="text-[11px] text-muted-foreground">{g.hint}</p>
              </span>
              {g.kind === 'stays' ? (
                <span className="flex shrink-0 gap-1">
                  <Button size="sm" variant="ghost" onClick={() => setAll(list, 'own')}>
                    All
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => setAll(list, 'skip')}>
                    None
                  </Button>
                </span>
              ) : null}
            </header>
            <ul className="grid gap-1">
              {list.map((r) => (
                <GameRow key={r.key} row={r} pick={picks[r.key] ?? ''} name={name} showAdult={showAdult} onPick={(p) => setPicks((cur) => ({ ...cur, [r.key]: p }))} />
              ))}
            </ul>
          </section>
        )
      })}
      {missing.length || before.length ? (
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          {missing.length ? `Not on this PC, so left in ${name}: ${missing.map((r) => r.title).join(', ')}.` : ''}
          {missing.length && before.length ? ' ' : ''}
          {before.length ? `${keeps ? 'In your Library already' : 'Brought over before'}: ${before.map((r) => r.title).join(', ')}.` : ''}
        </p>
      ) : null}
    </div>
  )
}

/** A long library gets a box to find one game in. */
const SEARCH_FROM = 12
const named = (title: string, find: string) => title.toLowerCase().includes(find.trim().toLowerCase())
function FindGame({ value, onChange, total }: { value: string; onChange: (v: string) => void; total: number }) {
  return <input value={value} onChange={(e) => onChange(e.target.value)} placeholder={`Search ${total} games`} aria-label="Search these games" className={inputCls} />
}

function ReviewOut({
  list,
  name,
  user,
  setUser,
  chosen,
  setChosen,
  showAdult,
  error,
  canUndo,
  onUndo,
}: {
  list: ExportList
  name: string
  user: string
  setUser: (id: string) => void
  chosen: Set<string>
  setChosen: (next: Set<string>) => void
  showAdult: boolean
  error: string | null
  canUndo: boolean
  onUndo: () => void
}) {
  const [find, setFind] = useState('')
  const toggle = (id: string, on: boolean) => {
    const next = new Set(chosen)
    if (on) next.add(id)
    else next.delete(id)
    setChosen(next)
  }
  const hidden = !showAdult && list.games.some((g) => g.nsfw && chosen.has(g.id))
  return (
    <div className="grid min-h-0 content-start gap-4 overflow-y-auto pr-1" aria-label={`Games to send to ${name}`}>
      {list.note ? <p className="kryo-radius border border-warning/40 bg-warning/10 p-3 text-xs text-warning">{list.note}</p> : null}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
      {list.users.length > 1 ? (
        <label className="flex items-center gap-3 text-xs text-muted-foreground">
          {`${name} account`}
          <Dropdown size="sm" label={`${name} account`} value={user} options={list.users.map((u) => ({ value: u.id, label: u.name }))} onChange={setUser} className="w-56" />
        </label>
      ) : null}
      <header className="flex items-end justify-between gap-3">
        <span className="grid gap-0.5">
          <h3 className="text-xs font-bold uppercase tracking-wider text-foreground">
            Your Kryoto games <span className="text-muted-foreground">{list.games.length}</span>
          </h3>
          <p className="text-[11px] text-muted-foreground">{`Each one is added to ${name} with its pictures, and starts through Kryoto, so your play time and saves keep working.`}</p>
        </span>
        <span className="flex shrink-0 gap-1">
          <Button size="sm" variant="ghost" onClick={() => setChosen(new Set(list.games.map((g) => g.id)))}>
            All
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setChosen(new Set())}>
            None
          </Button>
        </span>
      </header>
      {list.games.length === 0 ? <p className="text-xs text-muted-foreground">No games to send yet. Games from {name} itself stay out of this list.</p> : null}
      {list.games.length > SEARCH_FROM ? <FindGame value={find} onChange={setFind} total={list.games.length} /> : null}
      <ul className="grid gap-1">
        {list.games.filter((g) => named(g.title, find)).map((g) => (
          <li key={g.id} className={cn('kryo-radius flex items-center gap-3 border border-border p-2', !chosen.has(g.id) && 'opacity-60')}>
            <Cover url={g.cover} blur={adultBlur(g.nsfw, showAdult)} />
            <span className="min-w-0 grow">
              <Check checked={chosen.has(g.id)} onChange={(on) => toggle(g.id, on)} label={g.sent ? `${g.title} (in ${name} already)` : g.title} />
            </span>
          </li>
        ))}
      </ul>
      {hidden ? <p className="text-[11px] text-muted-foreground">Adult games go without their pictures, because you hide adult art.</p> : null}
      {list.undoable ? (
        <Button size="sm" variant="ghost" className="w-fit" disabled={!canUndo} onClick={onUndo}>
          <Undo2 className="size-3" />
          {`Take back the ${count(list.undoable)} sent before`}
        </Button>
      ) : null}
    </div>
  )
}

/** The big picture and words for the steps with no list. */
function Stage({
  phase,
  way,
  name,
  keeps,
  imported,
  sent,
  undone,
  hours,
  error,
}: {
  phase: Phase
  way: Way
  name: string
  keeps: boolean
  imported: BridgeImported | null
  sent: Sent | null
  undone: number | null
  hours: number
  error: string | null
}) {
  if (phase === 'reading') {
    return (
      <>
        <Matrix state="scan" className="size-20 text-muted-foreground" announce />
        <Words
          title={way === 'out' ? 'Getting your games ready' : `Reading ${name}'s library`}
          text={keeps || way === 'out' ? 'On this PC only.' : "Matching each game with Kryoto's list, on this PC."}
        />
      </>
    )
  }
  if (phase === 'working') {
    return (
      <>
        <SceneMotion scene={way === 'in' && !keeps ? SHELF_MOVE : SHELF_SEND} className="h-24 text-foreground" label={way === 'out' ? `Games moving from Kryoto's shelf to ${name}'s` : `Games moving from ${name}'s shelf to Kryoto's`} />
        <Words
          title={way === 'out' ? `Sending them to ${name}` : 'Bringing them in'}
          text={way === 'out' ? 'Copying their pictures too.' : hours ? `${formatPlaytime(hours)} of play time comes with them.` : 'Adding them to your library.'}
        />
      </>
    )
  }
  if (undone !== null) {
    return (
      <>
        <PixelSplash icon="Check" className="size-24 text-success" label="Done" />
        <Words title="Taken back" text={`${count(undone)} removed from ${name}, with the pictures Kryoto added. Everything else there is as it was.`} />
      </>
    )
  }
  if (sent) {
    const parts = [sent.added ? `${sent.added} added` : null, sent.updated ? `${sent.updated} updated` : null, sent.pictures ? count(sent.pictures, 'picture', 'pictures') : null].filter(Boolean)
    return (
      <>
        <PixelSplash icon="Check" className="size-24 text-success" label="Done" />
        <Words title={`Your games are in ${name}`} text={`${parts.join(', ')}. Start ${name} to see them. Play there opens Kryoto, which starts the game.`} />
        {sent.backup ? (
          <span className="max-w-md truncate text-[11px] text-muted-foreground" title={sent.backup}>
            {`The list from before: ${sent.backup}`}
          </span>
        ) : null}
        {error ? <p className="text-xs text-destructive">{error}</p> : null}
      </>
    )
  }
  if (imported && keeps) {
    return (
      <>
        <PixelSplash icon="Check" className="size-24 text-success" label="Done" />
        <Words title="Your games are here" text={`${count(imported.own)} from ${name} in your Library. Play starts them through ${name}, which keeps their hours and saves. They stay on this PC.`} />
      </>
    )
  }
  if (imported) {
    const parts = [
      imported.linked ? `${imported.linked} now Kryoto ${imported.linked === 1 ? 'game' : 'games'}` : null,
      imported.own ? `${imported.own} of your own` : null,
      imported.joined ? `${imported.joined} joined ${imported.joined === 1 ? 'a game' : 'games'} already here` : null,
    ].filter(Boolean)
    return (
      <>
        <PixelSplash icon="Check" className="size-24 text-success" label="Done" />
        <Words
          title="Your games are here"
          text={`${parts.join(', ')}.${imported.seconds ? ` ${formatPlaytime(imported.seconds)} of play time came along.` : ''} ${
            imported.saves ? `Their saves are being kept as versions now, ${name}'s own backup too.` : 'Turn on save versions in Settings > Cloud to keep their saves.'
          } Your own games stay on this PC: if Kryoto adds one later, it asks you once whether to link it.`}
        />
      </>
    )
  }
  return <Matrix state="busy" className="size-16 text-muted-foreground" announce />
}

function Words({ title, text }: { title: string; text: string }) {
  return (
    <div className="grid justify-items-center gap-1.5 text-center">
      <p className="text-lg font-bold text-foreground">{title}</p>
      <p className="max-w-md text-xs leading-relaxed text-muted-foreground">{text}</p>
    </div>
  )
}

function Cover({ url, blur }: { url: string | null; blur?: string }) {
  const shape = { borderRadius: 'min(var(--kryo-radius), 4px)' }
  if (!url) return <span className="h-12 w-8 shrink-0 bg-secondary" style={shape} />
  return (
    <img
      decoding="async"
      loading="lazy"
      src={artSrc(url) ?? undefined}
      alt=""
      className={cn('h-12 w-8 shrink-0 bg-secondary object-cover', blur)}
      style={shape}
      // A picture that does not load leaves the blank tile, not a broken image.
      onError={(e) => (e.currentTarget.style.visibility = 'hidden')}
    />
  )
}

/** One game: its picture, its hours, and what it becomes. */
function GameRow({ row, pick, name, showAdult, onPick }: { row: BridgeRow; pick: Pick; name: string; showAdult: boolean; onPick: (p: Pick) => void }) {
  const match = row.options.find((o) => o.slug === pick) ?? null
  const stays = row.kind === 'stays'
  const options = stays
    ? [
        { value: 'own', label: 'Add to my Library' },
        { value: 'skip', label: 'Leave it out' },
      ]
    : [
        ...(row.kind === 'ask' && pick === '' ? [{ value: '', label: 'Choose…' }] : []),
        ...row.options.map((o) => ({ value: o.slug, label: `Kryoto: ${o.title}`, hint: [o.developer, o.year].filter(Boolean).join(' · ') || undefined })),
        { value: 'own', label: 'My own game' },
        { value: 'skip', label: `Leave it in ${name}` },
      ]
  const why = row.remembered ? 'Your earlier answer' : row.how ? HOW[row.how] : null
  const nsfw = row.options.some((o) => o.nsfw)
  return (
    <li className={cn('kryo-radius flex items-center gap-3 border border-border p-2', pick === 'skip' && 'opacity-50')}>
      <Cover url={row.cover} blur={adultBlur(nsfw, showAdult)} />
      <span className="grid min-w-0 grow">
        <span className="truncate text-sm text-foreground">{row.title}</span>
        <span className="truncate text-[11px] text-muted-foreground">
          {[
            row.playtimeSeconds ? formatPlaytime(row.playtimeSeconds) : 'Not played',
            row.lastPlayed ? `last played ${formatLastPlayed(row.lastPlayed)}` : null,
            match && why ? why : null,
            row.joins ? (stays ? `${row.joins} is in your library already` : `joins ${row.joins} in your library`) : null,
          ]
            .filter(Boolean)
            .join(' · ')}
        </span>
      </span>
      <Dropdown size="sm" label={`What ${row.title} becomes`} value={pick} options={options} onChange={onPick} className="w-56 shrink-0" emphasis={pick === ''} />
    </li>
  )
}

/**
 * Kryoto's 3x3 marks for the three steps: a tick for each one done, the one
 * at hand scanning; beside them the whole setup as a filling matrix.
 */
function Progress({ steps, step }: { steps: string[]; step: number }) {
  return (
    <div className="grid justify-items-center gap-2.5">
      <div className="flex justify-center gap-4" aria-hidden>
        {steps.map((label, i) => (
          <span key={label} className={cn('flex items-center gap-1.5 text-[10px] uppercase tracking-wider', i <= step ? 'text-foreground' : 'text-muted-foreground')}>
            <Matrix state={i < step ? 'success' : i === step ? 'scan' : 'empty'} className={cn('size-3', i < step ? 'text-success' : i === step ? 'text-primary' : 'text-muted-foreground')} />
            {label}
          </span>
        ))}
      </div>
      <span className="flex items-center gap-2 text-[10px] uppercase tracking-wider text-muted-foreground">
        <Matrix progress={Math.min(step, steps.length) / steps.length} className="size-3 text-foreground" label={`${Math.min(step, steps.length)} of ${steps.length}`} />
        {step >= steps.length ? 'Done' : `Step ${step + 1} of ${steps.length}`}
      </span>
    </div>
  )
}
