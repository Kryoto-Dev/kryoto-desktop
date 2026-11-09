import { useEffect, useState } from 'react'
import { Dropdown, inputCls } from '@/ui'
import { call } from '@/lib/bridge'

export type CompatTool = { name: string; path: string; kind: 'proton' | 'umu' | 'wine' }

const AUTO = '__auto__'
const CUSTOM = '__custom__'

/** Every Wine and Proton found on this computer, for the dropdowns. */
export function useCompatTools() {
  const [tools, setTools] = useState<CompatTool[] | null>(null)
  useEffect(() => {
    void call<CompatTool[]>('compat_tools')
      .then(setTools)
      .catch(() => setTools([]))
  }, [])
  return tools
}

/**
 * Steam's compatibility-tool list: the Proton and Wine builds on this
 * computer, newest Proton first, or a path of your own. "Automatic" uses the
 * first one found.
 */
export function CompatPicker({
  value,
  onChange,
  autoLabel = 'Automatic',
}: {
  value: string | null
  onChange: (v: string | null) => void
  autoLabel?: string
}) {
  const tools = useCompatTools()
  const known = tools?.some((t) => t.path === value)
  const [custom, setCustom] = useState(!!value && tools !== null && !known)
  /** Picked just now (not restored from a saved path): the field takes the cursor. */
  const [picked, setPicked] = useState(false)
  useEffect(() => {
    if (tools && value && !tools.some((t) => t.path === value)) setCustom(true)
  }, [tools, value])

  const first = tools?.[0]
  const options = [
    { value: AUTO, label: first ? `${autoLabel} (${first.name})` : autoLabel },
    ...(tools ?? []).map((t) => ({ value: t.path, label: t.name, hint: t.kind === 'proton' ? 'Proton' : t.kind === 'umu' ? 'umu' : 'Wine' })),
    { value: CUSTOM, label: 'Another one…' },
  ]
  const current = custom ? CUSTOM : value && known ? value : AUTO

  return (
    <div className="grid gap-2">
      <Dropdown
        label="Compatibility tool"
        value={current}
        options={options}
        onChange={(v) => {
          if (v === CUSTOM) {
            setPicked(true)
            return setCustom(true)
          }
          setCustom(false)
          onChange(v === AUTO ? null : v)
        }}
      />
      {custom ? (
        <input
          className={inputCls}
          autoFocus={picked}
          value={value ?? ''}
          placeholder="/path/to/proton or /usr/bin/wine"
          onChange={(e) => onChange(e.target.value || null)}
        />
      ) : null}
      {tools && tools.length === 0 ? (
        <p className="text-xs text-muted-foreground">No Proton or Wine found. Install Steam with Proton, or Wine, then reopen this.</p>
      ) : null}
    </div>
  )
}
