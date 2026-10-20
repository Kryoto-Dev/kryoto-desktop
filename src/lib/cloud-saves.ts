import { useCallback, useEffect, useRef, useState } from 'react'
import { call, on } from '@/lib/bridge'

/** Mirrors `src-tauri/src/saves/mod.rs` and `saves/api.rs`. */

export type Allowance = { totalBytes: number; versionsPerGame: number; maxBackupBytes: number }
export type Usage = { bytes: number; games: number; versions: number }

export type CloudSavesStatus = {
  /** kryo.to has cloud saves on for this account (feature flag). */
  feature: boolean
  /** Settings has them on. */
  on: boolean
  perk: boolean | null
  allowance: Allowance | null
  usage: Usage | null
  lastSync: number | null
  conflicts: string[]
}

export type SaveSource = 'player' | 'emulator' | 'engine' | 'known' | 'community' | 'staff' | 'detected'

export type SaveLocation = {
  key: string
  kind: 'files' | 'registry'
  root: string
  label: string
  source: SaveSource
  sourceLabel: string
  path: string | null
  exists: boolean
  files: number
  bytes: number
  score: number | null
  reasons: string[]
}

export type SaveVersion = {
  id: string
  size: number
  sha256: string
  md5: string
  fileCount: number
  machine: string | null
  createdAt: number
  parentId: string | null
}

export type GameSaveState = {
  baseVersion: string | null
  baseHash: string | null
  lastBackupAt: number | null
  lastRestoreAt: number | null
  cloudHead: SaveVersion | null
  conflict: boolean
  error: string | null
  tooBig: boolean
}

export type GameSaves = {
  identity: { key: string; aliases: string[]; appid: string | null }
  feature: boolean
  on: boolean
  perk: boolean | null
  locations: SaveLocation[]
  suggestions: SaveLocation[]
  files: number
  bytes: number
  truncated: boolean
  state: GameSaveState
  maxBackupBytes: number
  localBackups: { file: string; createdAt: number; size: number }[]
}

export type SavesPrompt = {
  gameId: string
  title: string
  kind: 'conflict' | 'restore'
  cloud: { machine: string | null; createdAt: number; size: number; files: number }
  local: { files: number; bytes: number; lastBackupAt: number | null; changedAt: number | null }
}

export const cloudSaves = {
  status: () => call<CloudSavesStatus>('cloud_saves_status'),
  refresh: () => call<CloudSavesStatus>('cloud_saves_refresh'),
  game: (id: string) => call<GameSaves>('cloud_saves_game', { id }),
  location: (id: string, key: string, action: 'keep' | 'ignore' | 'restore') => call<void>('cloud_saves_location', { id, key, action }),
  addFolder: (id: string, path: string) => call<void>('cloud_saves_add_folder', { id, path }),
  /** `over`: this PC's saves win a conflict. */
  backup: (id: string, over = false) => call<string>('cloud_saves_backup', { id, over }),
  versions: (id: string) => call<SaveVersion[]>('cloud_saves_versions', { id }),
  restore: (id: string, versionId: string) => call<string>('cloud_saves_restore', { id, versionId }),
  restoreLocal: (id: string, file: string) => call<string>('cloud_saves_restore_local', { id, file }),
  answer: (id: string, choice: 'cloud' | 'local' | 'later') => call<void>('cloud_saves_answer', { id, choice }),
  library: () =>
    call<{ id: string; title: string; lastBackupAt: number | null; conflict: boolean; error: string | null; tooBig: boolean }[]>('cloud_saves_library'),
}

/** The account's cloud-saves status, kept current. */
export function useCloudSavesStatus() {
  const [s, setS] = useState<CloudSavesStatus | null>(null)
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void cloudSaves.status().then((v) => !cancelled && setS(v)).catch(() => {})
    void on<CloudSavesStatus>('cloud-saves', (v) => !cancelled && setS(v)).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])
  return s
}

/** One game's saves, reloaded whenever the native side says they changed. */
export function useGameSaves(id: string) {
  const [data, setData] = useState<GameSaves | null>(null)
  const [error, setError] = useState<string | null>(null)
  const request = useRef(0)
  const load = useCallback(() => {
    const n = ++request.current
    void cloudSaves
      .game(id)
      .then((d) => {
        if (n !== request.current) return
        setData(d)
        setError(null)
      })
      .catch((e: unknown) => n === request.current && setError(String(e)))
  }, [id])
  useEffect(() => {
    setData(null)
    load()
    let stop: (() => void) | undefined
    let cancelled = false
    void on<{ gameId: string }>('cloud-saves-game', (e) => e.gameId === id && load()).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      request.current++
      stop?.()
    }
  }, [id, load])
  return { data, error, reload: load }
}

const MB = 1024 * 1024

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < MB) return `${Math.max(1, Math.round(n / 1024))} KB`
  if (n < 1024 * MB) return `${(n / MB).toFixed(n < 10 * MB ? 1 : 0)} MB`
  return `${(n / (1024 * MB)).toFixed(1)} GB`
}

export function formatAgo(unix: number | null | undefined): string {
  if (!unix) return 'never'
  const s = Math.max(0, Math.floor(Date.now() / 1000) - unix)
  if (s < 60) return 'just now'
  if (s < 3600) return `${Math.floor(s / 60)} min ago`
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`
  return new Date(unix * 1000).toLocaleDateString()
}
