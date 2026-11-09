// The styling rules a reviewer would catch, enforced before a reviewer has to.
//
// Ported from kryo.to's scripts/check-styles.mts, which explains them at length.
// The short version: the reader picks a palette and a corner radius at runtime,
// and every surface is drawn from tokens so both preferences apply. A literal
// colour or a fixed `rounded-lg` is a component ignoring the reader, and it
// looks right to whoever added it because they were looking at one palette.
//
// The same file is copied into every Kryoto frontend that has no check of its
// own (Desktop, Mobile, the Minecraft launcher, status web); only ROOTS, ICONS
// and ALLOW differ. Run with `node scripts/check-styles.mjs`.
import fs from 'node:fs'
import path from 'node:path'

/** Folders scanned, relative to the app. */
const ROOTS = ['src']
/** Where this app's pixel icon module is imported from, for the messages. */
const ICONS = '@/ui/pixel-icons'

const RULES = [
  {
    id: 'literal-color',
    find: /(?:text|bg|border|fill|stroke|ring|shadow|from|via|to|decoration|outline|accent|caret)-\[(?:#[0-9a-fA-F]{3,8}|(?:rgb|rgba|hsl|hsla|oklch|lab)\()/g,
    why: "a literal colour ignores the reader's palette. Use a token: bg-card, text-muted-foreground, border-border - or var(--token) in a style prop.",
  },
  {
    id: 'opaque-white-black',
    find: /\b(?:text|bg|border|fill|stroke|ring|divide)-(?:white|black)(?![/-])/g,
    why: 'white and black are not tokens. Use foreground/background/primary-foreground. The /NN alpha forms (bg-black/60) are allowed - those are scrims over media.',
  },
  {
    id: 'tailwind-palette-color',
    find: /\b(?:text|bg|border|fill|stroke|ring|divide|from|via|to)-(?:slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-(?:50|[1-9]00|950)\b/g,
    why: "Tailwind's own palette is a second, competing theme. Use the status tokens (text-warning, text-success, text-destructive) or the token set in the app's styles.",
  },
  {
    id: 'raw-radius',
    // A sized or arbitrary `rounded-*`, and a bare `rounded` / `rounded-t` (one
    // followed by the end of the string or by another class). The reader's own
    // tokens in brackets are not a fixed radius.
    find: /\brounded(?:-(?:t|r|b|l|tl|tr|br|bl|s|e|ss|se|es|ee))?-(?:xs|sm|md|lg|xl|2xl|3xl|4xl|\[(?!var\(--kryo-radius(?:-pill)?\)\])[^\]]+\])|(?<=\s)rounded(?:-(?:t|r|b|l|tl|tr|br|bl|s|e|ss|se|es|ee))?(?=["'`])|(?<![\w-])rounded(?:-(?:t|r|b|l|tl|tr|br|bl|s|e|ss|se|es|ee))?(?=\s+!?(?:[a-z][\w:]*-|(?:border|flex|grid|block|inline|hidden|relative|absolute|fixed|sticky|shadow|truncate|uppercase)\b))/g,
    why: 'the corner radius is a reader preference. Use `kryo-radius` (surfaces) or `kryo-pill` (chips, controls and dots); `rounded-full` and `rounded-none` only when the shape is the point.',
  },
  {
    id: 'shape-variant',
    // Only where `kryo-radius` is plain CSS rather than an `@utility` (kryo.to
    // and the pages copied from it): a variant in front of a plain class
    // compiles to nothing, so `md:kryo-radius` stays square on every setting.
    plainShapesOnly: true,
    find: /(?:^|[\s"'`])(?:[\w-]+:)+kryo-(?:radius|pill|radius-end|segment)\b/g,
    why: '`kryo-radius` and `kryo-pill` are plain classes in this app and take no variant: `md:kryo-radius` draws nothing. Behind a breakpoint write `md:rounded-[var(--kryo-radius)]` (or `--kryo-radius-pill`).',
  },
  {
    id: 'round-chip',
    // A class string with `rounded-full` and the width of a chip or a box, or
    // a thin clipped track (a meter): it stays a capsule when the reader picks
    // Sharp. A circle (size-*, a dot's equal h/w) keeps rounded-full.
    find: /(["'`])(?=(?:[^"'`\n]*\s)?rounded-full(?=[\s"'`]))(?:(?=(?:[^"'`\n]*\s)?(?:px-(?:[1-9]|\[)|w-fit|w-auto|min-w-[1-9[]))|(?=(?:[^"'`\n]*\s)?overflow-hidden(?=[\s"'`]))(?=(?:[^"'`\n]*\s)?h-(?:0\.5|1|1\.5|2|2\.5|3)(?=[\s"'`]))(?!(?:[^"'`\n]*\s)?(?:size-|w-(?:0\.5|1|1\.5|2|2\.5|3)(?=[\s"'`]))))[^"'`\n]*\1/g,
    why: '`rounded-full` is for circles (avatars, dots, status lights). A chip, a badge, a meter or a button wider than it is tall follows the reader: `kryo-pill`.',
  },
  {
    id: 'pill-textarea',
    // `inputCls` is the one-line field, shaped as a pill.
    find: /<(?:textarea|RichTextarea|RichBox)\b[^>]*\b(?:inputCls|kryo-pill)\b/g,
    why: 'a box of several lines is a surface: `kryo-radius`. The one-line field (`inputCls`, `kryo-pill`) turns it into a capsule on Round and Pill.',
  },
  {
    id: 'inline-literal-radius',
    // A number, or a string with a length in it, that does not come from the
    // reader's tokens. 0, 50%, 9999 and `inherit` are shapes, not radii.
    find: /\bborder(?:Top|Bottom|Start|End)?(?:Left|Right|Start|End)?Radius\s*:\s*(?:(?!(?:999|9999)\b)[1-9][\d.]*|(["'`])(?![^"'`]*var\(--)(?!\s*(?:0|0px|50%|100%|9999px|inherit)\s*["'`])[^"'`]*\d[^"'`]*\1)/g,
    why: "a fixed radius in a style prop ignores the reader's corners. Use the `kryo-radius` / `kryo-pill` class, or `var(--kryo-radius)` in the style.",
  },
  {
    id: 'css-literal-radius',
    in: 'css',
    // An absolute length (px, rem, em) that is not one of the reader's tokens.
    // Percentages and `calc(var(--u) ...)` are proportions of a drawn shape.
    find: /\bborder(?:-(?:top|bottom|start|end)-(?:left|right|start|end))?-radius\s*:(?![^;}]*var\(--(?:kryo-radius|radius))(?![^;}]*\b(?:9999|999)px)(?=[^;}]*\b(?:[1-9]\d*|\d*\.\d+)(?:px|rem|em)\b)[^;}]*/g,
    why: "a fixed radius in a stylesheet ignores the reader's corners. Use `var(--kryo-radius)` (surfaces) or `var(--kryo-radius-pill)` (chips), with min()/calc() around it if a small thing needs less.",
  },
  {
    id: 'unshaped-dot',
    // A small filled dot whose class string gives it no shape: it stays square
    // whatever the reader picked. Matched within one string literal.
    find: /(["'`])(?=[^"'`\n]*\b(?:size-(?:1|1\.5|2)|h-(?:1|1\.5|2) w-(?:1|1\.5|2))(?![\w.-]))(?=[^"'`\n]*(?:^|\s)bg-(?!transparent)[a-z])(?![^"'`\n]*(?:kryo-pill|kryo-radius|kryo-dot|kryo-square|rounded))[^"'`\n]*\1/g,
    why: "a dot needs the reader's corners: `kryo-pill`, or the shared new/unread dot. `rounded-full` only for a status light, `kryo-square` where square is the point.",
  },
  {
    id: 'inline-literal-color',
    find: /style=\{\{[^}]*(?:color|background|border|fill|stroke)[^}]*['"]#[0-9a-fA-F]{3,8}['"]/g,
    why: 'same as literal-color, in a style prop. `var(--primary)` and a computed token both work here.',
  },
  {
    id: 'native-select',
    find: /<select[\s>]/g,
    why: "a native <select> is drawn by the operating system and ignores the reader's palette and corners. Use the shared Dropdown.",
  },
  {
    id: 'native-checkbox',
    find: /<input[^>]*type=["']checkbox["']/g,
    why: "a native checkbox is the OS's own widget and ignores the reader's palette. Use the shared Checkbox.",
  },
  {
    id: 'glyph-icon',
    find: new RegExp(
      [
        `>\\s*(?:[+xX×✕✖✗✘✓✔\\-−–←→↑↓↗↘▲▼▾▴▸◂◀▶⋯⋮☰⟳↻⚙✎★☆]|&(?:times|larr|rarr|uarr|darr|minus|check|raquo|laquo);)\\s*<`,
        `^\\s*(?:[+×✕✖✗✘✓✔\\-−–←→↑↓↗↘▲▼▾▴▸◂◀▶⋯⋮☰⟳↻⚙✎★☆]|&(?:times|larr|rarr|uarr|darr|minus|check);)\\s*$`,
        `(?:[A-Za-z][^<>{}=;'"\`]*|\\})\\s(?:[→←↗↘»›]|&(?:rarr|larr|raquo);)\\s*(?:<|$)`,
        `>\\s*(?:[←‹«]|&(?:larr|laquo);)\\s+[A-Za-z]`,
        `['"\`](?:[×✕✖✗✘✓✔−←→↗▾▸★☆⋯⋮☰]|&times;)(?:\\s+[A-Za-z][^'"\`]*)?['"\`]`,
      ].join('|'),
      'g',
    ),
    why: `a typed character is not an icon: it renders in whatever the font does with it and is read aloud as "times" or "right arrow". Use the pixel icon (X, Plus, Minus, ArrowRight, ArrowUpRight, ChevronDown...) from ${ICONS} with aria-hidden.`,
  },
  {
    id: 'lucide-import',
    find: /from ['"]lucide-react['"]/g,
    why: `icons come from ${ICONS}, which draws the pixel art, carries the classic icon for readers who pick Classic, and plays the hover motion.`,
  },
  {
    id: 'native-dialog',
    find: /\bwindow\.(?:alert|confirm|prompt)\s*\(/g,
    why: "native browser dialogs are drawn by the operating system in its own font and colours. Use the app's confirm/alert dialog or Modal.",
  },
]

/**
 * The exceptions, each with the reason it is one. Keyed `path::rule`; paths use
 * forward slashes and match as a prefix, so a folder can be excused.
 */
const ALLOW = {
  'src/bigpicture/BigPicture.tsx::opaque-white-black':
    'game titles sit on a black scrim over the cover art, which is a photo and not a theme surface',
  'src/bigpicture/BigPictureIntro.tsx::opaque-white-black':
    'the TV-mode intro is a black stage, like a console boot screen, in every palette',
  'src/boot/Boot.tsx::opaque-white-black':
    'the boot splash draws the ASCII mark on black, as the brand is made to be seen',
  'src/ui/ReleaseIntro.tsx::opaque-white-black':
    'the release video letterboxes against black - a video surround, not a page surface',
  'src/shell/TitleBar.tsx::opaque-white-black':
    "the Windows close button turns Windows' own red with white on hover, as every native window's does",
  'src/shell/TitleBar.tsx::literal-color':
    "the Windows close button turns Windows' own red on hover, as every native window's does",
  'src/controller/PadArtView.tsx::literal-color':
    'controller button badges are drawn as the pad prints them, the same in every palette',
  'src/controller/PadArtView.tsx::raw-radius':
    'controller button badges copy the physical button shape',
}

/**
 * Existing violations, frozen: a list of places that predate the rule and a
 * promise that the number only goes down. `--update` rewrites it when the count
 * drops and refuses when it grows.
 */
const BASELINE_PATH = path.join('scripts', 'styles-baseline.json')
const EXT = /\.(tsx|ts|jsx|css)$/

const files = []
const walk = (dir) => {
  if (!fs.existsSync(dir)) return
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, entry.name)
    if (entry.isDirectory()) {
      if (entry.name === 'node_modules' || entry.name.startsWith('.') || entry.name === 'pkg') continue
      walk(p)
    } else if (EXT.test(entry.name) && !entry.name.endsWith('.d.ts')) files.push(p)
  }
}
ROOTS.forEach(walk)

const allowed = (rel, id) => Object.keys(ALLOW).some((k) => {
  const [p, rule] = k.split('::')
  return rule === id && rel.startsWith(p)
})

/** `kryo-radius` is an `@utility` here (variants work) or plain CSS (they do not). */
const plainShapes = !files.some((f) => f.endsWith('.css') && /@utility\s+kryo-radius\b/.test(fs.readFileSync(f, 'utf8')))

const hits = []
for (const file of files) {
  const rel = file.split(path.sep).join('/')
  const kind = rel.endsWith('.css') ? 'css' : 'code'
  fs.readFileSync(file, 'utf8').split(/\r?\n/).forEach((line, i) => {
    if (/^\s*(?:\/\/|\*|\/\*)/.test(line)) return
    for (const rule of RULES) {
      if ((rule.in ?? 'code') !== kind || (rule.plainShapesOnly && !plainShapes)) continue
      if (allowed(rel, rule.id)) continue
      for (const m of line.matchAll(rule.find)) {
        hits.push({ file: rel, line: i + 1, rule, text: m[0].slice(0, 80), snippet: line.trim().slice(0, 88) })
      }
    }
  })
}

const key = (h) => `${h.file}::${h.rule.id}::${h.text}`
const firstRun = !fs.existsSync(BASELINE_PATH)
const known = new Set(firstRun ? [] : JSON.parse(fs.readFileSync(BASELINE_PATH, 'utf8')).allowed ?? [])
const current = new Set(hits.map(key))
const fresh = hits.filter((h) => !known.has(key(h)))
const fixed = [...known].filter((k) => !current.has(k))

if (process.argv.includes('--update')) {
  if (fresh.length > 0 && !firstRun) {
    console.log(`\n  Refusing to record ${fresh.length} NEW violation(s). Fix them, or add a reasoned entry to ALLOW.\n`)
    for (const h of fresh) console.log(`      ${h.file}:${h.line}  ${h.text}`)
    process.exit(1)
  }
  if (current.size === 0) {
    if (!firstRun) fs.rmSync(BASELINE_PATH)
  } else fs.writeFileSync(BASELINE_PATH, `${JSON.stringify({ allowed: [...current].sort() }, null, 2)}\n`)
  console.log(`  ok    baseline rewritten: ${current.size} entries (${fixed.length} fewer)`)
  process.exit(0)
}

if (fresh.length === 0) {
  const note = fixed.length > 0 ? ` (${fixed.length} baselined one(s) now fixed - run --update)` : ''
  console.log(`  ok    ${files.length} files, no new styling-token violations${current.size > 0 ? `; ${current.size} baselined` : ''}${note}`)
  process.exit(0)
}

const byRule = new Map()
for (const h of fresh) byRule.set(h.rule.id, [...(byRule.get(h.rule.id) ?? []), h])
console.log('')
for (const [id, list] of byRule) {
  console.log(`  ${id}\n    ${list[0].rule.why}\n`)
  for (const h of list) console.log(`      ${h.file}:${h.line}  ${h.text}\n        ${h.snippet}`)
  console.log('')
}
console.log(`  ${fresh.length} NEW violation(s). Fix them, or - if the case is genuinely an exception -\n  add it to ALLOW in scripts/check-styles.mjs WITH THE REASON.\n`)
process.exit(1)
