import { useEffect, useState } from 'react'
import { AlertTriangle, ArrowRight, Cloud, Trophy } from 'lucide-react'
import { AsciiBar, Button, Caption, Check, Section, Segmented } from '@/ui'
import { call, errorText, on } from '@/lib/bridge'
import { cloudSaves, formatAgo, formatBytes, useCloudSavesStatus } from '@/lib/cloud-saves'
import type { Settings } from '@/lib/settings'
import { openStorePage } from '@/lib/window'

type AchievementsStatus = { perk: boolean | null; games: number; lastSync: number | null }

/**
 * Settings > Cloud: achievements and saves on the kryo.to account, side by
 * side. What is stored and how much room is left; how this PC syncs (its own
 * settings, here); and the way to what the account holds (kryo.to's own
 * settings, Support > Cloud), where both can be removed.
 */
export function CloudPane({ s, set, onManage }: { s: Settings; set: <K extends keyof Settings>(k: K, v: Settings[K]) => void; onManage: () => void }) {
  const status = useCloudSavesStatus()
  const [ach, setAch] = useState<AchievementsStatus | null>(null)
  const [games, setGames] = useState<Awaited<ReturnType<typeof cloudSaves.library>>>([])
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void call<AchievementsStatus>('cloud_achievements_status').then((v) => !cancelled && setAch(v)).catch(() => {})
    void on<AchievementsStatus>('cloud-achievements', (v) => !cancelled && setAch(v)).then((fn) => (cancelled ? fn() : (stop = fn)))
    void cloudSaves.library().then((g) => !cancelled && setGames(g)).catch(() => {})
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])

  // What the account holds and its room, asked for each time the page opens.
  useEffect(() => {
    if (status?.feature) void cloudSaves.refresh().catch((e: unknown) => setError(errorText(e)))
  }, [status?.feature])

  const feature = !!status?.feature
  const perk = status?.perk
  const used = status?.usage?.bytes ?? 0
  const total = status?.allowance?.totalBytes ?? 0
  const notable = games.filter((g) => g.conflict || g.error || g.tooBig || g.lastBackupAt)

  return (
    <>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="kryo-radius grid content-start gap-2 border border-border p-4">
          <span className="flex items-center gap-2 text-xs font-bold text-foreground">
            <Trophy className="size-3.5" aria-hidden />
            Achievements
          </span>
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            {ach?.perk === false
              ? 'A supporter perk: every achievement you unlock, kept on your account.'
              : ach?.games
                ? `${ach.games} game${ach.games === 1 ? '' : 's'} on your account · synced ${formatAgo(ach.lastSync)}`
                : 'Saved to your account as you unlock them.'}
          </p>
        </div>
        <div className="kryo-radius grid content-start gap-2 border border-border p-4">
          <span className="flex items-center gap-2 text-xs font-bold text-foreground">
            <Cloud className="size-3.5" aria-hidden />
            Saves
          </span>
          {!feature ? (
            <p className="text-[11px] leading-relaxed text-muted-foreground">Cloud saves are not on your account yet.</p>
          ) : perk === false ? (
            <p className="text-[11px] leading-relaxed text-muted-foreground">A supporter perk: your game saves on your account, back on any PC you play on.</p>
          ) : total ? (
            <>
              <AsciiBar fraction={Math.min(1, used / total)} cells={24} />
              <Caption>
                {formatBytes(used)} of {formatBytes(total)} · {status?.usage?.games ?? 0} games · synced {formatAgo(status?.lastSync)}
              </Caption>
            </>
          ) : (
            <p className="text-[11px] text-muted-foreground">{error ?? 'Checking your account…'}</p>
          )}
        </div>
      </div>

      {perk === false || ach?.perk === false ? (
        <Button size="sm" variant="primary" className="justify-self-start" onClick={() => openStorePage('/donate')}>
          Become a supporter
        </Button>
      ) : null}

      {feature ? (
        <>
          <Section title="Cloud saves" hint="Kryoto Desktop backs a game's saves up when it closes, and puts newer saves from your other PCs back before it starts. What a restore replaces is kept on this PC first.">
            <Check checked={s.cloudSaves} onChange={(v) => set('cloudSaves', v)} label="Sync game saves with my kryo.to account" />
            <Check checked={s.cloudSavesOnExit} disabled={!s.cloudSaves} onChange={(v) => set('cloudSavesOnExit', v)} label="Back up a game's saves when it closes" />
          </Section>
          <Section title="Newer saves from another PC">
            <Segmented
              label="Newer saves from another PC"
              value={s.cloudSavesRestore}
              options={[
                { value: 'auto', label: 'Put them here' },
                { value: 'ask', label: 'Ask me first' },
              ]}
              onChange={(v) => set('cloudSavesRestore', v)}
            />
          </Section>
          <Section
            title="Finding saves"
            hint="Known locations come from PCGamingWiki (through the Ludusavi project), the game's emulator and engine, and other players. For the rest, Kryoto Desktop watches the game while it runs. Shared locations are folder names only, never your files or your user name."
          >
            <Check checked={s.cloudSavesDetect} disabled={!s.cloudSaves} onChange={(v) => set('cloudSavesDetect', v)} label="Watch games while they run to find their saves" />
            <Segmented
              label="How to find saves"
              value={s.cloudSavesFind}
              options={[
                { value: 'precise', label: 'Precise' },
                { value: 'smart', label: 'Smart' },
              ]}
              onChange={(v) => set('cloudSavesFind', v)}
            />
            <p className="text-[11px] leading-relaxed text-muted-foreground">
              {s.cloudSavesFind === 'precise'
                ? 'The first time you play a game, Windows asks you once to let Kryoto Desktop see exactly which files that game writes. Say no and Smart is used for that game.'
                : 'No administrator needed. A folder counts only when the game had files open in it, or it is named after the game, its developer, or in the game’s own files.'}
            </p>
            <Check checked={s.cloudSavesShare} disabled={!s.cloudSaves || !s.cloudSavesDetect} onChange={(v) => set('cloudSavesShare', v)} label="Share the save folders found, so other players' games find theirs" />
          </Section>
          <Section title="Backups kept on this PC" hint="Each game keeps this many copies of saves a restore replaced.">
            <Segmented
              label="Local backups per game"
              value={String(s.cloudSavesKeepLocal)}
              options={['3', '5', '10'].map((v) => ({ value: v, label: v }))}
              onChange={(v) => set('cloudSavesKeepLocal', Number(v))}
            />
          </Section>
          {notable.length ? (
            <Section title="Games">
              <ul className="kryo-radius divide-y divide-border overflow-hidden border border-border">
                {notable.slice(0, 50).map((g) => (
                  <li key={g.id} className="flex items-center justify-between gap-3 p-3 text-xs">
                    <span className="truncate text-foreground">{g.title}</span>
                    <span className={g.conflict || g.error || g.tooBig ? 'flex shrink-0 items-center gap-1.5 text-destructive' : 'shrink-0 text-muted-foreground'}>
                      {g.conflict || g.error || g.tooBig ? <AlertTriangle className="size-3" aria-hidden /> : null}
                      {g.conflict ? 'Pick which saves to keep on its page' : g.tooBig ? 'Too big to back up' : g.error ? g.error : `Backed up ${formatAgo(g.lastBackupAt)}`}
                    </span>
                  </li>
                ))}
              </ul>
            </Section>
          ) : null}
        </>
      ) : null}

      <Button size="sm" className="justify-self-start" onClick={onManage}>
        What is on my account
        <ArrowRight className="size-3" aria-hidden />
      </Button>
    </>
  )
}
