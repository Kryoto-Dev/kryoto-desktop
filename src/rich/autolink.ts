/**
 * Web addresses in text people wrote (comments, pinned notes), found so they
 * can be drawn as links.
 *
 * Pure and client-safe. Only http and https are ever linked: `javascript:`,
 * `data:` and every other scheme stay text whatever they look like, and a bare
 * `www.` address is taken as https. Punctuation that ends a sentence is left
 * outside the link ("see https://kryo.to/requests." links without the dot), and
 * a closing bracket stays outside unless the address opened one itself, so
 * "(https://en.wikipedia.org/wiki/Foo_(bar))" keeps its last bracket.
 */

export type LinkPiece = { text: string; href?: undefined } | { text: string; href: string }

/** An address: a scheme or `www.`, then anything up to whitespace or an angle bracket. */
const ADDRESS = /\b(?:https?:\/\/|www\.)[^\s<>"]+/gi

/** Characters that end a sentence rather than an address. */
const TRAILING = /[.,;:!?'"*_~]+$/

/** The longest a linked address may be; past that it stays text. */
const MAX_LENGTH = 2048

function trimAddress(raw: string): string {
  let url = raw
  for (;;) {
    const before = url
    url = url.replace(TRAILING, '')
    // A closing bracket the address did not open belongs to the sentence.
    while (/[)\]}]$/.test(url)) {
      const close = url.slice(-1)
      const open = close === ')' ? '(' : close === ']' ? '[' : '{'
      const opens = url.split(open).length - 1
      const closes = url.split(close).length - 1
      if (closes > opens) url = url.slice(0, -1)
      else break
    }
    if (url === before) return url
  }
}

/** The address to link to, or null when it is not a safe http(s) address. */
export function safeHref(address: string): string | null {
  const candidate = /^www\./i.test(address) ? `https://${address}` : address
  if (candidate.length > MAX_LENGTH) return null
  try {
    const url = new URL(candidate)
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null
    // `https://` and nothing else, or a host with no dot that is not a name
    // anyone would mean ("https://..."), is not a link.
    if (!url.hostname || !/[a-z0-9]\.[a-z]/i.test(url.hostname)) return null
    return url.href
  } catch {
    return null
  }
}

/** Split text into plain pieces and linkable addresses, in order. */
export function splitLinks(text: string): LinkPiece[] {
  const out: LinkPiece[] = []
  let at = 0
  for (const match of text.matchAll(ADDRESS)) {
    const start = match.index ?? 0
    const address = trimAddress(match[0])
    const href = address ? safeHref(address) : null
    if (!href) continue
    if (start > at) out.push({ text: text.slice(at, start) })
    out.push({ text: address, href })
    at = start + address.length
  }
  if (at < text.length) out.push({ text: text.slice(at) })
  return out.length ? out : [{ text }]
}

/** Does this text contain anything that would be linked? */
export function hasLinks(text: string): boolean {
  return splitLinks(text).some((p) => p.href)
}
