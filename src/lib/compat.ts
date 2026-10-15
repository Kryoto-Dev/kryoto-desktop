import { catalogApiUrl } from '@/lib/endpoint'

/**
 * What kryo.to's testers found for a game, platform by platform
 * (`/api/games/<slug>/compatibility`). The words and statuses are the site's
 * (kryo.to/lib/compat.ts); only the few this app shows are copied here.
 */

export type Platform = 'windows' | 'linux' | 'steamdeck' | 'macos'
export type Outcome = 'works' | 'setup' | 'partial' | 'broken'
export type PlatformStatus = Outcome | 'mixed' | 'retest' | 'untested' | 'na'

export const PLATFORM_LABEL: Record<Platform, string> = {
  windows: 'Windows',
  linux: 'Linux',
  steamdeck: 'Steam Deck',
  macos: 'macOS',
}

export const STATUS_INFO: Record<PlatformStatus, { label: string; tone: 'good' | 'warn' | 'bad' | 'none' }> = {
  works: { label: 'Working', tone: 'good' },
  setup: { label: 'Working, setup needed', tone: 'good' },
  partial: { label: 'Partly working', tone: 'warn' },
  broken: { label: 'Not working', tone: 'bad' },
  mixed: { label: 'Mixed reports', tone: 'warn' },
  retest: { label: 'Needs retesting', tone: 'none' },
  untested: { label: 'Not tested', tone: 'none' },
  na: { label: 'Not applicable', tone: 'none' },
}

const LAYER_LABEL: Record<string, string> = {
  proton: 'Proton',
  proton_ge: 'GE-Proton',
  wine: 'Wine',
  crossover: 'CrossOver',
  whisky: 'Whisky',
  gptk: 'Game Porting Toolkit',
  vm: 'Windows in a virtual machine',
}

type PlatformSummary = {
  status: PlatformStatus
  /** Accepted reports on the current build. */
  reports: number
  /** Accepted reports on older builds. */
  older: number
  detail: string | null
}

export type CompatReport = {
  id: string
  platform: Platform | null
  outcome: Outcome
  version: string
  current: boolean
  env: { osVersion?: string; layer?: string; layerVersion?: string; launcher?: string }
  instructions: string | null
  /** Already masked by kryo.to: an anonymous or deleted account has no username. */
  tester: { name: string; username: string | null }
  testedAt: string
}

export type Compatibility = {
  headline: string
  platforms: Record<Platform, PlatformSummary>
  /** Reports that never said what they ran on. */
  unspecified: PlatformSummary
  reports: CompatReport[]
}

/** Null when kryo.to has nothing to say: an older site, or a game nobody tested. */
export async function fetchCompatibility(slug: string): Promise<Compatibility | null> {
  const res = await fetch(await catalogApiUrl(`/api/games/${encodeURIComponent(slug)}/compatibility`))
  if (!res.ok) return null
  const json = (await res.json()) as {
    summary?: { headline?: string; platforms?: Record<Platform, PlatformSummary>; unspecified?: PlatformSummary }
    reports?: CompatReport[]
  }
  const s = json.summary
  if (!s?.platforms || !s.unspecified || typeof s.headline !== 'string') return null
  const reports = Array.isArray(json.reports) ? json.reports : []
  const tested = Object.values(s.platforms).some((p) => p.reports + p.older > 0) || s.unspecified.reports + s.unspecified.older > 0
  if (!tested && reports.length === 0) return null
  return { headline: s.headline, platforms: s.platforms, unspecified: s.unspecified, reports }
}

/** The platform this copy runs on. Linux players play the Windows build through Proton or Wine. */
export function hostPlatform(): Platform {
  const ua = navigator.userAgent
  if (/Windows/i.test(ua)) return 'windows'
  if (/Mac OS X|Macintosh/i.test(ua)) return 'macos'
  return 'linux'
}

/**
 * The newest steps someone followed to get it working here: a report that
 * worked on this platform and wrote its setup down, the current build's first.
 */
export function workingSetup(c: Compatibility, platform: Platform): CompatReport | null {
  const working = c.reports.filter((r) => r.platform === platform && (r.outcome === 'works' || r.outcome === 'setup') && r.instructions?.trim())
  return working.find((r) => r.current) ?? working[0] ?? null
}

/** "Proton 9.0, Arch Linux" - how a report was run, in a few words. */
export function setupLabel(r: CompatReport): string {
  const layer = r.env.layer ? (LAYER_LABEL[r.env.layer] ?? null) : null
  return [layer && [layer, r.env.layerVersion].filter(Boolean).join(' '), r.env.launcher, r.env.osVersion].filter(Boolean).join(', ')
}
