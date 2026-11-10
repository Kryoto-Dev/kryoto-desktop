import { useState } from 'react'
import { Button, Check, Section } from '@/ui'
import { errorText } from '@/lib/bridge'
import { pickPath } from '@/lib/pick'
import type { Settings } from '@/lib/settings'

/**
 * Settings > Downloads: game updates. Automatic updates are opt-in, with the
 * reason they are not on by default said out loud, and the backup folder that
 * makes an update undoable (src-tauri game_update.rs).
 */
export function GameUpdatesSection({
  settings: s,
  set,
}: {
  settings: Settings
  set: <K extends keyof Settings>(k: K, v: Settings[K]) => void
}) {
  const [error, setError] = useState<string | null>(null)

  async function choose() {
    setError(null)
    try {
      const dir = await pickPath({ title: 'Back up game updates to', directory: true, defaultPath: s.updateBackupDir || undefined })
      if (typeof dir === 'string' && dir) set('updateBackupDir', dir)
    } catch (e) {
      setError(errorText(e))
    }
  }

  return (
    <Section
      title="Game updates"
      hint="A new build can stop working with its emulator setup, a fix or a mod made for the old one, so this is off unless you turn it on. Kept builds (Properties > Builds) are never updated."
    >
      <Check checked={s.autoUpdateGames} onChange={(v) => set('autoUpdateGames', v)} label="Update my games automatically" />
      <div className="grid gap-1.5">
        <p className="text-xs leading-relaxed text-muted-foreground">
          Before an update replaces any file, Kryoto can copy the old ones to a folder of your choice. If the update fails or is
          interrupted, they are put back; after it, the game page offers Undo update. Saves and settings the update does not ship
          are never touched.
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" variant="outline" onClick={() => choose()}>
            {s.updateBackupDir ? 'Change folder' : 'Choose a backup folder'}
          </Button>
          {s.updateBackupDir ? (
            <>
              <span className="kryo-ascii-art min-w-0 select-text truncate text-[11px] text-foreground">{s.updateBackupDir}</span>
              <Button size="sm" variant="ghost" onClick={() => set('updateBackupDir', '')}>
                No backups
              </Button>
            </>
          ) : (
            <span className="text-[11px] text-muted-foreground">No backups: an update cannot be undone.</span>
          )}
        </div>
        {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
      </div>
    </Section>
  )
}
