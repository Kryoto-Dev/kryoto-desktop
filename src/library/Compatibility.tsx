import { TeamName } from '@/ui/TeamName'
import { useEffect, useState } from 'react'
import { ChevronDown } from 'lucide-react'
import { Caption } from '@/ui'
import { useOnline } from '@/lib/online'
import { cn } from '@/lib/utils'
import { fetchCompatibility, hostPlatform, PLATFORM_LABEL, setupLabel, STATUS_INFO, workingSetup, type Compatibility as Compat } from '@/lib/compat'

const TONE = { good: 'text-success', warn: 'text-warning', bad: 'text-destructive', none: 'text-muted-foreground' } as const

const linkClass = 'rounded-sm text-foreground underline-offset-2 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring'

/**
 * What testers found on this PC's platform, inside the game's Info card: the
 * site's headline, this platform's status and report count, and the newest
 * setup that worked here (Proton steps, for a Linux player). Nothing at all
 * for a game nobody has tested. The steps are shown as plain text, never as
 * HTML, so a report cannot load anything; the full list is on kryo.to.
 */
export function CompatibilityFacts({ slug, onWeb }: { slug: string; onWeb: (path: string) => void }) {
  const [compat, setCompat] = useState<Compat | null>(null)
  const online = useOnline()
  useEffect(() => {
    setCompat(null)
    if (!online) return
    let cancelled = false
    fetchCompatibility(slug)
      .then((c) => !cancelled && setCompat(c))
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [slug, online])

  if (!compat) return null
  const platform = hostPlatform()
  const here = compat.platforms[platform]
  const info = STATUS_INFO[here.status]
  const count = here.reports + here.older
  const setup = workingSetup(compat, platform)
  const go = (path: string) => (e: React.MouseEvent) => {
    e.preventDefault()
    onWeb(path)
  }
  const listPath = `/game/${encodeURIComponent(slug)}#compatibility-${platform}`
  const how = setup ? setupLabel(setup) : ''

  return (
    <div className="grid gap-2 border-t border-border pt-3 text-xs">
      <div className="flex items-baseline justify-between gap-2">
        <Caption>On {PLATFORM_LABEL[platform]}</Caption>
        <a href={`https://kryo.to${listPath}`} onClick={go(listPath)} className={cn(linkClass, 'text-[11px] text-muted-foreground')}>
          {count === 0 ? 'No reports' : count === 1 ? '1 report' : `${count} reports`}
        </a>
      </div>
      <p className={cn('m-0 font-bold', TONE[info.tone])}>{info.label}</p>
      <p className="m-0 text-[11px] leading-relaxed text-muted-foreground">{compat.headline}</p>
      {here.detail ? <p className="m-0 text-[11px] leading-relaxed text-muted-foreground">{here.detail}</p> : null}
      {setup ? (
        <details className="group" open={here.status === 'setup' || undefined}>
          <summary className="flex cursor-pointer list-none items-center justify-between text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:text-foreground">
            Setup that worked
            <ChevronDown className="size-3 transition-transform group-open:rotate-180" />
          </summary>
          <div className="mt-2 grid gap-2">
            <p className="m-0 max-h-60 select-text overflow-auto whitespace-pre-wrap break-words text-[11px] leading-relaxed text-foreground/90">{setup.instructions}</p>
            <p className="m-0 text-[11px] text-muted-foreground">
              {setup.tester.username ? (
                <a href={`https://kryo.to/user/${encodeURIComponent(setup.tester.username)}`} onClick={go(`/user/${encodeURIComponent(setup.tester.username)}`)} className={linkClass}>
                  <TeamName username={setup.tester.username}>{setup.tester.name}</TeamName>
                </a>
              ) : (
                <span className="text-foreground">{setup.tester.name}</span>
              )}
              {how ? `, ${how}` : ''}
              {setup.version ? `, build ${setup.version}` : ''}
              {setup.current ? '' : ' (an older build)'}
            </p>
          </div>
        </details>
      ) : null}
    </div>
  )
}
