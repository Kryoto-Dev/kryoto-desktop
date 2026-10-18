import { useState } from 'react'
import { Glasses, Play } from 'lucide-react'
import { Button, Check, Modal } from '@/ui'
import { startEntry, waysToPlay, type LibraryGame } from '@/lib/library'
import { cn } from '@/lib/utils'

/**
 * "How do you want to play?" - for a player who chose to be asked each time
 * (Properties > When you press Play). The plain way to start the game is first;
 * a VR mode, a config tool or a launch flag are under it. Pre-selects what Play
 * would start. "Don't ask again" saves the pick; Properties undoes it.
 */
export function LaunchChooser({
  game,
  onPlay,
  onClose,
}: {
  game: LibraryGame
  /** `entry` null is the game's own exe. */
  onPlay: (entry: number | null, remember: boolean) => void
  onClose: () => void
}) {
  const ways = waysToPlay(game)
  const [picked, setPicked] = useState<number | null>(startEntry(game))
  const [remember, setRemember] = useState(false)
  return (
    <Modal
      title={`Play ${game.title}`}
      onClose={onClose}
      footer={
        <>
          <span className="grow">
            <Check checked={remember} onChange={setRemember} label="Don't ask again" />
          </span>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={() => onPlay(picked, remember)}>
            <Play className="size-3.5 fill-current" />
            Play
          </Button>
        </>
      }
    >
      <p className="text-xs leading-relaxed text-muted-foreground">This game starts more than one way. Pick how you want to play.</p>
      <div role="radiogroup" aria-label="How to play" className="grid gap-2">
        {ways.map((w) => (
          <button
            key={w.entry ?? 'normal'}
            type="button"
            role="radio"
            aria-checked={picked === w.entry}
            onClick={() => setPicked(w.entry)}
            onDoubleClick={() => onPlay(w.entry, remember)}
            className={cn(
              'flex items-center gap-3 border p-3 text-left transition-colors',
              picked === w.entry ? 'border-foreground bg-secondary' : 'border-border hover:border-foreground/50',
            )}
          >
            <span
              className={cn(
                'kryo-pill grid size-9 shrink-0 place-items-center',
                picked === w.entry ? 'bg-primary text-primary-foreground' : 'bg-secondary text-muted-foreground',
              )}
            >
              {w.vr ? <Glasses className="size-4" /> : <Play className="size-4" />}
            </span>
            <span className="grid min-w-0 gap-0.5">
              <span className="text-sm font-bold text-foreground">
                {w.label}
                {w.vr ? (
                  <span className="kryo-pill ml-2 border border-border px-1.5 py-px align-middle text-[9px] uppercase tracking-wider text-muted-foreground">
                    headset
                  </span>
                ) : null}
              </span>
              <span className="kryo-ascii-art truncate text-[11px] text-muted-foreground">{w.detail}</span>
            </span>
          </button>
        ))}
      </div>
    </Modal>
  )
}
