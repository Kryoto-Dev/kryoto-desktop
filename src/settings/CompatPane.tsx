import { useCallback, useEffect, useState } from 'react'
import { Download, RefreshCw, Trash2 } from 'lucide-react'
import { AsciiBar, Button, Caption, Check, Section } from '@/ui'
import { call, errorText, on } from '@/lib/bridge'
import type { Settings } from '@/lib/settings'
import { CompatPicker, type CompatTool } from '@/settings/CompatPicker'

type Status = {
  tools: (CompatTool & { managed?: boolean })[]
  platform: 'linux' | 'macos' | 'windows'
  umu: string | null
  mangohud: boolean
  gamemode: boolean
}

type Progress = { step: string; fraction: number | null }

/**
 * Settings > Compatibility for Windows games. Linux uses Proton/Wine like
 * Steam; macOS uses user-created Sikarugir wrapper apps because those wrappers
 * own the Wine engine, renderer and prefix.
 */
export function CompatPane({ s, set }: { s: Settings; set: <K extends keyof Settings>(k: K, v: Settings[K]) => void }) {
  const [status, setStatus] = useState<Status | null>(null)
  const [busy, setBusy] = useState<Progress | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [version, setVersion] = useState(0)

  const refresh = useCallback(() => {
    void call<Status>('compat_status')
      .then(setStatus)
      .catch(() => setStatus({ tools: [], platform: 'linux', umu: null, mangohud: false, gamemode: false }))
    setVersion((v) => v + 1)
  }, [])

  useEffect(refresh, [refresh])
  useEffect(() => {
    let off: (() => void) | undefined
    void on<Progress>('compat-progress', (p) => setBusy(p)).then((fn) => (off = fn))
    return () => off?.()
  }, [])

  const install = async () => {
    setError(null)
    setBusy({ step: 'Starting', fraction: null })
    try {
      const tool = await call<CompatTool>('compat_install')
      if (!s.defaultCompatTool || !status?.tools.some((t) => t.path === s.defaultCompatTool)) set('defaultCompatTool', tool.path)
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(null)
      refresh()
    }
  }

  const remove = async (tool: CompatTool) => {
    setError(null)
    try {
      await call('compat_remove', { path: tool.path })
      if (s.defaultCompatTool === tool.path) set('defaultCompatTool', null)
    } catch (e) {
      setError(errorText(e))
    }
    refresh()
  }

  const macos = status?.platform === 'macos'
  const backends = status?.tools.filter((t) => (macos ? t.kind === 'sikarugir' : t.kind !== 'umu')) ?? []
  const ownProton = backends.some((t) => t.managed && t.kind === 'proton')

  return (
    <>
      <Section
        title={macos ? 'Sikarugir wrappers' : 'Proton'}
        hint={macos ? 'Sikarugir wrappers provide the Wine engine and graphics backend for Windows games on macOS.' : 'Proton runs Windows games on Linux. Kryoto gets it for you, like Steam does.'}
      >
        <div className="grid gap-3">
          {backends.length ? (
            <ul className="kryo-radius grid gap-px overflow-hidden border border-border bg-border">
              {backends.map((t) => (
                <li key={t.path} className="flex items-center gap-3 bg-card px-3 py-2.5">
                  <span className="grid min-w-0 grow">
                    <b className="truncate text-xs text-foreground">{t.name}</b>
                    <span className="truncate text-[10px] text-muted-foreground">
                      {t.managed ? 'From Kryoto' : t.kind === 'sikarugir' ? 'Sikarugir wrapper' : t.kind === 'proton' ? 'From Steam' : 'Installed on this computer'}
                    </span>
                  </span>
                  {t.managed && t.kind !== 'sikarugir' ? (
                    <Button size="sm" variant="ghost" onClick={() => void remove(t)} disabled={!!busy}>
                      <Trash2 className="size-3" />
                      Remove
                    </Button>
                  ) : null}
                </li>
              ))}
            </ul>
          ) : status ? (
            <Caption>{macos ? 'No Sikarugir wrappers found in ~/Applications/Sikarugir.' : 'No Proton or Wine on this computer yet.'}</Caption>
          ) : null}

          {macos ? (
            <Caption>
              Install Sikarugir, create a wrapper for a Wine game, and save the wrapper in <code>~/Applications/Sikarugir</code>. Refresh this page after creating it.
            </Caption>
          ) : busy ? (
            <div className="grid gap-1.5">
              <AsciiBar fraction={busy.fraction} cells={28} className="text-foreground" />
              <Caption>{busy.step}</Caption>
            </div>
          ) : (
            <div className="flex flex-wrap items-center gap-2">
              <Button variant={ownProton ? 'outline' : 'primary'} onClick={() => void install()}>
                {ownProton ? <RefreshCw className="size-3" /> : <Download className="size-3" />}
                {ownProton ? 'Get the newest Proton' : 'Get Proton'}
              </Button>
              <Caption>About 500 MB, once.</Caption>
            </div>
          )}
          {error ? <p className="text-xs text-destructive">{error}</p> : null}
          {!macos && status ? (
            <Caption>
              {status.umu
                ? "Games run inside Steam's game runtime, the same way Steam runs them."
                : "Get Proton also sets up Steam's game runtime, so games run the way Steam runs them."}
            </Caption>
          ) : null}
        </div>
      </Section>

      <Section title="Run Windows games with" hint="Each game can pick its own under Properties, Compatibility.">
        <CompatPicker key={version} value={s.defaultCompatTool} onChange={(v) => set('defaultCompatTool', v)} />
      </Section>

      {!macos ? (
        <Section title="While playing">
          <div className="grid gap-3">
            <Check
              checked={s.linuxMangohud && !!status?.mangohud}
              disabled={!status?.mangohud}
              onChange={(v) => set('linuxMangohud', v)}
              label={status?.mangohud ? 'Show frame rate and load (MangoHud)' : 'Show frame rate and load (install MangoHud first)'}
            />
            <Check
              checked={s.linuxGamemode && !!status?.gamemode}
              disabled={!status?.gamemode}
              onChange={(v) => set('linuxGamemode', v)}
              label={status?.gamemode ? 'Give games full speed (GameMode)' : 'Give games full speed (install GameMode first)'}
            />
            <Check
              checked={s.linuxFsr}
              onChange={(v) => set('linuxFsr', v)}
              label="Sharpen games played below your screen's resolution (FSR, Proton-GE only)"
            />
          </div>
        </Section>
      ) : null}
    </>
  )
}
