/**
 * kryo.to calls from the web chat: always with the chat's own Bearer token
 * (from the device sign-in), never cookies - kryo.to allows this origin for
 * exactly these routes (proxy.ts, CHAT_WEB_ORIGINS).
 */

export const API_BASE: string = (import.meta.env.VITE_KRYO_API as string | undefined) ?? 'https://kryo.to'
export const GATEWAY_WS: string = (import.meta.env.VITE_KRYO_GATEWAY as string | undefined) ?? 'wss://ws.kryo.to/v1/ws'
/** The gateway's https origin (attachments). */
export const GATEWAY_HTTP = GATEWAY_WS.replace(/^wss:/, 'https:').replace(/^ws:/, 'http:').replace(/\/v1\/ws$/, '')

export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
  ) {
    super(message)
  }
}

export async function api<T>(token: string | null, method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = {}
  if (token) headers.authorization = `Bearer ${token}`
  if (body !== undefined) headers['content-type'] = 'application/json'
  let res: Response
  try {
    res = await fetch(`${API_BASE}${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body), credentials: 'omit' })
  } catch {
    throw new ApiError('Could not reach kryo.to.', 0)
  }
  const data = (await res.json().catch(() => ({}))) as Record<string, unknown>
  if (!res.ok) throw new ApiError(typeof data.error === 'string' ? data.error : `kryo.to answered ${res.status}.`, res.status)
  return data as T
}

/** Saves a file the browser's way. Kryoto Mobile's api.ts asks Android instead. False when the player backs out. */
export async function saveFile(name: string, data: BlobPart, mime: string): Promise<boolean> {
  const url = URL.createObjectURL(new Blob([data], { type: mime }))
  const a = document.createElement('a')
  a.href = url
  a.download = name
  a.click()
  setTimeout(() => URL.revokeObjectURL(url), 10_000)
  return true
}

// ---- device sign-in (RFC 8628, client "chat-web") ----

export type DeviceStart = { device_code: string; user_code: string; verification_uri_complete: string; interval: number; expires_in: number }

export function startSignIn(): Promise<DeviceStart> {
  const name = `Web chat (${navigator.userAgent.includes('Firefox') ? 'Firefox' : navigator.userAgent.includes('Edg/') ? 'Edge' : navigator.userAgent.includes('Chrome') ? 'Chrome' : navigator.userAgent.includes('Safari') ? 'Safari' : 'browser'})`
  return api<DeviceStart>(null, 'POST', '/api/auth/device', { client: 'chat-web', device_name: name })
}

/** Poll until approved. Resolves the token, or throws. */
export async function waitForToken(s: DeviceStart, cancelled: () => boolean): Promise<string> {
  let interval = Math.min(Math.max(s.interval, 1), 10)
  const deadline = Date.now() + Math.min(s.expires_in, 900) * 1000
  while (Date.now() < deadline && !cancelled()) {
    await new Promise((r) => setTimeout(r, interval * 1000))
    const p = await api<{ status: string; token?: string }>(null, 'POST', '/api/auth/device/token', { device_code: s.device_code }).catch(() => null)
    if (!p) continue
    if (p.status === 'ok' && p.token) return p.token
    if (p.status === 'slow_down') interval = Math.min(interval + 1, 10)
    else if (p.status === 'access_denied') throw new Error('The sign-in was refused.')
    else if (p.status !== 'authorization_pending') throw new Error('The sign-in expired. Try again.')
  }
  throw new Error('The sign-in expired. Try again.')
}
