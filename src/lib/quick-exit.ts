/**
 * What to say when a game was gone within seconds (hooks/useLibrary.ts), and
 * whether that is worth reporting. The client already follows a launcher that
 * hands over to the real game (handoff.rs), so reaching this means nothing
 * from the game's folder was running afterwards either. Pure, for `pnpm test`.
 */

const RUNTIMES_HINT = 'Install everything under Settings > Game runtimes, then press Play again.'

/**
 * What a game's exit code means, for the ones that keep coming back in reports.
 * "Check the exe" sent these people to the wrong place: their exe was right.
 */
const EXIT_REASONS: Record<number, string> = {
  // 0xC0000906: Windows refused to run a file it flagged.
  [-1073739514]: "Your antivirus blocked a file in this game as a threat. Exclude the game's folder, then get the file back: both are on the game's page.",
  // 0xC0000142: a DLL failed as it loaded, mostly a blocked or missing one.
  [-1073741502]: `A file the game needs could not start, usually because an antivirus blocked it or a runtime is missing. ${RUNTIMES_HINT}`,
  // 0xC06D007E: a DLL loaded late was not found, the same missing runtime.
  [-1066598274]: `A file the game needs is missing, usually a runtime. ${RUNTIMES_HINT}`,
  // 0xC0000139: a DLL is there but too old for the game.
  [-1073741511]: `A runtime on this PC is too old for the game. ${RUNTIMES_HINT}`,
  // 0xC000007B: a 32-bit and a 64-bit file mixed up, nearly always a broken runtime.
  [-1073741701]: `A runtime on this PC is damaged or the wrong kind for the game. ${RUNTIMES_HINT}`,
  // 0xC0000005 and 0xC0000409: the game itself crashed.
  [-1073741819]: 'The game crashed as it started. Update your graphics driver and use Repair game. If it keeps crashing, report it on the game\'s page.',
  [-1073740791]: 'The game crashed as it started. Update your graphics driver and use Repair game. If it keeps crashing, report it on the game\'s page.',
  // 0xE0434352: an unhandled .NET error.
  [-532462766]: `The game crashed with a .NET error. ${RUNTIMES_HINT}`,
  // Steam's DRM wrapper's own code: it asked Steam, which said "No licenses" (a tester, 2026-10-30).
  53: 'The game stopped at its Steam check: it asked Steam whether you own it, and Steam said No licenses. Try Kryoto Online under Properties > Emulator & fixes, and report it on the game\'s page so the release can be fixed.',
}

/**
 * `steam`: why the game would have asked Steam whether this account owns it
 * (src-tauri steam_check.rs), which is what a player sees as Steam's "No
 * licenses". Said first: it names the file and what to do, where the exit
 * code alone only says the game closed.
 */
export function quickExit(code: number | null | undefined, detail?: string, steam?: string): [string, boolean] {
  if (steam) return [steam, false]
  // ERROR_CANCELLED: the game needs administrator rights and Windows' prompt was turned down.
  if (code === 1223) return ['It did not start: this game needs administrator rights. Press Play and answer Yes on Windows\' prompt.', true]
  // 0x80131700: the .NET runtime could not start, so it is missing.
  if (code === -2146232576) {
    return [
      'It closed straight away because the .NET Framework it needs is not installed. Install .NET Framework 4.8 (and 3.5 for older games) from Microsoft, then press Play again.',
      true,
    ]
  }
  // 0xC0000135: a DLL it needs is missing, almost always a Visual C++ runtime.
  if (code === -1073741515) {
    return [
      'It closed straight away because a DLL it needs is missing. Install the Visual C++ Redistributables (2015-2022, x64 and x86) and the DirectX runtime, then press Play again.',
      true,
    ]
  }
  const known = code == null ? undefined : EXIT_REASONS[code]
  if (known) return [`${known} (exit code ${code})`, false]
  if (code === 0 || code == null) {
    return [
      'It started and closed again without an error. If no window opened, pick the game\'s own .exe (not a launcher) in Properties.',
      true,
    ]
  }
  // Linux: Wine, Proton and umu all end with a bare 1; their last lines say why.
  if (detail) {
    return [`It closed again straight away (exit code ${code}). Its last lines: ${detail} The whole log is under Properties > Logs.`, false]
  }
  return [
    `It closed again straight away (exit code ${code}). Check the launch options and the exe in Properties.`,
    false,
  ]
}
