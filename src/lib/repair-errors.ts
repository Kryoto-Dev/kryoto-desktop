/**
 * The emulator cache folder a Repair error names when Kryoto could not replace
 * a stale copy (kryoto-repair release.rs `stuck_cache`, KB-0382), for its
 * Open folder button. Pure, for `pnpm test`.
 */
export function stuckCacheFolder(error: string | null | undefined): string | null {
  return error?.match(/^Kryoto's copy of .+? in (.+?) changed on this PC and could not be replaced/)?.[1] ?? null
}
