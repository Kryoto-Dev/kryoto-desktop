import { useEffect, useState } from 'react'
import { call, on } from '@/lib/bridge'

/**
 * What Discord shows for each game, from one place (src-tauri discord_rpc/).
 * The game page, Properties, Settings and the Manage menu all read this.
 */

export type DiscordGame = {
  id: string
  title: string
  /** Why it is not shown as itself. */
  why: 'off' | 'game' | 'hidden' | 'adult' | null
  playing: boolean
  /** It is on Discord this moment. */
  now: boolean
  /** The player chose something for this game that differs from Settings. */
  set: boolean
}

export type DiscordOverview = { on: boolean; open: boolean; games: DiscordGame[] }

let current: DiscordOverview | null = null
const subscribers = new Set<(v: DiscordOverview) => void>()

function load() {
  void call<DiscordOverview>('discord_games_status')
    .then((v) => {
      current = v
      subscribers.forEach((s) => s(v))
    })
    .catch(() => {})
}

let listening = false
function listen() {
  if (listening) return
  listening = true
  void on('discord-status', load)
  void on('library-changed', load)
}

export const discordGames = {
  /** Show or hide one game, wherever the player flipped it. */
  set: (id: string, show: boolean) => call<void>('discord_game_set', { id, show }).then(load),
}

export function useDiscord(): DiscordOverview | null {
  const [v, setV] = useState(current)
  useEffect(() => {
    listen()
    subscribers.add(setV)
    load()
    return () => {
      subscribers.delete(setV)
    }
  }, [])
  return v
}

/** The words for a game, and what they mean if the player hovers. */
export function discordWords(g: DiscordGame, open: boolean): { text: string; tip: string } {
  if (g.why === 'off') return { text: 'Hidden from Discord · off everywhere', tip: 'Discord is off in Settings.' }
  if (g.why === 'game') return { text: 'Hidden from Discord · off for this game', tip: 'Discord shows nothing while you play this.' }
  if (g.why === 'hidden') return { text: 'Hidden from Discord · hidden game', tip: 'You hid this game, so Discord shows nothing while you play it. Show it, and anyone who can see you on Discord will see it.' }
  if (g.why === 'adult') {
    const text = g.now ? 'Showing on Discord now as Playing a game' : 'Name hidden from Discord · adult game'
    return { text, tip: "Discord just says Playing a game. Show it, and anyone who can see you on Discord will see this game's name and art." }
  }
  if (g.now) return { text: 'Showing on Discord now', tip: 'Anyone who can see you on Discord sees this.' }
  if (g.playing) return { text: "Shows on Discord once it's open", tip: "Discord isn't open on this PC." }
  return { text: open ? 'Shown on Discord' : "Shown on Discord · Discord isn't open", tip: 'Anyone who can see you on Discord sees this while you play.' }
}
