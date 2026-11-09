import { useState } from 'react'
import { Cloud, FolderOpen, HardDrive, Server, X } from '@/ui/pixel-icons'
import { Button, IconButton, Segmented } from '@/ui'
import { errorText } from '@/lib/bridge'
import { pickPath } from '@/lib/pick'
import { settingsApi, useSettings, type KeptIn, type Settings } from '@/lib/settings'
import { openSettingsSection, openStorePage } from '@/lib/window'
import { library } from '@/lib/library'

/**
 * Where save versions and achievements are kept, for everyone: in a folder on
 * this PC, on a server of your own (Settings > Cloud, Patchbay), or on the kryo.to
 * account, which is the supporter perk. The same versions, restore and
 * keep-N in all three (src-tauri saves/local.rs, achievements_cloud.rs).
 *
 * Shown in Settings > Cloud and on a game's saves card.
 */

type What = 'saves' | 'achievements'

const LABEL: Record<KeptIn, string> = { local: 'On this PC', patchbay: 'My own server', account: 'kryo.to account' }

/** What Settings has now, with the blank of before there was a choice read the way the app reads it. */
export function keptIn(s: Settings | null, what: What): KeptIn {
  const mode = what === 'saves' ? s?.savesMode : s?.achievementsMode
  if (mode === 'local' && s?.savesLocalDir) return 'local'
  if (mode === 'account') return 'account'
  const endpoint = what === 'saves' ? s?.savesEndpoint : s?.achievementsEndpoint
  return endpoint ? 'patchbay' : 'account'
}

async function save(p: Partial<Settings>) {
  const latest = await settingsApi.get()
  return settingsApi.save({ ...latest, ...p })
}

export function SavePlace({ what, compact = false }: { what: What; compact?: boolean }) {
  const s = useSettings()
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  if (!s) return null
  const mode = keptIn(s, what)
  const key = what === 'saves' ? 'savesMode' : 'achievementsMode'
  const endpoint = what === 'saves' ? s.savesEndpoint : s.achievementsEndpoint

  const run = async (job: () => Promise<unknown>) => {
    setBusy(true)
    setError(null)
    try {
      await job()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  const chooseFolder = async (andUse: boolean) => {
    const dir = await pickPath({ title: 'A folder for your save versions and achievements', directory: true, defaultPath: s.savesLocalDir || undefined })
    if (typeof dir !== 'string') return
    await save({ savesLocalDir: dir, ...(andUse ? { [key]: 'local' as const } : {}) })
  }

  const pick = (next: KeptIn) =>
    void run(async () => {
      if (next === 'local' && !s.savesLocalDir) return chooseFolder(true)
      if (next === 'patchbay' && !endpoint) return openSettingsSection('patchbay')
      await save({ [key]: next })
    })

  return (
    <div className="grid gap-2">
      <Segmented
        label={what === 'saves' ? 'Where save versions are kept' : 'Where achievements are kept'}
        value={mode}
        options={(['local', 'patchbay', 'account'] as const).map((v) => ({ value: v, label: LABEL[v] }))}
        onChange={pick}
      />
      {mode === 'local' ? (
        <div className="flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
          <HardDrive className="size-3 shrink-0" aria-hidden />
          <span className="min-w-0 truncate text-foreground" title={s.savesLocalDir}>
            {s.savesLocalDir}
          </span>
          <Button size="sm" variant="ghost" disabled={busy} onClick={() => run(() => chooseFolder(false))}>
            Change
          </Button>
          <IconButton label="Open folder" className="size-7" onClick={() => void library.openFolder(s.savesLocalDir ?? '', true)}>
            <FolderOpen className="size-3.5" />
          </IconButton>
        </div>
      ) : mode === 'patchbay' ? (
        <p className="flex items-center gap-2 text-[11px] text-muted-foreground">
          <Server className="size-3 shrink-0" aria-hidden />
          <span className="grow">On {hostOf(endpoint)}, set up in Patchbay.</span>
        </p>
      ) : compact ? null : (
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          On your kryo.to account, back on any PC you sign in on. A supporter perk.
        </p>
      )}
      {mode === 'local' && what === 'saves' && !compact ? (
        <Segmented
          label="Versions kept per game"
          value={String(s.savesLocalKeep ?? 10)}
          options={['5', '10', '20', '50'].map((v) => ({ value: v, label: `${v} versions` }))}
          onChange={(v) => void run(() => save({ savesLocalKeep: Number(v) }))}
        />
      ) : null}
      {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
    </div>
  )
}

function hostOf(endpoint: string): string {
  try {
    return new URL(endpoint).host
  } catch {
    return endpoint
  }
}

/**
 * kryo.to's own cloud saves, beside the free places: so players know they
 * exist if they support Kryoto. Closed once, closed for good.
 */
export function KryotoCloudCard() {
  const s = useSettings()
  if (!s || s.savesPromoClosed || (keptIn(s, 'saves') === 'account' && keptIn(s, 'achievements') === 'account')) return null
  return (
    <div className="kryo-radius flex items-start gap-3 border border-border bg-secondary/40 p-3">
      <Cloud className="mt-0.5 size-4 shrink-0 text-foreground" aria-hidden />
      <div className="grid grow gap-1.5">
        <p className="text-xs font-bold text-foreground">Real cloud saves on kryo.to</p>
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          Supporters keep their save versions and achievements on their kryo.to account, and get them back on any PC they sign in on. Nothing to set up.
        </p>
        <Button size="sm" className="w-fit" onClick={() => openStorePage('/perks#cloud')}>
          See the supporter perks
        </Button>
      </div>
      <IconButton label="Close this card for good" className="size-7 shrink-0" onClick={() => void save({ savesPromoClosed: true }).catch(() => {})}>
        <X className="size-3.5" />
      </IconButton>
    </div>
  )
}
