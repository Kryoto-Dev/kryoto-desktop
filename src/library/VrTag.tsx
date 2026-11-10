import { Glasses } from '@/ui/pixel-icons'
import { cn } from '@/lib/utils'
import type { VrKind } from '@/lib/library'

/**
 * A VR game, the way Steam marks one: "VR" for a game that also plays flat,
 * "VR only" for one that needs a headset. Nothing for a flat game.
 */
export function VrTag({ vr, className }: { vr: VrKind | null | undefined; className?: string }) {
  if (vr !== 'vr_supported' && vr !== 'vr_only') return null
  const only = vr === 'vr_only'
  return (
    <span
      title={only ? 'Needs a VR headset' : 'Plays in VR, or flat on the screen'}
      className={cn(
        'kryo-pill inline-flex shrink-0 items-center gap-1 border border-border bg-background/80 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-muted-foreground',
        className,
      )}
    >
      <Glasses className="size-3" aria-hidden />
      {only ? 'VR only' : 'VR'}
    </span>
  )
}
