import { Gamepad2, GitBranch } from '@/ui/pixel-icons'
import { launcherName } from '@/lib/launchers'
import { cn } from '@/lib/utils'

/**
 * Which Steam branch a game is, when it is not the public one.
 *
 * kryo.to lists a beta of a Steam game as a game of its own (GoreBox27 is
 * GoreBox's `gb27` branch). Both have the same Steam art, so side by side in the
 * Library they looked like the same game twice. Nothing for the public branch.
 *
 * Or, for a game another launcher keeps (Bridge), that launcher.
 */
export function BranchTag({ branch, launcher, className }: { branch: string | null | undefined; launcher?: string | null; className?: string }) {
  const keeper = launcherName(launcher)
  const name = keeper ?? branch?.trim()
  if (!name || name === 'public') return null
  const Icon = keeper ? Gamepad2 : GitBranch
  return (
    <span
      title={keeper ? `Starts through ${keeper}` : `Built from Steam's ${name} branch`}
      className={cn(
        'kryo-pill inline-flex shrink-0 items-center gap-1 border border-border bg-background/80 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-muted-foreground',
        className,
      )}
    >
      <Icon className="size-3" aria-hidden />
      {name}
    </span>
  )
}
