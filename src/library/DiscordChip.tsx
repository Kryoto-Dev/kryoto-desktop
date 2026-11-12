import { discordGames, discordWords, useDiscord } from '@/lib/discord'
import { openSettingsSection } from '@/lib/window'

const LINK = 'kryo-square text-foreground/80 underline-offset-4 hover:text-foreground hover:underline'

/**
 * One quiet line: what Discord shows for this game, and the switch for it.
 * The same on a game's page, in Properties and in Settings. Nothing for a game
 * another launcher keeps.
 */
export function DiscordChip({ id, named = false, stat = false }: { id: string; named?: boolean; stat?: boolean }) {
  const d = useDiscord()
  const g = d?.games.find((x) => x.id === id)
  if (!d || !g) return null
  const { text, tip } = discordWords(g, d.open)
  const action =
    g.why === 'off' ? (
      <button type="button" className={LINK} onClick={() => openSettingsSection('general', 'discord')}>
        Settings
      </button>
    ) : (
      <button type="button" className={LINK} title={g.why === 'adult' ? tip : undefined} onClick={() => void discordGames.set(id, !!g.why)}>
        {g.why ? 'Show' : 'Hide'}
      </button>
    )
  // On a game's page it sits among Last played and Play time, so it is drawn like
  // them: a small label over the value (KB-0303).
  if (stat) {
    return (
      <div className="grid min-w-0 gap-1" title={tip}>
        <span className="text-[10px] uppercase tracking-wider text-muted-foreground">Discord</span>
        <span className="flex min-w-0 items-baseline gap-2 text-sm text-foreground">
          {g.now ? <span className="kryo-blink size-1.5 shrink-0 self-center rounded-full bg-success" aria-hidden /> : null}
          <span className="truncate">{text.replace(/ (on|from) Discord/, '')}</span>
          <span className="text-[11px]">{action}</span>
        </span>
      </div>
    )
  }
  return (
    <span className="inline-flex min-w-0 items-center gap-2 text-[11px] text-muted-foreground" title={tip}>
      {g.now ? <span className="kryo-blink size-1.5 shrink-0 rounded-full bg-success" aria-hidden /> : null}
      {named ? <span className="truncate text-foreground">{g.title}</span> : null}
      <span className="truncate">{text}</span>
      {action}
    </span>
  )
}
