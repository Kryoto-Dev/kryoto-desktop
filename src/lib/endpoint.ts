import { call, isTauri } from '@/lib/bridge'

/**
 * The site this app talks to: kryo.to, or the Kryoto Preview for the preview
 * builds (built with VITE_KRYOTO_CHANNEL=preview; kryo.to docs/preview.md).
 */
export const IS_PREVIEW_BUILD = import.meta.env.VITE_KRYOTO_CHANNEL === 'preview'
export const SITE_ORIGIN = IS_PREVIEW_BUILD ? 'https://preview.kryo.to' : 'https://kryo.to'
const DEFAULT_ENDPOINT = SITE_ORIGIN

let endpoint: Promise<string> | null = null

/** The site origin used by the embedded Store and its direct API requests. */
export function catalogApiUrl(path: string): Promise<string> {
  endpoint ??= isTauri()
    ? call<{ catalogEndpoint?: string }>('settings_get').then((s) => s.catalogEndpoint?.trim() || DEFAULT_ENDPOINT)
    : Promise.resolve(DEFAULT_ENDPOINT)
  return endpoint.then((base) => new URL(path, `${base.replace(/\/+$/, '')}/`).toString())
}

export function clearCatalogEndpointCache() {
  endpoint = null
}
