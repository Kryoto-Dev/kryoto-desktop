import { useEffect, useRef, useState, type ReactNode } from 'react'
import { ArrowUpRight, BookOpen, Cloud, FolderInput, Glasses, HardDrive, History, Layers, Server, Settings, Shield, X } from '@/ui/pixel-icons'
import { Button, Caption, IconButton } from '@/ui'
import { SceneMotion, type AnyScene } from '@/ui/ascii/SceneMotion'
import { HATCH } from '@/ui/ascii/motion'
import { settingsApi, useSettings } from '@/lib/settings'
import { openExternal, openSettingsSection } from '@/lib/window'
import type { SettingsTarget } from '@/lib/settings-links'
import { logWarn } from '@/lib/log'
import { usePromptTurn, useQuiet } from '@/lib/prompts'
import { cn } from '@/lib/utils'
import type { SettingsSection } from '@/settings/SettingsPage'

/**
 * What is new in Kryoto Desktop, told without a pile of pop-ups.
 *
 * - **One pop-up per update, at most** (`FEATURE_NOTICES`): something that
 *   steps in by itself and changes what the player sees, so they hear about
 *   it before it surprises them. For 0.3.0 that is HatchZip, which unpacks in
 *   place on its own when a drive is short. It waits its turn (lib/prompts.ts)
 *   and sits above the toasts, so nothing lands on it.
 * - **Everything else is quiet** (`NEW_IN_SETTINGS`): a dot on the account
 *   menu and on the setting's entry in the Settings rail.
 *
 * Both are told once, recorded in `settings.seenNotices`.
 */
export type FeatureNoticeSpec = {
  id: string
  title: string
  body: ReactNode
  scene: AnyScene
  /** A kryo.to path, opened in the Store. */
  learnMore?: string
  /** A video to watch, opened in the browser. */
  video?: string
  /** The setting it names: a button opens it there. */
  settings?: SettingsTarget & { label: string }
}

export const FEATURE_NOTICES: FeatureNoticeSpec[] = [
  {
    id: 'hatchzip',
    title: 'Room for the game, not for its archive too',
    body: 'When a drive is short, Kryoto now unpacks a game in place, so it needs about the size of the game and not twice that. Settings, Downloads can turn it on for every install.',
    scene: HATCH,
    learnMore: '/faq#free-space',
    settings: { section: 'downloads', anchor: 'unpacking', label: 'Open Downloads' },
  },
]

/**
 * New settings, by the Settings section they live in. `intro` is shown at the
 * top of a Desktop section; a kryo.to section (Appearance) says it itself.
 */
type NewSetting = { id: string; intro?: Omit<FeatureNoticeSpec, 'id'> }
export const NEW_IN_SETTINGS: Partial<Record<SettingsSection, NewSetting>> = {
  // Pixel icons, with a Classic choice: kryo.to's Appearance page tags it New itself.
  appearance: { id: 'icons' },
}

/** Where a row of What's new goes when it is pressed. */
export type WhatsNewPlace = (SettingsTarget & { kind: 'settings' }) | { kind: 'library' } | { kind: 'bridge' } | { kind: 'store' }

/**
 * What this version is about, in the words the app itself uses, each with the
 * place it lives. The setup opens with it after an update (setup/FirstStart.tsx)
 * and the account menu has it for good. Rewritten for each version that changes
 * how the app is used; the full list is the changelog.
 */
export const WHATS_NEW: { title: string; body: string; where: string; icon: ReactNode; place: WhatsNewPlace }[] = [
  {
    title: 'Bridge',
    body: 'Your Steam and Hydra games in your Library, and your Kryoto games in Steam.',
    where: 'Library, Bridge',
    icon: <FolderInput />,
    place: { kind: 'bridge' },
  },
  {
    title: 'Saves and achievements for everyone',
    body: 'Versions of your saves and your achievements, kept on this PC or on your own server.',
    where: 'Settings, Cloud',
    icon: <Cloud />,
    place: { kind: 'settings', section: 'cloud' },
  },
  {
    title: 'Collections in your Library',
    body: 'Yours and the ones you follow are in the side rail, and one press installs a whole collection.',
    where: 'Library, side rail',
    icon: <Layers />,
    place: { kind: 'library' },
  },
  {
    title: 'Game updates you can undo',
    body: 'Kryoto can update your games by itself and put the last build back if one does not run right.',
    where: 'Settings, Downloads',
    icon: <History />,
    place: { kind: 'settings', section: 'downloads', anchor: 'game-updates' },
  },
  {
    title: 'Room for the game alone',
    body: 'When a drive is short a game unpacks in place, so it needs about its own size and not twice that.',
    where: 'Settings, Downloads',
    icon: <HardDrive />,
    place: { kind: 'settings', section: 'downloads', anchor: 'unpacking' },
  },
  {
    title: 'Your own servers and debrid',
    body: 'Keep saves and achievements on a server of yours, and put Real-Debrid or TorBox in front of downloads.',
    where: 'Settings, Cloud and Downloads',
    icon: <Server />,
    place: { kind: 'settings', section: 'patchbay' },
  },
  {
    title: 'VR games start in VR',
    body: 'A VR game gets Play in VR, and Kryoto starts SteamVR first.',
    where: 'Settings, Running games',
    icon: <Glasses />,
    place: { kind: 'settings', section: 'vr' },
  },
  {
    title: 'Fewer ads on other sites',
    body: 'Sites you open from the address bar load without their ad servers. Press its shield to see what it blocked.',
    where: 'Store, address bar',
    icon: <Shield />,
    place: { kind: 'store' },
  },
]

/** Record a notice or a new setting as seen, once. */
export function markSeen(id: string) {
  void settingsApi
    .change((latest) => (latest.seenNotices.includes(id) ? latest : { ...latest, seenNotices: [...latest.seenNotices, id] }))
    .catch((e: unknown) => logWarn('notice', `could not mark ${id} seen: ${String(e)}`))
}

/** kryo.to's own sections: a guest cannot open them, so they are never new to one. */
const ACCOUNT_SECTIONS: ReadonlySet<SettingsSection> = new Set(['profile', 'appearance', 'notifications', 'security', 'support'])

/** Settings sections with something the player has not opened yet. */
export function useNewSettings(guest = false): SettingsSection[] {
  const settings = useSettings()
  if (!settings) return []
  const seen = new Set(settings.seenNotices)
  return (Object.entries(NEW_IN_SETTINGS) as [SettingsSection, NewSetting][])
    .filter(([section, n]) => !seen.has(n.id) && !(guest && ACCOUNT_SECTIONS.has(section)))
    .map(([section]) => section)
}

/** The small dot beside something new. */
export function NewDot({ className }: { className?: string }) {
  return <span className={cn('kryo-pill inline-block size-1.5 shrink-0 bg-primary', className)} aria-label="New" role="img" />
}

/**
 * The dot beside one new setting inside a section that already has its own
 * (`NEW_IN_SETTINGS`): shown the first time it is on screen, which marks it seen.
 */
export function NewHere({ id }: { id: string }) {
  const settings = useSettings()
  const shown = useRef<boolean | null>(null)
  if (shown.current === null && settings) shown.current = !settings.seenNotices.includes(id)
  useEffect(() => {
    if (shown.current) markSeen(id)
  }, [id, settings])
  return shown.current ? <NewDot /> : null
}

export function HatchZipExplainer({ compact = false, className }: { compact?: boolean; className?: string }) {
  return (
    <div className={cn('flex items-start gap-4', className)}>
      <SceneMotion scene={HATCH} className={compact ? 'h-12 text-muted-foreground' : 'h-[72px] text-muted-foreground'} label="HatchZip off, the drive runs out; on, the archive shrinks as the game appears" />
      <p className="text-xs leading-relaxed text-muted-foreground">
        Unpacking usually needs room for the game and its archive at the same time. In place, the archive is read from its end and cut short as each file lands, so the drive only has to hold the game and a little extra. A few archives packed as one block gain nothing from it, and 7-Zip unpacks those as before.
      </p>
    </div>
  )
}

export function FeatureNotice({ onWeb }: { onWeb: (path: string) => void }) {
  const settings = useSettings()
  const [dismissed, setDismissed] = useState<Set<string>>(new Set())
  // Not in the first seconds: the client is still drawing itself, and the
  // questions that open by themselves (lib/prompts.ts) have the first turn.
  const quiet = useQuiet(12000)
  const seen = new Set([...(settings?.seenNotices ?? []), ...dismissed])
  const next = settings ? FEATURE_NOTICES.find((n) => !seen.has(n.id)) : undefined
  const turn = usePromptTurn('notice', quiet && !!next)
  if (!next || !turn) return null

  const dismiss = () => {
    setDismissed((d) => new Set(d).add(next.id))
    markSeen(next.id)
  }

  return (
    <aside
      role="status"
      aria-label={`New: ${next.title}`}
      className="kryo-pop kryo-radius grid w-[380px] max-w-[calc(100vw-32px)] gap-3 border border-border bg-card/95 p-4 shadow-2xl shadow-black/60 backdrop-blur"
    >
      <div className="flex items-start justify-between gap-3">
        <Caption>New in Kryoto Desktop</Caption>
        <IconButton label="Dismiss" onClick={dismiss} className="-mr-1 -mt-1 size-7">
          <X className="size-3.5" />
        </IconButton>
      </div>
      <SceneMotion scene={next.scene} className="h-[72px] text-foreground" />
      <div className="grid gap-1">
        <p className="text-sm font-bold text-foreground">{next.title}</p>
        <p className="text-xs leading-relaxed text-muted-foreground">{next.body}</p>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {next.settings ? (
          <Button size="sm" onClick={() => { openSettingsSection(next.settings!.section, next.settings!.anchor); dismiss() }}>
            <Settings className="size-3" />
            {next.settings.label}
          </Button>
        ) : null}
        {next.learnMore ? (
          <Button size="sm" variant={next.settings ? 'ghost' : undefined} onClick={() => { onWeb(next.learnMore!); dismiss() }}>
            <BookOpen className="size-3" />
            Learn more
          </Button>
        ) : null}
        {next.video ? (
          <Button size="sm" variant="ghost" onClick={() => void openExternal(next.video!)}>
            Watch video
            <ArrowUpRight className="size-3" />
          </Button>
        ) : null}
        <Button size="sm" variant="ghost" className="ml-auto" onClick={dismiss}>
          Got it
        </Button>
      </div>
    </aside>
  )
}
