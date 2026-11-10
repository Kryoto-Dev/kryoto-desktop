/**
 * Browser URL helpers for the Kryoto Desktop embedded navigator.
 *
 * The address bar accepts either a URL or a plain search query. Anything that
 * is not a valid navigable URL falls back to the configured search engine so
 * typing "hollow knight" just works instead of producing an error state.
 */

import { SITE_ORIGIN } from '@/lib/endpoint'

/** Home page for the embedded browser (also the catalog root). */
export const BROWSER_HOME = `${SITE_ORIGIN}/`

/** Default search engine used when the address input is not a URL. */
export const SEARCH_ENGINE = {
  name: 'DuckDuckGo',
  queryUrl: 'https://duckduckgo.com/?q=',
} as const

/** Schemes the embedded view is allowed to navigate to. */
const ALLOWED_SCHEMES = new Set(['http', 'https'])

/**
 * Resolve raw address-bar input to a navigation URL.
 * Returns a fully qualified http(s) URL, using the search engine fallback
 * when the input is not a valid URL.
 */
export function resolveUserInputToUrl(input: string): string {
  const trimmed = input.trim()
  if (!trimmed) return BROWSER_HOME
  const direct = normalizeUrlCandidate(trimmed)
  if (direct) return direct
  return `${SEARCH_ENGINE.queryUrl}${encodeURIComponent(trimmed)}`
}

/**
 * Try to interpret raw input as a direct URL.
 * Returns null when the input should be treated as a search query.
 */
export function normalizeUrlCandidate(input: string): string | null {
  const value = input.trim()
  if (!value || /\s/.test(value)) return null

  // Explicit scheme: only http(s) navigate inline; anything else (file:,
  // javascript:, data:, tauri:, ...) is never a direct navigation.
  const schemeMatch = /^([a-zA-Z][a-zA-Z0-9+.-]*):\/\//.exec(value)
  if (schemeMatch) {
    const scheme = (schemeMatch[1] ?? '').toLowerCase()
    if (!ALLOWED_SCHEMES.has(scheme)) return null
    try {
      const parsed = new URL(value)
      if (!parsed.hostname || parsed.username) return null
      return parsed.toString()
    } catch {
      return null
    }
  }

  // Bare host: "kryo.to/library", "localhost:3000", "192.168.1.2/x".
  if (isBareHostCandidate(value)) {
    try {
      const parsed = new URL(`https://${value}`)
      if (!parsed.hostname || parsed.username) return null
      return parsed.toString()
    } catch {
      return null
    }
  }
  return null
}

function isBareHostCandidate(value: string): boolean {
  if (value.includes('@')) return false
  const host = value.split('/')[0] ?? ''
  if (!host) return false
  if (host.toLowerCase() === 'localhost') return true
  // IPv4 / IPv6 / port-style hosts.
  if (/^\d{1,3}(\.\d{1,3}){3}(:\d+)?$/.test(host)) return true
  if (host.startsWith('[')) return true
  if (/^[^/\s]+:\d+$/.test(host)) return true
  // Dotted domain with a plausible TLD.
  return /^[^/\s]+\.[^/\s:]{2,}(\/\S*)?$/.test(value)
}
