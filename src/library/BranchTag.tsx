import { Folder, Gamepad2, GitBranch } from '@/ui/pixel-icons'
import { launcherName } from '@/lib/launchers'
import { cn } from '@/lib/utils'
import type { LibraryGame } from '@/lib/library'

/**
 * A Steam branch's name the way people say it: `public-beta` is "Public beta",
 * `gb27` is "GB27". Mirrored in `src-tauri/src/library.rs` (`branch_label`).
 */
export function branchName(branch: string): string {
  return branch
    .trim()
    .split(/[-_\s]+/)
    .filter(Boolean)
    .map((w, i) => (/\d/.test(w) && w.length <= 6 ? w.toUpperCase() : i === 0 ? w[0]!.toUpperCase() + w.slice(1).toLowerCase() : w.toLowerCase()))
    .join(' ')
}

/**
 * What tells a branch apart, or null for the public one and for a title that
 * already names its branch. The same rule as kryo.to's `lib/steam-branch.ts`.
 */
export function branchLabel(branch: string | null | undefined, title?: string | null): string | null {
  const name = branch?.trim()
  if (!name || name.toLowerCase() === 'public') return null
  const plain = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim()
  if (title && ` ${plain(title)} `.includes(` ${plain(name)} `)) return null
  return branchName(name)
}

/** The name with its branch, for plain-text places: "GoreBox (Public beta branch)". */
export function titleWithBranch(title: string, branch: string | null | undefined): string {
  const label = branchLabel(branch, title)
  return label ? `${title} (${label} branch)` : title
}

/**
 * Another entry in the Library with the same name and the same branch (a second
 * install of one game): this one's folder, which is what tells them apart.
 */
export function copyLabel(game: Pick<LibraryGame, 'id' | 'title' | 'steamBranch' | 'installDir'>, games: Pick<LibraryGame, 'id' | 'title' | 'steamBranch'>[]): string | null {
  const key = (g: Pick<LibraryGame, 'title' | 'steamBranch'>) => `${g.title.trim().toLowerCase()}\n${branchLabel(g.steamBranch) ?? ''}`
  const mine = key(game)
  if (!games.some((g) => g.id !== game.id && key(g) === mine)) return null
  return game.installDir.split(/[\\/]+/).filter(Boolean).pop() ?? null
}

/**
 * Which Steam branch a game is, when it is not the public one.
 *
 * kryo.to lists a beta of a Steam game as a game of its own (GoreBox27 is
 * GoreBox's `gb27` branch). Both have the same Steam art, so side by side in the
 * Library they looked like the same game twice. Nothing for the public branch.
 *
 * Or, for a game another launcher keeps (Bridge), that launcher; or, for a
 * second copy of the same game, the folder this one is in (`copy`).
 */
export function BranchTag({
  branch,
  launcher,
  title,
  copy,
  className,
}: {
  branch: string | null | undefined
  launcher?: string | null
  /** The game's name: a branch it already names is not repeated. */
  title?: string
  /** From `copyLabel`. */
  copy?: string | null
  className?: string
}) {
  const keeper = launcherName(launcher)
  const label = keeper ?? branchLabel(branch, title)
  const pill = 'kryo-pill inline-flex max-w-full shrink-0 items-center gap-1 border px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider'
  if (!label && !copy) return null
  return (
    <>
      {label ? (
        <span
          title={keeper ? `Starts through ${keeper}` : `Steam's ${branch?.trim()} branch: a different build from the main game`}
          className={cn(pill, keeper ? 'border-border bg-background/80 text-muted-foreground' : 'border-foreground/50 bg-background/90 text-foreground', className)}
        >
          {keeper ? <Gamepad2 className="size-3 shrink-0" aria-hidden /> : <GitBranch className="size-3 shrink-0" aria-hidden />}
          {keeper ? label : <span className="truncate">{label}</span>}
          {keeper ? null : <span className="sr-only"> branch</span>}
        </span>
      ) : null}
      {copy ? (
        <span title={`Another copy of this game is in your Library. This one is in the folder ${copy}.`} className={cn(pill, 'max-w-40 border-foreground/50 bg-background/90 text-foreground', className)}>
          <Folder className="size-3 shrink-0" aria-hidden />
          <span className="truncate normal-case tracking-normal">{copy}</span>
        </span>
      ) : null}
    </>
  )
}
