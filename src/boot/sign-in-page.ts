/** kryo.to's pages for signing in, where the box stays open. */
export function onSignInPage(url: string): boolean {
  try {
    return /^\/(login|register|signup|link|invite|recover|forgot-password|reset-password|verify-email)(\/|$)/.test(new URL(url).pathname)
  } catch {
    return true
  }
}

/**
 * Where the sign-in box's page has come to rest: on a sign-in page, or
 * `elsewhere` (signed in, and the app taking the account in). Null while it
 * opens, loads or has failed, when it is neither yet.
 */
export function settledOn(page: { url: string; loading: boolean; error: unknown }, opened: boolean): 'sign-in' | 'elsewhere' | null {
  if (!opened || page.loading || page.error || !page.url) return null
  return onSignInPage(page.url) ? 'sign-in' : 'elsewhere'
}
