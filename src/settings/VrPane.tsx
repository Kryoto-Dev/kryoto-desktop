import { useEffect, useState } from 'react'
import { ArrowUpRight, Glasses, RotateCw } from '@/ui/pixel-icons'
import { AsciiBar, Button, Caption, Check, Dropdown, Section, Tag } from '@/ui'
import { errorText } from '@/lib/bridge'
import { vrApi, type VrStatus } from '@/lib/library'
import type { Settings } from '@/lib/settings'
import { openExternal } from '@/lib/window'
import { cn } from '@/lib/utils'

const STEAMVR_STORE = 'https://store.steampowered.com/app/250820/SteamVR/'

/**
 * Settings > Running games, VR: the headset runtimes on this PC, which one VR games use,
 * and whether SteamVR is started first, as Steam does. A flat game is never
 * touched; a VR start is a VR entry, "Play in VR", or a game kryo.to marks
 * VR only.
 */
export function VrPane({ s, set }: { s: Settings; set: <K extends keyof Settings>(k: K, v: Settings[K]) => void }) {
  const [status, setStatus] = useState<VrStatus | null>(null)
  const [error, setError] = useState<string | null>(null)
  const look = () => {
    setError(null)
    void vrApi
      .status()
      .then(setStatus)
      .catch((e: unknown) => setError(errorText(e)))
  }
  useEffect(look, [])

  if (!status) return error ? <p className="text-xs text-destructive">{error}</p> : <AsciiBar fraction={null} cells={16} showPct={false} />
  const withManifest = status.runtimes.filter((r) => r.manifest)
  return (
    <div className="grid gap-7">
      {status.experimental ? (
        <p className="text-xs leading-relaxed text-muted-foreground">
          VR on Linux is experimental: Kryoto points games at an OpenXR runtime (Monado, WiVRn) when one is installed. SteamVR through Proton is not set up for you.
        </p>
      ) : null}
      <Section title="On this PC">
        {status.runtimes.length ? (
          <ul className="kryo-radius grid overflow-hidden border border-border">
            {status.runtimes.map((r) => (
              <li key={r.id} className="flex items-center gap-3 border-b border-border px-3 py-2.5 last:border-b-0">
                <Glasses className="size-3.5 shrink-0 text-muted-foreground" />
                <span className="grid min-w-0 grow gap-0.5">
                  <span className="text-xs text-foreground">{r.name}</span>
                  {r.manifest ? <span className="kryo-ascii-art truncate text-[10px] text-muted-foreground" title={r.manifest}>{r.manifest}</span> : null}
                </span>
                {r.active ? <Tag tone="primary">System default</Tag> : null}
                {r.id === 'steamvr' ? <Tag>{status.steamvrRunning ? 'Running' : 'Not running'}</Tag> : null}
              </li>
            ))}
          </ul>
        ) : (
          <p className="kryo-radius border border-dashed border-border p-4 text-center text-xs text-muted-foreground">No VR runtime found. Install SteamVR, or your headset&apos;s own app (Meta Quest Link).</p>
        )}
        <div className="flex flex-wrap gap-2">
          <Button size="sm" onClick={look}>
            <RotateCw className="size-3" />
            Look again
          </Button>
          {!status.steamvrInstalled && !status.experimental ? (
            <Button size="sm" variant="primary" onClick={() => openExternal(STEAMVR_STORE)}>
              Get SteamVR
              <ArrowUpRight className="size-3" />
            </Button>
          ) : null}
        </div>
      </Section>
      <Section title="Runtime VR games use" hint="Most games ask the system's OpenXR runtime. Pick another here to point VR games at it instead.">
        <Dropdown
          className="w-80"
          label="VR runtime"
          value={s.vrRuntime}
          options={[{ value: '', label: "The system's choice" }, ...withManifest.map((r) => ({ value: r.id, label: r.name }))]}
          onChange={(v) => set('vrRuntime', v)}
        />
      </Section>
      {!status.experimental ? (
        <Section title="Before a VR game">
          <Check checked={s.vrStartSteamvr} onChange={(v) => set('vrStartSteamvr', v)} label="Start SteamVR first when it is installed" />
          <Caption className={cn(!status.steamvrInstalled && 'opacity-60')}>For a headset on Quest Link or Air Link, start Link on the headset first.</Caption>
        </Section>
      ) : null}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
  )
}
