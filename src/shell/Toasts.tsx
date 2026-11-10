import { useCallback, useEffect, useState, type ReactNode } from 'react'
import { Play } from '@/ui/pixel-icons'
import { Matrix } from '@/ui'
import { on } from '@/lib/bridge'
import type { Download, DownloadStatus, Notice } from '@/lib/downloads'
import type { ChatMessage } from '@/lib/chat'
import { notify } from '@/lib/notify'
import { play } from '@/lib/sound'
import type { SettingsTarget } from '@/lib/settings-links'
import { cn } from '@/lib/utils'

/** A button beside the notice: Undo, for one that confirms something the player can take back. */
export type ToastAction = { label: string; run: () => unknown }

export type Toast = Notice & {
  key: number
  controllerGuid?: string
  action?: ToastAction
  /** A second answer beside `action`, for a notice that asks yes or no. */
  decline?: ToastAction
  /** The setting the notice is about: clicking it opens there (lib/settings-links.ts). */
  settings?: SettingsTarget
}

export type ToastInput = Omit<Toast, 'key'>

/** Long enough to read the notice, notice the button and reach it. */
const ACTION_MS = 10000

/** Notices from Rust (`notify`) and from the shell, each for seven seconds. */
export function useToasts() {
  const [toasts, setToasts] = useState<Toast[]>([])
  const push = useCallback((n: ToastInput) => {
    const key = Date.now() + Math.random()
    setToasts((t) => [...t.slice(-2), { ...n, key }])
    setTimeout(() => setToasts((t) => t.filter((x) => x.key !== key)), n.controllerGuid || n.decline ? 15000 : n.action ? ACTION_MS : 7000)
  }, [])
  const dismiss = useCallback((key: number) => setToasts((t) => t.filter((x) => x.key !== key)), [])
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    // The app's news (a game ready to play, an add-on applied): in the app,
    // and from the system too when the window is not in front.
    void on<Notice>('notify', (n) => {
      push(n)
      if (!document.hasFocus()) void notify(n.title, n.body)
      // In front, the optional sound instead (lib/sound.ts).
      else play(n.sound ?? 'notify')
    }).then((fn) => (cancelled ? fn() : (stop = fn)))
    // The system could not show a notification (Windows without its
    // notification platform, a Linux desktop with no notification service):
    // the app's own toast, quietly, instead of nothing at all.
    let stopFallback: (() => void) | undefined
    void on<Notice>('notify-fallback', (n) => push(n)).then((fn) => (cancelled ? fn() : (stopFallback = fn)))
    return () => {
      cancelled = true
      stop?.()
      stopFallback?.()
    }
  }, [push])
  useDownloadSounds()
  useChatSound()
  return { toasts, push, dismiss }
}

/**
 * A download starting and a download failing, from the live list: each only
 * the first time a download reaches that state, never for the list as it
 * stands when the app opens.
 */
function useDownloadSounds() {
  useEffect(() => {
    let seen: Map<string, DownloadStatus> | null = null
    let stop: (() => void) | undefined
    let cancelled = false
    void on<Download[]>('downloads', (list) => {
      const before = seen
      seen = new Map(list.map((d) => [d.id, d.status]))
      if (!before) return
      for (const d of list) {
        const was = before.get(d.id)
        if (was === d.status || d.addon) continue
        if (d.status === 'downloading' && (was === undefined || was === 'queued' || was === 'resolving')) play('downloadStart')
        else if (d.status === 'failed') play('downloadFailed')
      }
    }).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])
}

/** A chat message from somebody else. */
function useChatSound() {
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void on<ChatMessage>('chat-message', (m) => {
      if (!m.outgoing) play('message')
    }).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])
}

/**
 * Corner pop-ups, the site's toast: a card with a white badge. Only drawn
 * where nothing native covers the corner; over the Store the footer carries
 * the latest one instead. `children` sit above them in the same corner.
 */
export function Toasts({ toasts, onOpen, onDismiss, children }: { toasts: Toast[]; onOpen: (t: Toast) => void; onDismiss: (key: number) => void; children?: ReactNode }) {
  return (
    <div aria-live="polite" className="fixed bottom-14 right-4 z-[600] grid justify-items-end gap-2">
      {children}
      {toasts.map((t) => (
        <div key={t.key} className="kryo-toast kryo-radius flex w-80 items-stretch overflow-hidden border border-border bg-card shadow-2xl shadow-black/60">
          <button
            type="button"
            onClick={() => {
              onDismiss(t.key)
              onOpen(t)
            }}
            className="grid min-w-0 grow grid-cols-[36px_1fr] items-center gap-3 p-3 text-left hover:bg-secondary/50"
          >
            <span className="kryo-pill grid size-9 place-items-center bg-primary text-primary-foreground">
              {t.gameId ? <Play className="size-4" /> : <Matrix state="info" className="size-3.5" />}
            </span>
            <span className="min-w-0">
              <span className="line-clamp-2 text-xs font-bold text-foreground [overflow-wrap:anywhere]">{t.title}</span>
              <span className="line-clamp-3 text-[11px] leading-snug text-muted-foreground [overflow-wrap:anywhere]">{t.body}</span>
            </span>
          </button>
          {t.action && t.decline ? (
            <span className="grid shrink-0 grid-rows-2 border-l border-border">
              <ToastActionButton toast={t} onDismiss={onDismiss} className="px-3" />
              <ToastActionButton toast={t} which="decline" onDismiss={onDismiss} className="border-t border-border px-3 text-muted-foreground" />
            </span>
          ) : t.action ? (
            <ToastActionButton toast={t} onDismiss={onDismiss} className="border-l border-border px-3" />
          ) : null}
        </div>
      ))}
    </div>
  )
}

/** The notice's Undo (or other action, or its No): runs it and puts the notice away. */
export function ToastActionButton({
  toast,
  onDismiss,
  className,
  which = 'action',
}: {
  toast: Toast
  onDismiss: (key: number) => void
  className?: string
  which?: 'action' | 'decline'
}) {
  const action = which === 'decline' ? toast.decline : toast.action
  if (!action) return null
  const { label, run } = action
  return (
    <button
      type="button"
      onClick={() => {
        onDismiss(toast.key)
        void Promise.resolve().then(run).catch(() => {})
      }}
      className={cn('shrink-0 text-[10px] font-bold uppercase tracking-wider text-foreground hover:bg-secondary', className)}
    >
      {label}
    </button>
  )
}
