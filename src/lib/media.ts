import hosts from './media-hosts.json'

/** Keep this host policy in sync with kryo.to/lib/media-hosts.json. */
export function imageProxyUrl(raw: string): string {
  try {
    const url = new URL(raw.startsWith('//') ? `https:${raw}` : raw)
    if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password || url.port) return raw
    const allowed = [...hosts.steamHosts, ...hosts.mediaHosts].includes(url.hostname)
      || [...hosts.steamDomains, ...hosts.mediaDomains].some((domain) => url.hostname === domain || url.hostname.endsWith(`.${domain}`))
    if (!allowed) return raw
    url.protocol = 'https:'
    url.hash = ''
    return `https://kryo.to/api/steam/image.jpg?url=${encodeURIComponent(url.toString())}`
  } catch {
    return raw
  }
}
