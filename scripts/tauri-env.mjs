// The environment `pnpm app:dev` and `pnpm app:build` run `tauri` in.
//
// Two Windows problems, both fixed here so neither script has to know:
//
// - Git's MSYS tools. In a bash step they sit ahead of everything on PATH, so
//   OpenSSL's Configure finds MSYS perl (missing modules it needs) and the
//   linker resolves to MSYS `link` instead of MSVC's. Those directories are
//   dropped and the real toolchain goes first. Local builds from Git Bash hit
//   the same shadowing.
// - Programs started from inside a workspace folder can be confined to writing
//   inside it, which makes esbuild fail with "remove ...\Temp\esbuild-...:
//   Access is denied". Scratch files go in `.tmp/` in this folder instead
//   (git-ignored); it is the same folder everywhere, so it costs nothing where
//   the confinement does not exist.

import { existsSync, mkdirSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

export function tauriEnv(extra = {}) {
  const env = { ...process.env, ...extra }

  if (process.platform === 'win32') {
    const sep = ';'
    const parts = (env.PATH || env.Path || '').split(sep)
    const shadow = /(^|[\\/])git[\\/](usr|mingw64)[\\/]bin[\\/]?$/i
    const kept = parts.filter((p) => !shadow.test(p.replace(/\//g, '\\')))
    const first = []
    const linker = env.CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER
    if (linker && existsSync(linker)) first.push(path.dirname(linker))
    for (const dir of ['C:\\Strawberry\\perl\\bin', 'C:\\Program Files\\NASM']) {
      if (existsSync(dir)) first.push(dir)
    }
    const seen = new Set()
    const PATH = [...first, ...kept]
      .filter((p) => p && !seen.has(p.toLowerCase()) && (seen.add(p.toLowerCase()), true))
      .join(sep)
    // Windows env names are case-insensitive but a JS copy of them is not:
    // writing both spellings would leave the child with whichever wins.
    delete env.Path
    env.PATH = PATH

    const tmp = path.join(root, '.tmp')
    mkdirSync(tmp, { recursive: true })
    env.TEMP = tmp
    env.TMP = tmp
  }

  return env
}
