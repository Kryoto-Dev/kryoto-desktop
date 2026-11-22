import { useEffect, useRef, useState } from 'react'
import { downloadDir } from '@tauri-apps/api/path'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { ArrowLeft, FileArchive, FolderOpen, Library } from '@/ui/pixel-icons'
import { AsciiBar, Button, Modal } from '@/ui'
import { errorText, isTauri } from '@/lib/bridge'
import { pickPath } from '@/lib/pick'
import { downloads as api, formatBytes } from '@/lib/downloads'
import { ARCHIVE_EXTENSIONS, fileVerdict, pickDropped, type ArchiveLook } from '@/lib/from-file'
import { adultBlur, useShowAdult } from '@/lib/adult'
import { artSrc } from '@/lib/art'
import { cn } from '@/lib/utils'
import { GameSearch } from '@/library/AddGameDialog'

/**
 * The system's file picker for a game's archive, opened on the Downloads
 * folder, where a browser puts it. Rejects when it cannot open (KB-0042):
 * callers say so.
 */
export async function pickArchive(): Promise<string | null> {
  const defaultPath = isTauri() ? await downloadDir().catch(() => undefined) : undefined
  return pickPath({ title: "Pick the game's .7z file", extensions: ARCHIVE_EXTENSIONS, filterName: 'Game archive (.7z)', defaultPath })
}

/**
 * Install from a file: a game's archive the player already downloaded (from
 * kryo.to in a browser, or a mirror), installed from where it is instead of
 * downloaded again (src-tauri from_file.rs). The file's name says which game
 * when it can; else the player picks it. One press then queues it in
 * Downloads, where it is checked against kryo.to's copy and unpacked like any
 * download.
 */
export function InstallFromFile({
  path,
  slug,
  onClose,
  onStarted,
  onOpenCatalog,
}: {
  path: string
  /** The game, when the player came from its page. */
  slug: string | null
  onClose: () => void
  /** Queued: Downloads shows it from here. */
  onStarted: () => void
  /** An add-on whose game is not installed: that game's page, to install it first. */
  onOpenCatalog: (slug: string) => void
}) {
  const showAdult = useShowAdult()
  const [file, setFile] = useState(path)
  const [chosen, setChosen] = useState<string | null>(slug)
  const [look, setLook] = useState<ArchiveLook | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  // "Which game is this?": by itself when the name says nothing, or "Not this game?".
  const [searching, setSearching] = useState(false)
  const [query, setQuery] = useState('')
  const input = useRef<HTMLInputElement | null>(null)

  useEffect(() => {
    let live = true
    setLook(null)
    setError(null)
    api
      .look(file, chosen)
      .then((l) => {
        if (!live) return
        setLook(l)
        setSearching(!l.slug)
      })
      .catch((e: unknown) => live && setError(errorText(e)))
    return () => {
      live = false
    }
  }, [file, chosen])

  useEffect(() => {
    if (searching) requestAnimationFrame(() => input.current?.focus())
  }, [searching])

  const another = async () => {
    setError(null)
    try {
      const picked = await pickArchive()
      if (picked) {
        setFile(picked)
        setChosen(slug)
      }
    } catch (e) {
      setError(`The file picker did not open: ${errorText(e)}`)
    }
  }

  const verdict = look ? fileVerdict(look) : null
  const install = async (anyway: boolean) => {
    if (!look?.slug) return
    setBusy(true)
    setError(null)
    try {
      await api.installFile(look.path, look.slug, look.title, anyway)
      onStarted()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  const name = look?.fileName ?? file.split(/[\\/]/).pop() ?? file
  const facts = look ? [look.addon, look.addon ? null : look.version ? `build ${look.version}` : null, formatBytes(look.size)].filter(Boolean).join(' · ') : ''

  return (
    <Modal
      title="Install from a file"
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={() => void another()}>
            <FolderOpen className="size-3.5" />
            Another file
          </Button>
          {look && verdict?.kind === 'needs-game' && look.slug ? (
            <Button variant="primary" onClick={() => onOpenCatalog(look.slug!)}>
              <Library className="size-3.5" />
              Open {look.title ?? 'the game'}
            </Button>
          ) : null}
          {look && verdict?.kind === 'install' && !searching ? (
            <Button variant="primary" disabled={busy} onClick={() => void install(verdict.anyway)}>
              <FileArchive className="size-3.5" />
              {busy ? 'Adding' : verdict.button}
            </Button>
          ) : null}
        </>
      }
    >
      <p className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
        <FileArchive className="size-3.5 shrink-0" />
        <span className="truncate text-foreground" title={look?.path ?? file}>
          {name}
        </span>
      </p>
      {!look && !error ? (
        <div role="status" className="grid gap-1">
          <AsciiBar fraction={null} cells={18} showPct={false} className="text-muted-foreground" />
          <span className="text-xs text-muted-foreground">Looking at the file</span>
        </div>
      ) : null}
      {look && look.slug && !searching ? (
        <div className="flex gap-4">
          {look.cover ? (
            <img
              decoding="async"
              src={artSrc(look.cover) ?? undefined}
              alt=""
              className={cn('w-20 shrink-0 object-cover', adultBlur(look.nsfw, showAdult))}
              style={{ aspectRatio: '2 / 3', borderRadius: 'min(var(--kryo-radius), 8px)' }}
            />
          ) : null}
          <div className="grid content-center gap-1">
            <b className="text-base text-foreground">{look.title ?? look.slug}</b>
            <span className="text-xs text-muted-foreground">{facts}</span>
            <p className="text-xs leading-relaxed text-muted-foreground">{verdict?.line}</p>
            <button
              type="button"
              onClick={() => {
                setQuery(look.title ?? '')
                setSearching(true)
              }}
              className="kryo-square w-fit text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
            >
              Not this game?
            </button>
          </div>
        </div>
      ) : null}
      {look && searching ? (
        <>
          <p className="text-xs leading-relaxed text-muted-foreground">
            {verdict?.kind === 'pick' ? verdict.line : 'Which game is this?'}
          </p>
          <GameSearch
            inputRef={input}
            query={query}
            onQuery={setQuery}
            onPick={(picked) => {
              setSearching(false)
              if (picked === look.slug) return
              setChosen(picked)
            }}
          />
          {look.slug ? (
            <Button variant="ghost" size="sm" className="w-fit" onClick={() => setSearching(false)}>
              <ArrowLeft className="size-3" />
              Back
            </Button>
          ) : null}
        </>
      ) : null}
      {error ? (
        <p role="alert" className="text-xs leading-relaxed text-destructive">
          {error}
        </p>
      ) : null}
      <p className="text-[11px] text-muted-foreground">Your file stays where it is. Kryoto only reads it.</p>
    </Modal>
  )
}

/**
 * Drop an archive on the Library or Downloads: the same Install from a file.
 * Tauri hands the window's file drops over as paths (`onDragDropEvent`, on
 * Windows, macOS and Linux alike); the page itself never sees the file. While
 * a file is dragged over, the area says what a drop does.
 */
export function ArchiveDrop({ enabled, onFile }: { enabled: boolean; onFile: (path: string) => void }) {
  const [over, setOver] = useState(false)
  const latest = useRef({ enabled, onFile })
  latest.current = { enabled, onFile }
  useEffect(() => {
    if (!isTauri()) return
    let stop: (() => void) | undefined
    let cancelled = false
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload
        if (!latest.current.enabled) {
          setOver(false)
          return
        }
        if (p.type === 'enter' || p.type === 'over') setOver(true)
        else if (p.type === 'leave') setOver(false)
        else if (p.type === 'drop') {
          setOver(false)
          const file = pickDropped(p.paths)
          if (file) latest.current.onFile(file)
        }
      })
      .then((fn) => {
        if (cancelled) fn()
        else stop = fn
      })
      .catch(() => {})
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])
  if (!over || !enabled) return null
  return (
    <div role="status" className="kryo-radius pointer-events-none absolute inset-3 z-40 grid place-items-center border-2 border-dashed border-foreground/60 bg-background/85">
      <div className="grid justify-items-center gap-2 px-6 text-center">
        <FileArchive className="size-8 text-foreground" />
        <b className="text-sm text-foreground">Drop to install from this file</b>
        <span className="text-xs text-muted-foreground">A game&apos;s .7z from kryo.to or a mirror. Your file stays where it is.</span>
      </div>
    </div>
  )
}
