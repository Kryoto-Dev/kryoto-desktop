/**
 * The small square picture on a game's row in the Library list, best first:
 * the player's own icon, Steam's icon, then the cover the list drew before
 * games had icons. A row tries the next one when a picture does not load.
 * Pure, for `pnpm test`.
 */
export type IconArt = { icon?: string | null; customIcon?: string | null; cover: string | null; customCover?: string | null }

export function iconsFor(game: IconArt): string[] {
  const all = [game.customIcon, game.icon, game.customCover ?? game.cover]
  return [...new Set(all.filter((u): u is string => !!u))]
}
