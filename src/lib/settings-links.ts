import type { SettingsSection } from '@/settings/SettingsPage'
import { anchorId } from '@/lib/setting-anchor'

/**
 * Addresses inside Settings.
 *
 * Every rail entry is a section, and every `Section` in it (ui/index.tsx) has
 * an anchor made from its title: "Desktop pet" is `desktop-pet`. Written as
 * `general#desktop-pet`, the address is what `openSettingsSection`
 * (lib/window.ts) carries, what a toast that names a setting opens, and what
 * `kryoto://settings/general/desktop-pet` asks for from outside (links.rs).
 * Arriving scrolls to it and lights it for a moment, as kryo.to does.
 */

export type SettingsTarget = { section: SettingsSection; anchor?: string }

export const SETTINGS_SECTIONS = [
  'profile',
  'appearance',
  'notifications',
  'security',
  'support',
  'general',
  'devices',
  'storage',
  'downloads',
  'sounds',
  'chat',
  'runtimes',
  'phone',
  'controller',
  'compat',
  'developer',
  'logs',
  'cloud',
  'achievements',
  'patchbay',
  'vr',
] as const satisfies readonly SettingsSection[]

// Fails to compile when a section is added to SettingsSection and not here.
type Unlisted = Exclude<SettingsSection, (typeof SETTINGS_SECTIONS)[number]>
const everySectionListed: [Unlisted] extends [never] ? true : never = true
void everySectionListed

/**
 * Pages that became part of another keep their address: `settings/phone`,
 * `settings/patchbay` and the rest still go to the same words, in their new
 * place. An anchor asked for by name is kept.
 */
const windows = typeof navigator !== 'undefined' && /Windows/i.test(navigator.userAgent)
const running = windows ? ('runtimes' as const) : ('compat' as const)
const MOVED: Partial<Record<SettingsSection, SettingsTarget>> = {
  achievements: { section: 'cloud' },
  patchbay: { section: 'cloud', anchor: 'patchbay' },
  developer: { section: 'logs', anchor: 'kryo-to-endpoint' },
  sounds: { section: 'general', anchor: 'sounds' },
  phone: { section: 'devices', anchor: 'phone' },
  controller: { section: 'devices', anchor: 'controller' },
  vr: { section: running, anchor: 'vr' },
  runtimes: { section: running },
  compat: { section: running },
}

/** Where a section lives now. */
export function settingsHome(target: SettingsTarget): SettingsTarget {
  const moved = MOVED[target.section]
  if (!moved) return target
  // Debrid was under Patchbay; it runs downloads, so Downloads has it.
  if (target.section === 'patchbay' && target.anchor === 'debrid') return { section: 'downloads', anchor: 'debrid' }
  return { section: moved.section, anchor: target.anchor ?? moved.anchor }
}

const isSection = (s: string): s is SettingsSection => (SETTINGS_SECTIONS as readonly string[]).includes(s)

export { anchorId, anchorOf } from '@/lib/setting-anchor'

/** `section`, `section#anchor` or `section/anchor`; null for anything that is not one. */
export function parseSettingsAddress(raw: string): SettingsTarget | null {
  const [section = '', anchor] = raw.trim().toLowerCase().split(/[#/]/, 2)
  if (!isSection(section)) return null
  return anchor && /^[a-z0-9-]{1,64}$/.test(anchor) ? { section, anchor } : { section }
}

/** How long the target stays lit. Matches `kryo-deep-link-flash` in styles.css. */
const FLASH_MS = 2000

/**
 * Wait for the anchor to be drawn (a pane loads its settings first), then
 * scroll to it and light it up. Bounded: a section that is not there on this
 * PC (Graphics is Linux only) is simply not found.
 */
export function revealSetting(anchor: string): () => void {
  const started = Date.now()
  let timer = 0
  const attempt = () => {
    const el = document.getElementById(anchorId(anchor))
    if (!el) {
      if (Date.now() - started < 3000) timer = window.setTimeout(attempt, 80)
      return
    }
    // Folded away (More options, Patchbay): opened, or there is nothing to light.
    for (let up: HTMLElement | null = el; up; up = up.parentElement) if (up instanceof HTMLDetailsElement) up.open = true
    el.scrollIntoView({ behavior: 'smooth', block: 'center' })
    el.classList.remove('kryo-deep-link-target')
    void el.offsetWidth
    el.classList.add('kryo-deep-link-target')
    window.setTimeout(() => el.classList.remove('kryo-deep-link-target'), FLASH_MS)
  }
  attempt()
  return () => window.clearTimeout(timer)
}
