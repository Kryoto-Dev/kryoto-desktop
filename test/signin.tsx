// The real sign-in box, with the Store's web view stood in for by its calls.
import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { useWebBackend } from '@/lib/bridge'
import { SignInPanel } from '@/boot/Boot'
import type { BrowserPageState } from '@/hooks/useBrowserPage'
import '../src/styles.css'

Object.assign(window, { __TAURI_INTERNALS__: {} })
const shown: boolean[] = []
useWebBackend({
  call: async (command, args) => {
    if (command === 'store_visible') shown.push(!!args.visible)
    return null
  },
  on: () => () => {},
})
const at = (url: string, loading: boolean): BrowserPageState => ({ url, title: '', loading, back: null, forward: null, error: null, retrying: false, notice: null })
let go: (page: BrowserPageState) => void = () => {}
Object.assign(window, { signinFixture: {
  // Whether the Store's web view is showing now.
  shown: () => shown.at(-1) ?? null,
  at: (url: string, loading = false) => go(at(url, loading)),
} })
function Harness() {
  const [page, setPage] = useState(at('https://kryo.to/login?next=/', false))
  go = setPage
  return <main className="grid h-screen place-items-center bg-background text-foreground">
    <div className="grid h-[600px]"><SignInPanel page={page} onBack={() => {}} onRetry={() => {}} /></div>
  </main>
}
createRoot(document.getElementById('root')!).render(<Harness />)
