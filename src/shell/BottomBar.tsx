import { Bell, Download, Play, Plus, Users } from 'lucide-react'
import { asciiTrack } from '@/ui'
import { formatBytes, isWorking, phaseOf, progressOf, type Download as Dl } from '@/lib/downloads'
import type { Toast } from '@/shell/Toasts'
import { useSocialBadge } from '@/hooks/useSocialBadge'

const chip =
  'kryo-pill flex h-7 items-center gap-2 px-3 text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground'

/**
 * The footer: Add a Game, the download in progress drawn as the site's ASCII
 * bar (click for Downloads), the latest notice while the Store covers the
 * corner, and Friends & Chat.
 */
export function BottomBar({
  downloads,
  notice,
  onNotice,
  onAddGame,
  onDownloads,
  onFriends,
  friendsActive = false,
}: {
  downloads: Dl[]
  notice: Toast | null
  onNotice: (t: Toast) => void
  onAddGame: () => void
  onDownloads: () => void
  onFriends: () => void
  friendsActive?: boolean
}) {
  const waitingSocial = useSocialBadge().total
  const active = downloads.find(isWorking)
  const waiting = downloads.filter((d) => d.status === 'queued' || d.status === 'paused').length
  return (
    <footer className="grid h-10 shrink-0 grid-cols-[1fr_auto_1fr] items-center border-t border-border bg-background px-2">
      <div className="flex">
        <button type="button" className={chip} onClick={onAddGame}>
          <Plus className="size-3" />
          Add a game
        </button>
      </div>

      <button type="button" onClick={onDownloads} className={`${chip} min-w-72 justify-center`} aria-label="Downloads">
        {active ? (
          <>
            <span className="max-w-40 truncate text-foreground">
              {active.status === 'downloading' ? active.meta.title : phaseOf(active)}
            </span>
            <span aria-hidden className="kryo-ascii-art text-[11px] tracking-normal text-foreground">
              {asciiTrack(progressOf(active), 16)}
            </span>
            <span className="tabular-nums">
              {Math.round(progressOf(active) * 100)}%
              {active.status === 'downloading' && active.speed ? ` · ${formatBytes(active.speed)}/s` : ''}
            </span>
          </>
        ) : (
          <>
            <Download className="size-3" />
            {waiting ? `Downloads · ${waiting} paused` : 'Downloads'}
          </>
        )}
      </button>

      <div className="flex justify-end gap-1">
        {notice ? (
          <button
            type="button"
            onClick={() => onNotice(notice)}
            className="kryo-pill kryo-toast flex h-7 max-w-80 items-center gap-2 bg-primary px-3 text-[10px] font-bold uppercase tracking-wider text-primary-foreground"
          >
            {notice.gameId ? <Play className="size-3" /> : <Bell className="size-3" />}
            <span className="truncate">{notice.title}</span>
          </button>
        ) : null}
        <button
          type="button"
          className={`${chip} ${friendsActive ? 'bg-secondary text-foreground' : ''}`}
          aria-current={friendsActive ? 'page' : undefined}
          aria-label={waitingSocial ? `Friends & chat, ${waitingSocial} new` : 'Friends & chat'}
          onClick={onFriends}
        >
          Friends &amp; chat
          {waitingSocial ? (
            <span className="kryo-pill bg-primary px-1.5 text-[10px] font-bold tabular-nums tracking-normal text-primary-foreground" aria-hidden>
              {waitingSocial > 99 ? '99+' : waitingSocial}
            </span>
          ) : (
            <Users className="size-3" />
          )}
        </button>
      </div>
    </footer>
  )
}
