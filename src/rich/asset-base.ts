import { useEffect, useState } from 'react'
import { isTauri } from '@/lib/bridge'
import { catalogApiUrl } from '@/lib/endpoint'

/**
 * The site emoji and stickers load from (rich text, ./rich-catalog.ts): the
 * catalog endpoint this app talks to, so a build pointed at a local kryo.to
 * draws that site's art. The web chat takes `VITE_KRYO_API`, like its API.
 */
let base = isTauri() ? 'https://kryo.to' : ((import.meta.env.VITE_KRYO_API as string | undefined) ?? 'https://kryo.to').replace(/\/+$/, '')
let asked: Promise<string> | null = null

export function useAssetBase(): string {
  const [value, setValue] = useState(base)
  useEffect(() => {
    if (!isTauri()) return
    let live = true
    asked ??= catalogApiUrl('/')
      .then((u) => (base = u.replace(/\/+$/, '')))
      .catch(() => base)
    void asked.then((b) => live && setValue(b))
    return () => {
      live = false
    }
  }, [])
  return value
}
