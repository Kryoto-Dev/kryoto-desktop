import { FlaskConical } from '@/ui/pixel-icons'
import { useSettings } from '@/lib/settings'
import { cn } from '@/lib/utils'

/**
 * Says so whenever the app talks to a site other than kryo.to (Settings >
 * Developer > Site origin). Shown on the boot boxes and in the title bar, so a
 * local endpoint left behind is never mistaken for kryo.to being down.
 */
export function DevEndpointNotice({ className, compact = false }: { className?: string; compact?: boolean }) {
  const endpoint = useSettings()?.catalogEndpoint.trim()
  if (!endpoint) return null
  // The client's one row has no room for the sentence: the address is its tooltip.
  if (compact) {
    return (
      <p title={`Developer mode, using ${endpoint}`} className={cn('kryo-pill flex h-6 items-center gap-1 border border-warning/60 bg-warning/15 px-2 text-[10px] font-bold uppercase tracking-wider text-warning', className)}>
        <FlaskConical className="size-3" aria-hidden />
        Dev
      </p>
    )
  }
  return (
    <p
      className={cn(
        'pointer-events-none kryo-pill flex items-center gap-1.5 border border-warning/60 bg-warning/15 px-2.5 py-1 text-[10px] uppercase tracking-wider text-warning',
        className,
      )}
    >
      <FlaskConical className="size-3" aria-hidden />
      <span className="font-bold">Developer mode</span>
      <span className="normal-case tracking-normal text-foreground/80">using {endpoint}</span>
    </p>
  )
}
