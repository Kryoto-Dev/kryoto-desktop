// Use the real native menu decision and download hook without touching the installed app.
import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { useWebBackend } from '@/lib/bridge'
import { useDownloads, type Download } from '@/lib/downloads'
import { NavBar, type TopTab } from '@/shell/NavBar'
import '../src/styles.css'

Object.assign(window, { __TAURI_INTERNALS__: {} })
const listeners = new Map<string, Set<(value: unknown) => void>>()
const calls: string[] = []
let update = 0
let opened = ''
const download = { id: 'fixture', status: 'extracting', received: 0 } as Download
const emit = (event: string, value: unknown) => listeners.get(event)?.forEach(fn => fn(value))
useWebBackend({
  call: async command => { calls.push(command); return command === 'downloads_list' ? [download] : null },
  on: (event, fn) => {
    const entries = listeners.get(event) ?? new Set()
    entries.add(fn); listeners.set(event, entries)
    return () => { entries.delete(fn) }
  },
})
const fixture = {
  calls,
  tick: () => emit('downloads', [{ ...download, received: ++update }]),
  opened: () => opened,
}
Object.assign(window, { navFixture: fixture })
function Harness() {
  const dl = useDownloads()
  const [current, setCurrent] = useState<TopTab>('library')
  const progress = dl.list[0]?.received ?? 0
  return <main className="h-screen bg-background text-foreground">
    <div className="flex h-13 items-stretch border-b border-border">
      <NavBar current={current} canBack={false} canForward={false} onBack={() => {}} onForward={() => {}}
        tabs={(['store', 'library', 'community'] as const).map(id => ({
          id,
          label: id === 'store' ? 'Store' : id === 'library' ? 'Library' : 'Community',
          onOpen: () => { opened = id; setCurrent(id) },
        }))}
        right={<input aria-label="Address" readOnly value={`kryo.to/${current}`} className="kryo-pill h-9 grow border border-border bg-card pl-3 text-[11px] text-muted-foreground" />} />
    </div>
    <output id="progress">{progress}</output>
  </main>
}
createRoot(document.getElementById('root')!).render(<Harness />)
