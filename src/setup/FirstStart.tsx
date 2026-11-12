import { useEffect, useState, type ReactNode } from 'react'
import { ArrowLeft, ChevronRight, FolderOpen, ScrollText } from '@/ui/pixel-icons'
import { Button, Check, Matrix, Modal, Segmented } from '@/ui'
import { PixelSplash } from '@/ui/pixel-icons-motion'
import { SceneMotion } from '@/ui/ascii/SceneMotion'
import { HATCH, SCAN_SKIP, SOUND } from '@/ui/ascii/motion'
import { WindowControls } from '@/shell/TitleBar'
import { DefenderSection, defenderDone } from '@/settings/DefenderSection'
import { ProtonSection, protonFound } from '@/settings/CompatPane'
import { call, errorText, isTauri } from '@/lib/bridge'
import { settingsApi, useSettings, type Settings } from '@/lib/settings'
import { formatBytes } from '@/lib/downloads'
import { pickPath } from '@/lib/pick'
import { isWindowsHost } from '@/lib/library'
import { setSoundsOn, soundsOn, play } from '@/lib/sound'
import { fetchReleaseIntro, introId, IntroVideo, markShown, type ReleaseIntroData } from '@/ui/ReleaseIntro'
import { WHATS_NEW, type WhatsNewPlace } from '@/ui/FeatureNotice'
import { usePromptTurn } from '@/lib/prompts'
import { openStorePage } from '@/lib/window'
import { cn } from '@/lib/utils'

/**
 * The first start: a few steps that set Kryoto Desktop up, one thing at a
 * time, in the way the phone and controller setups work. Three ways in, one
 * set of steps:
 *
 * - **A fresh install** (`settings.welcomed` is false) sees all of it, full
 *   window, after sign-in or choosing to go on as a guest (`FirstStart`).
 * - **Someone who updated** sees it once (`SetupPrompt`, recorded as `SETUP_ID`
 *   in `settings.seenNotices`), in its turn in the prompt queue: what changed
 *   and where it lives, then only what they have not done yet.
 * - **Anyone, any time**, from Settings, General
 *   (`SetupDialog`), with nothing left out.
 *
 * Windows gets Windows Security, Linux gets Proton: each is what stands
 * between a download and a game that starts. Game runtimes are asked on the
 * start screen before it (boot/RuntimesPrompt.tsx), so not again here.
 */
type Folder = { path: string; isDefault: boolean; exists: boolean; total: number; free: number }

/** Seen by everyone who has been through the setup, in whichever way. */
export const SETUP_ID = 'setup-0-3-0'

const PLATFORM_STEP: Step = isWindowsHost() ? 'defender' : 'proton'
type Step = 'welcome' | 'news' | 'folder' | 'unpack' | 'defender' | 'proton' | 'look' | 'extras' | 'games' | 'done'
const FRESH: readonly Step[] = ['welcome', 'folder', PLATFORM_STEP, 'look', 'extras', 'games', 'done']
const AGAIN: readonly Step[] = ['folder', PLATFORM_STEP, 'look', 'extras', 'games']

/** After an update: what changed, then only what is still to do. */
async function stepsAfterUpdate(extractInPlace: boolean): Promise<Step[]> {
  const ready = await (isWindowsHost() ? defenderDone() : protonFound()).catch(() => false)
  return ['news', ...(extractInPlace ? [] : (['unpack'] as const)), ...(ready ? [] : [PLATFORM_STEP])]
}

/** Save one change straight away, on top of whatever is saved now. */
async function save(patch: Partial<Settings>) {
  await settingsApi.change((now) => ({ ...now, ...patch }))
}

/** This version's introduction video, when it has one. */
function useIntro() {
  const [intro, setIntro] = useState<ReleaseIntroData | null>(null)
  useEffect(() => {
    void fetchReleaseIntro().then(setIntro)
  }, [])
  return intro
}

export function FirstStart({ onDone }: { onDone: (start: 'store' | 'library') => void }) {
  const settings = useSettings()
  const [at, setAt] = useState(0)
  const intro = useIntro()
  const step: Step = FRESH[at] ?? 'done'

  // Done or skipped: never shown again, and this version's update video counts as seen.
  const finish = (start: 'store' | 'library') => {
    void settingsApi
      .get()
      .then((now) => settingsApi.save({ ...now, welcomed: true, startPage: start, seenNotices: [...new Set([...now.seenNotices, introId(), SETUP_ID])] }))
      .finally(() => onDone(start))
  }

  if (!settings) return null
  const next = () => setAt((i) => Math.min(FRESH.length - 1, i + 1))
  const back = () => setAt((i) => Math.max(0, i - 1))

  return (
    <main className="relative grid h-screen grid-rows-[auto_auto_1fr_auto] bg-background text-foreground">
      {/* The window has no frame of its own, and the title bar comes with the client. */}
      <div data-maximize className="drag flex h-9 justify-end">
        {isTauri() ? <WindowControls /> : null}
      </div>
      <header className="flex items-center justify-between gap-4 px-8 pt-2">
        <Progress steps={FRESH} at={at} />
        {step !== 'done' ? (
          <button type="button" onClick={() => finish(settings.startPage)} className="kryo-pill px-3 py-1.5 text-[11px] uppercase tracking-wider text-muted-foreground hover:bg-secondary hover:text-foreground">
            Skip setup
          </button>
        ) : null}
      </header>

      <section key={step} aria-live="polite" className="kryo-in mx-auto grid w-full max-w-xl content-center justify-items-center gap-5 overflow-auto px-8 py-6 text-center">
        <StepBody step={step} settings={settings} intro={intro} />
      </section>

      <footer className="flex items-center justify-between gap-3 border-t border-border px-8 py-4">
        {at > 0 ? (
          <Button variant="ghost" onClick={back}>
            <ArrowLeft className="size-3.5" />
            Back
          </Button>
        ) : (
          <span />
        )}
        {step === 'done' ? (
          <div className="flex gap-2">
            <Button onClick={() => finish('library')}>Open my library</Button>
            <Button variant="primary" onClick={() => finish('store')}>
              Find a game
            </Button>
          </div>
        ) : (
          <Button variant="primary" onClick={next}>
            {step === 'welcome' ? 'Get started' : 'Next'}
            <ChevronRight className="size-3.5" />
          </Button>
        )}
      </footer>
    </main>
  )
}

/** What one step shows, wherever it is shown. */
function StepBody({ step, settings, intro, news }: { step: Step; settings: Settings; intro: ReleaseIntroData | null; news?: (place: WhatsNewPlace) => void }) {
  const [watching, setWatching] = useState(false)
  if (step === 'welcome') {
    return (
      <Page
        icon={watching && intro ? null : 'Gamepad'}
        title="Welcome to Kryoto Desktop"
        text="Your games from kryo.to, downloaded, unpacked, kept up to date and started in one place. A few quick choices and you are in."
      >
        {watching && intro ? <IntroVideo video={intro.video} /> : null}
        {intro && !watching ? (
          <Button variant="ghost" size="sm" onClick={() => setWatching(true)}>
            Watch the introduction
          </Button>
        ) : null}
      </Page>
    )
  }
  if (step === 'news') return <NewsStep intro={intro} onPlace={news} />
  if (step === 'folder') return <FolderStep extractInPlace={settings.extractInPlace} />
  if (step === 'unpack') return <UnpackStep extractInPlace={settings.extractInPlace} />
  if (step === 'defender') return <DefenderStep />
  if (step === 'proton') return <ProtonStep settings={settings} />
  if (step === 'look') {
    return (
      <Page icon="Palette" title="How it looks" text="Kryoto Desktop can follow kryo.to's theme, your system, or always be light or dark.">
        <Segmented
          label="Theme"
          value={settings.theme ?? 'account'}
          options={[
            { value: 'account', label: 'Follow kryo.to' },
            { value: 'system', label: 'System' },
            { value: 'light', label: 'Light' },
            { value: 'dark', label: 'Dark' },
          ]}
          onChange={(v) => void save({ theme: v as Settings['theme'] })}
        />
      </Page>
    )
  }
  if (step === 'extras') return <ExtrasStep settings={settings} />
  if (step === 'games') {
    return (
      <Page
        icon="FolderSearch"
        title="Games you already have"
        text="Games on this PC from Steam, other launchers or anywhere else can join your library: Library, then Add a game, finds them and lets you pick. Their play time stays on this PC."
      />
    )
  }
  return (
    <Page icon="Check" title="You're set" text="Everything here is in Settings too, and Settings, General opens this setup again.">
      <Matrix state="success" className="size-5 text-success" announce />
    </Page>
  )
}

/** The setup in a dialog over the app: after an update, from Settings, or only What's new from the account menu. */
export function SetupDialog({
  steps,
  title,
  onClose,
  onPlace,
}: {
  steps: readonly Step[]
  title: string
  onClose: () => void
  /** A row of What's new was pressed: go there. */
  onPlace?: (place: WhatsNewPlace) => void
}) {
  const settings = useSettings()
  const [at, setAt] = useState(0)
  const intro = useIntro()
  const step: Step = steps[at] ?? 'done'
  const last = at >= steps.length - 1
  if (!settings) return null
  return (
    <Modal
      title={title}
      wide
      onClose={onClose}
      footer={
        <div className="flex w-full items-center justify-between gap-3">
          {at > 0 ? (
            <Button variant="ghost" onClick={() => setAt(at - 1)}>
              <ArrowLeft className="size-3.5" />
              Back
            </Button>
          ) : step === 'news' ? (
            <Button
              variant="ghost"
              onClick={() => {
                onClose()
                openStorePage('/changelog')
              }}
            >
              <ScrollText className="size-3.5" />
              Full changelog
            </Button>
          ) : (
            <span />
          )}
          {steps.length > 1 ? <Progress steps={steps} at={at} /> : null}
          <Button variant="primary" onClick={() => (last ? onClose() : setAt(at + 1))}>
            {last ? 'Done' : 'Next'}
            {last ? null : <ChevronRight className="size-3.5" />}
          </Button>
        </div>
      }
    >
      <section key={step} aria-live="polite" className="kryo-in mx-auto grid w-full max-w-3xl content-center justify-items-center gap-3 text-center">
        <StepBody
          step={step}
          settings={settings}
          intro={intro}
          news={(place) => {
            onClose()
            onPlace?.(place)
          }}
        />
      </section>
    </Modal>
  )
}

/**
 * The setup after an update, once. It holds its place in the queue from the
 * start, so nothing else opens first while Windows answers what is already
 * done. However it is closed it counts as seen, with this version's video
 * (offered here) and the notice about unpacking when it asked about that.
 */
export function SetupPrompt({ onPlace }: { onPlace: (place: WhatsNewPlace) => void }) {
  const settings = useSettings()
  const [closed, setClosed] = useState(false)
  // Saved as seen once it opens (KB-0259); it stays up until it is closed.
  const [shown, setShown] = useState(false)
  const [steps, setSteps] = useState<Step[] | null>(null)
  const due = !!settings && settings.welcomed && !settings.seenNotices.includes(SETUP_ID) && !closed
  const turn = usePromptTurn('setup', (due || shown) && !closed)
  const inPlace = settings?.extractInPlace ?? false
  useEffect(() => {
    if (!due || steps) return
    let alive = true
    void stepsAfterUpdate(inPlace).then((s) => alive && setSteps(s))
    return () => {
      alive = false
    }
    // Decided once: ticking the box in a step must not take the step away.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [due])
  useEffect(() => {
    if (!turn || !steps || shown) return
    setShown(true)
    // What's new tells them about these: no dots and intros saying it again.
    void markShown([introId(), SETUP_ID, ...(steps.includes('unpack') ? ['hatchzip'] : [])])
  }, [turn, steps, shown])
  if (!turn || !steps) return null
  const close = () => {
    setClosed(true)
    // What's new has just told them about these: no dots and intros saying it again.
    const seen = [introId(), SETUP_ID, ...(steps.includes('unpack') ? ['hatchzip'] : [])]
    void settingsApi.change((now) => ({ ...now, seenNotices: [...new Set([...now.seenNotices, ...seen])] }))
  }
  return <SetupDialog steps={steps} title={`What's new in Kryoto Desktop ${__APP_VERSION__}`} onClose={close} onPlace={onPlace} />
}

/** Reopened on purpose: every step, nothing skipped. */
export const SETUP_AGAIN = AGAIN
/** What's new alone, from the account menu. */
export const SETUP_NEWS: readonly Step[] = ['news']

/** What changed and where it lives: each row goes there. */
function NewsStep({ intro, onPlace }: { intro: ReleaseIntroData | null; onPlace?: (place: WhatsNewPlace) => void }) {
  const [watching, setWatching] = useState(false)
  return (
    <>
      <p className="max-w-lg text-xs leading-relaxed text-muted-foreground">Here is what changed, and where to find it. Everything you had is where you left it.</p>
      {watching && intro ? <IntroVideo video={intro.video} /> : null}
      {intro && !watching ? (
        <Button size="sm" onClick={() => setWatching(true)}>
          Watch the introduction
        </Button>
      ) : null}
      <ul className="grid w-full gap-2 text-left sm:grid-cols-2">
        {WHATS_NEW.map((n) => (
          <li key={n.title} className="grid">
            <button
              type="button"
              onClick={() => onPlace?.(n.place)}
              className="kryo-radius grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 border border-border bg-background/40 p-3 text-left transition-colors hover:border-foreground/50"
            >
              <span className="row-span-3 pt-0.5 text-muted-foreground [&>svg]:size-4">{n.icon}</span>
              <b className="text-xs text-foreground">{n.title}</b>
              <span className="text-[11px] leading-relaxed text-muted-foreground">{n.body}</span>
              <span className="pt-1 text-[10px] uppercase tracking-wider text-foreground/80">{n.where}</span>
            </button>
          </li>
        ))}
      </ul>
    </>
  )
}

/** One step: its big icon, a title, a line, and what it asks. */
function Page({ icon, title, text, children }: { icon: Parameters<typeof PixelSplash>[0]['icon'] | null; title: string; text: string; children?: ReactNode }) {
  return (
    <>
      {icon ? <PixelSplash icon={icon} className="size-24 text-foreground" /> : null}
      <div className="grid gap-1.5">
        <h1 className="text-xl font-bold text-foreground">{title}</h1>
        <p className="text-xs leading-relaxed text-muted-foreground">{text}</p>
      </div>
      {children ? <div className="grid w-full justify-items-center gap-3">{children}</div> : null}
    </>
  )
}

/** One 3x3 matrix per step: a tick for each one behind, the current one working, the rest waiting. */
function Progress({ steps, at }: { steps: readonly Step[]; at: number }) {
  return (
    <ol className="flex items-center gap-2" aria-label={`Step ${at + 1} of ${steps.length}`}>
      {steps.map((s, i) => (
        <li key={s}>
          <Matrix state={i < at ? 'success' : i === at ? 'busy' : 'empty'} className={cn('size-3.5', i <= at ? 'text-primary' : 'text-muted-foreground')} />
        </li>
      ))}
    </ol>
  )
}

/** Where new games go: the default library folder, how much room it has, another one, and unpacking in place. */
function FolderStep({ extractInPlace }: { extractInPlace: boolean }) {
  const [folder, setFolder] = useState<Folder | null>(null)
  const [error, setError] = useState<string | null>(null)
  const load = () =>
    void call<{ folders: Folder[] }>('storage_overview')
      .then((o) => setFolder(o.folders.find((f) => f.isDefault) ?? o.folders[0] ?? null))
      .catch((e: unknown) => setError(errorText(e)))
  useEffect(load, [])

  const choose = async () => {
    setError(null)
    try {
      const dir = isTauri() ? await pickPath({ directory: true, title: 'Where should new games go?' }) : 'E:\\Kryoto Games'
      if (typeof dir !== 'string') return
      await call('storage_add_folder', { path: dir })
      await call('storage_set_default', { path: dir })
      load()
    } catch (e) {
      setError(errorText(e))
    }
  }

  return (
    <Page icon="HardDrive" title="Where your games go" text="Each game gets a folder of its own here. More folders, on other drives too, can be added in Settings, Storage.">
      <div className="kryo-radius grid w-full gap-1 border border-border bg-card px-4 py-3 text-left">
        <span className="select-text truncate font-mono text-xs text-foreground">{folder?.path ?? '…'}</span>
        <span className="text-[11px] text-muted-foreground">{folder ? `${formatBytes(folder.free)} free of ${formatBytes(folder.total)}` : 'Looking at the drive'}</span>
      </div>
      <Button size="sm" onClick={() => choose()}>
        <FolderOpen className="size-3.5" />
        Choose another folder
      </Button>
      {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
      <div className="grid w-full justify-items-center gap-2 pt-2">
        <SceneMotion scene={HATCH} className="h-12 text-muted-foreground" />
        <Check checked={extractInPlace} onChange={(v) => void save({ extractInPlace: v })} label="Unpack every game in place, so it needs room for the game and not twice that" />
      </div>
    </Page>
  )
}

/** After an update the folder is long since chosen: only what is new about it. */
function UnpackStep({ extractInPlace }: { extractInPlace: boolean }) {
  return (
    <Page icon={null} title="Room for the game, not for its archive too" text="Kryoto now does this by itself when a drive is short. It can do it for every install, so a game needs about its own size and not twice that.">
      <SceneMotion scene={HATCH} className="h-[72px] text-muted-foreground" />
      <Check checked={extractInPlace} onChange={(v) => void save({ extractInPlace: v })} label="Unpack every game in place" />
    </Page>
  )
}

/** Windows Security exclusions for the game folders: optional, with the risk said plainly. Settings, Storage has it too. */
function DefenderStep() {
  return (
    <Page icon={null} title="Windows Security and your games" text="Optional: leave it, and Settings, Storage has it later.">
      <SceneMotion scene={SCAN_SKIP} className="h-12 text-muted-foreground" />
      <DefenderSection compact />
    </Page>
  )
}

/** Proton for Windows games: what Linux needs before a game can start. Settings, Compatibility has it too. */
function ProtonStep({ settings }: { settings: Settings }) {
  return (
    <Page icon={null} title="Windows games on Linux" text="Most games are made for Windows. Proton runs them here, and Kryoto can get it for you. Leave it, and Settings, Running games has it later.">
      <div className="w-full text-left">
        <ProtonSection defaultTool={settings.defaultCompatTool} onDefaultTool={(tool) => void save({ defaultCompatTool: tool })} />
      </div>
    </Page>
  )
}

/** Sounds, the pet, starting with the computer, and error reports: each a tick, each changeable later. */
function ExtrasStep({ settings }: { settings: Settings }) {
  const [sounds, setSounds] = useState(soundsOn)
  return (
    <Page icon="Sparkles" title="A few extras" text="All of them can be changed later in Settings.">
      <SceneMotion scene={SOUND} className="h-9 text-muted-foreground" />
      <div className="grid w-full justify-items-start gap-2.5 text-left">
        <Check
          checked={sounds}
          onChange={(v) => {
            setSounds(v)
            setSoundsOn(v)
            if (v) play('ready', { force: true })
          }}
          label="Sounds for finished downloads, achievements and messages"
        />
        <Check checked={settings.pet} onChange={(v) => void save({ pet: v })} label="A desktop pet that keeps you company and shows your downloads" />
        <Check
          checked={settings.startWithSystem}
          onChange={(v) => void save({ startWithSystem: v })}
          label={`Start Kryoto when I sign in to ${isWindowsHost() ? 'Windows' : 'my computer'}`}
        />
        <Check checked={settings.sendReports} onChange={(v) => void save({ sendReports: v })} label="Send crash and error reports, so problems get fixed (no personal files)" />
      </div>
    </Page>
  )
}
