import { Bell, Download, FileArchive, Play, Plus, Users } from '@/ui/pixel-icons'
import { asciiTrack } from '@/ui'
import { formatSpeed, isWorking, phaseOf, progressOf, type Download as Dl } from '@/lib/downloads'
import { useSettings } from '@/lib/settings'
import { ToastActionButton, type Toast } from '@/shell/Toasts'
import { useSocialBadge } from '@/hooks/useSocialBadge'

const chip =
  'kryo-pill flex h-7 items-center gap-2 px-3 text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground'

/**
 * The footer: Add a Game and Install from a file, the download in progress drawn as the site's ASCII
 * bar (click for Downloads), the latest notice while the Store covers the
 * corner, and Friends & Chat.
 */
export function BottomBar({
  downloads,
  notice,
  onNotice,
  onDismissNotice,
  onAddGame,
  onInstallFile,
  onDownloads,
  onFriends,
  friendsActive = false,
}: {
  downloads: Dl[]
  notice: Toast | null
  onNotice: (t: Toast) => void
  onDismissNotice: (key: number) => void
  onAddGame: () => void
  /** A game's .7z already on this PC (downloads/InstallFromFile.tsx). */
  onInstallFile: () => void
  onDownloads: () => void
  /** None for an account friends and chat are not on for. */
  onFriends?: () => void
  friendsActive?: boolean
}) {
  const waitingSocial = useSocialBadge().total
  const bits = !!useSettings()?.speedInBits
  const active = downloads.find(isWorking)
  const count = (status: Dl['status']) => downloads.filter((d) => d.status === status).length
  // A row stopped only for a sign-in carries on by itself once there is one
  // (src-tauri downloads.rs `signed_out`): waiting, not failed.
  const forSignIn = downloads.filter((d) => d.status === 'failed' && /once you are signed in|Sign in and it carries on/i.test(d.error ?? '')).length
  const waiting = [`${count('failed') - forSignIn} failed`, `${forSignIn} waiting for a sign-in`, `${count('paused')} paused`, `${count('queued')} queued`].filter((s) => !s.startsWith('0 '))
  return (
    <footer className="grid h-10 shrink-0 grid-cols-[1fr_auto_1fr] items-center border-t border-border bg-background px-2">
      <div className="flex">
        <button type="button" className={chip} onClick={onAddGame}>
          <Plus className="size-3" />
          Add a game
        </button>
        <button type="button" className={chip} onClick={onInstallFile} title="A game's .7z you already downloaded, from kryo.to or a mirror">
          <FileArchive className="size-3" />
          Install from a file
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
            {/* A fixed width: the whole group is centred, so a figure changing length would slide it. */}
            <span className="w-32 text-left tabular-nums">
              {Math.round(progressOf(active) * 100)}%
              {active.status === 'downloading' && active.speed ? ` · ${formatSpeed(active.speed, bits)}` : ''}
            </span>
          </>
        ) : (
          <>
            <Download className="size-3" />
            {['Downloads', ...waiting].join(' · ')}
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
        {notice?.action ? <ToastActionButton toast={notice} onDismiss={onDismissNotice} className="kryo-pill h-7 px-3" /> : null}
        {notice?.decline ? <ToastActionButton toast={notice} which="decline" onDismiss={onDismissNotice} className="kryo-pill h-7 px-3 text-muted-foreground" /> : null}
        {onFriends ? <button
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
        </button> : null}
      </div>
    </footer>
  )
}
