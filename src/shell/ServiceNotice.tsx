import { Matrix } from '@/ui'
import { namedParts, type ServiceHealth } from '@/lib/health'
import { openExternal } from '@/lib/window'

/**
 * One quiet line above the footer while Kryoto is not all there. It is outside
 * the Store's web view, so it shows on every tab, and it goes when the problem
 * does. What it promises is what catches up by itself (health.rs, Shell).
 */
export function ServiceNotice({ health, store }: { health: ServiceHealth; store: boolean }) {
  if (health.state === 'ok') return null
  const lead =
    health.state === 'partial'
      ? `${namedParts(health.parts)} ${health.parts.length > 1 ? 'are' : 'is'} having problems right now.`
      : health.state === 'down'
        ? 'Kryoto is down.'
        : 'Kryoto is not answering.'
  const rest =
    health.state === 'partial'
      ? ''
      : store
        ? ' The Store comes back by itself, on the page you were on.'
        : ' Some things will not sync for now, and catch up by themselves when it is back.'
  return (
    <div role="status" className="flex h-7 shrink-0 items-center justify-center gap-2 border-t border-border bg-background px-3 text-[11px] text-muted-foreground">
      <Matrix state="connect" className="size-3 shrink-0" />
      <span className="truncate">
        {lead}
        {rest}
      </span>
      {health.statusUp ? (
        <button
          type="button"
          onClick={() => void openExternal(health.statusUrl)}
          className="kryo-pill shrink-0 px-1.5 text-foreground/80 underline underline-offset-2 hover:text-foreground"
        >
          Status page
        </button>
      ) : null}
    </div>
  )
}
