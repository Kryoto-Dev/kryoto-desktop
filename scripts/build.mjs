// Build the installer without the build machine in it. OPSEC!!
//
//   pnpm app:build            the release build, signed for the updater with
//                             TAURI_SIGNING_PRIVATE_KEY. If that key is set and
//                             its password is not, tauri stops to ask for the
//                             password - which is what a plain run on a machine
//                             with the key installed does. On macOS the merged
//                             config produces an app and a DMG.
//   pnpm app:build:local      the same build for trying on this PC: no signing,
//                             no key or password asked for, and the installer is
//                             copied out of the repository (see below).
//   pnpm app:build:mac        macOS app + DMG targets explicitly (run on macOS).
//
// Extra arguments go to `tauri build` and `tauri bundle`.
//
// Rust keeps the source path of every dependency in the binary for its panic
// messages - `C:\Users\<name>\.cargo\registry\...` - which names the account
// that built it. This remaps those paths to neutral ones for the build, taken
// from this machine at build time so no path is written into the repository.
//
// C code built from source keeps paths Rust's remap never reaches (OpenSSL's
// install prefix, SDL2's __FILE__ in its asserts); those are overwritten in
// the binary. So the order is: build the executable (no installers), clean it,
// check it, and only then bundle and sign the installers - from the cleaned
// executable. Flags cannot do the C part: OpenSSL bakes its whole compiler
// command line into the binary, so a path-mapping flag would itself leak.

import { spawnSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { tauriEnv, root } from './tauri-env.mjs'

const home = os.homedir()
const cargoHome = process.env.CARGO_HOME || path.join(home, '.cargo')
const rustupHome = process.env.RUSTUP_HOME || path.join(home, '.rustup')

const remaps = [
  [cargoHome, '/cargo'],
  [rustupHome, '/rustup'],
  [root, '/kryoto-desktop'],
  [home, '/home'],
]
const flags = remaps.map(([from, to]) => `--remap-path-prefix=${from}=${to}`).join(' ')
const env = tauriEnv({ RUSTFLAGS: [process.env.RUSTFLAGS, flags].filter(Boolean).join(' ') })

// A local build is never an update anyone receives, so it is not signed and
// nothing may ask for the signing key's password.
const local = process.argv.includes('--local')
if (local) {
  for (const name of Object.keys(env)) if (name.startsWith('TAURI_SIGNING_PRIVATE_KEY')) delete env[name]
  mkdirSync(path.join(root, '.tmp'), { recursive: true })
  writeFileSync(path.join(root, '.tmp', 'local.conf.json'), JSON.stringify({ bundle: { createUpdaterArtifacts: false } }))
}


const extra = [...process.argv.slice(2).filter((a) => a !== '--local'), ...(local ? ['--config', '.tmp/local.conf.json'] : [])]
const tauri = (args) => spawnSync(['pnpm tauri', ...args, ...extra].join(' '), { cwd: root, env, stdio: 'inherit', shell: true })
const built = tauri(['build', '--no-bundle'])
if (built.status !== 0) process.exit(built.status ?? 1)

const exe = path.join(process.env.CARGO_TARGET_DIR || path.join(root, 'src-tauri', 'target'), 'release', process.platform === 'win32' ? 'kryoto-desktop.exe' : 'kryoto-desktop')

// OpenSSL bakes its Configure --prefix (under Cargo's target/ dir) into its
// default enginesdir/modulesdir strings. The C compiler never sees RUSTFLAGS,
// so no --remap-path-prefix can reach them, and they name the build machine.
// They are dead defaults - the build is `no-module`, nothing is ever loaded
// from them, and the path does not exist on any player's PC either - so they
// are overwritten in place with a short neutral path, NUL-padded to the exact
// same byte length (they are C strings; the binary layout must not move).
// The leak check below then proves the binary is clean.
const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/\\\\|\//g, '[\\\\/]')
{
  const buf = readFileSync(exe)
  const text = buf.toString('latin1')
  const pattern = /([A-Za-z]:[\\/]|\/)[^\0"]*?target[\\/][^\\/"\0]+[\\/]build[\\/]openssl-sys-[^\\/"\0]+[\\/]out[\\/]openssl-build[\\/]install/g
  let patched = 0
  const clean = text.replace(pattern, (match) => {
    patched++
    const replacement = '/kryoto/openssl-install'
    return replacement + '\0'.repeat(match.length - replacement.length)
  })
  // C source paths (__FILE__ in SDL2's asserts, and in any other C built
  // from a crate): the C string is cut to the path inside the crate - from
  // `/home/x/.cargo/registry/src/index.../sdl2-sys-0.38.0/SDL/src/a.c` to
  // `sdl2-sys-0.38.0/SDL/src/a.c` - and NUL-padded to the same length.
  // Anything else under the build folder or the home folder keeps only its
  // file name. Matched case-insensitively and with either separator, the way
  // compilers spell them.
  const prefixes = [
    new RegExp(`${escape(cargoHome)}[\\\\/]registry[\\\\/]src[\\\\/][^\\\\/\\0"]+[\\\\/]`, 'i'),
    new RegExp(`${escape(root)}[\\\\/]`, 'i'),
    new RegExp(`${escape(home)}[\\\\/]`, 'i'),
  ]
  let cpaths = 0
  const cleaner = clean.replace(new RegExp(`(?:${prefixes.map((r) => r.source).join('|')})[^\\0"]*?\\.(?:c|h|cc|cpp|inc)(?=\\0)`, 'gi'), (match) => {
    let rest = match
    for (const re of prefixes) {
      const m = re.exec(match)
      if (m && m.index === 0) {
        rest = match.slice(m[0].length)
        if (re !== prefixes[0]) rest = rest.split(/[\\/]/).pop()
        break
      }
    }
    cpaths++
    return rest + '\0'.repeat(match.length - rest.length)
  })
  if (patched > 0 || cpaths > 0) {
    writeFileSync(exe, Buffer.from(cleaner, 'latin1'))
    console.log(`\nNeutralized ${patched} baked OpenSSL install path(s) and ${cpaths} C source path(s).`)
  }
}

const bytes = readFileSync(exe).toString('latin1').toLowerCase()
const name = os.userInfo().username.toLowerCase()
// Each as the start of a path (followed by a separator), not as bare text: a
// home of `/root` (a container building as root) is also the tail of Steam's
// own `~/.steam/root`, which the app looks for on purpose.
const leaks = [home.toLowerCase(), `users\\${name}`, `users/${name}`, `home/${name}`]
  .flatMap((s) => [`${s}/`, `${s}\\`])
  .filter((s) => bytes.includes(s))
if (leaks.length) {
  console.error(`\nThe build still contains ${leaks.map((l) => JSON.stringify(l)).join(' and ')}. Do not ship it.`)
  // Where: a few of them, with the text around each, so the next fix is obvious.
  const raw = readFileSync(exe).toString('latin1')
  const at = raw.toLowerCase()
  for (const l of leaks.slice(0, 2)) {
    let i = at.indexOf(l)
    for (let n = 0; i >= 0 && n < 4; n++, i = at.indexOf(l, i + 1)) {
      const from = raw.lastIndexOf('\0', i) + 1
      const to = raw.indexOf('\0', i)
      console.error(`  ${JSON.stringify(raw.slice(from, Math.min(to < 0 ? i + 160 : to, from + 240)))}`)
    }
  }
  process.exit(1)
}
console.log('\nNo build-machine paths in the binary.')

// The installers, from the cleaned executable (and signed, when the key is set).
const bundled = tauri(['bundle'])
if (bundled.status !== 0) process.exit(bundled.status ?? 1)

// A local build is for running on this PC. A program started from inside the
// repository folder can be confined to writing inside it - an installer run from
// `target/` cannot write the install folder, and the app cannot write its data -
// so the installer is copied somewhere outside the repository to be run from.
if (local) {
  const bundleDir = path.join(process.env.CARGO_TARGET_DIR || path.join(root, 'src-tauri', 'target'), 'release', 'bundle')
  const kind = process.platform === 'win32' ? ['nsis', '.exe'] : process.platform === 'darwin' ? ['dmg', '.dmg'] : ['appimage', '.AppImage']
  const dir = path.join(bundleDir, kind[0])
  const found = existsSync(dir) ? readdirSync(dir).filter((f) => f.endsWith(kind[1])) : []
  if (found.length === 0) {
    console.error(`\nNo installer found in ${dir}.`)
    process.exit(1)
  }
  const out =
    process.env.KRYOTO_LOCAL_OUT ||
    (process.platform === 'win32' && existsSync('D:/') ? 'D:/kryo-builds/kryoto-desktop' : path.join(home, 'kryoto-builds', 'kryoto-desktop'))
  mkdirSync(out, { recursive: true })
  for (const file of found) copyFileSync(path.join(dir, file), path.join(out, file))
  console.log(`\nLocal build ready (unsigned): ${path.join(out, found[0])}\nRun it from there, not from the repository.`)
}
