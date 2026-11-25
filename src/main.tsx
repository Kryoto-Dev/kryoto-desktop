import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import { PopupApp } from './popup/PopupApp'
import { PetApp } from './pet/PetApp'
import { TrayApp } from './tray/TrayApp'
import { GlobalTooltip } from './ui/GlobalTooltip'
import { installErrorLogging } from './lib/log'
import { installWindowDrag } from './lib/window-drag'
import { call } from './lib/bridge'
import { isTauri } from './lib/window'
import { installTextSizeKeys } from './lib/text-size'
import { webglRenderer } from './lib/settings'
import './styles.css'
import { PixelIconMotion } from '@/ui/pixel-icons-motion'

/** The menu view (menus.rs) loads this same page; its label says which it is. */
function windowLabel(): string {
  try {
    const internals = (window as unknown as { __TAURI_INTERNALS__?: { metadata?: { currentWebview?: { label?: string } } } })
      .__TAURI_INTERNALS__
    return internals?.metadata?.currentWebview?.label ?? 'main'
  } catch {
    return 'main'
  }
}

const label = windowLabel()
document.documentElement.dataset.window = label
// WebKitGTK (Linux) draws some effects on the CPU that WebView2 draws on the GPU;
// styles.css trims those under `data-os='linux'`.
if (/Linux/i.test(navigator.userAgent) && !/Android/i.test(navigator.userAgent)) document.documentElement.dataset.os = 'linux'
installErrorLogging(label)
installWindowDrag()
// Keep custom app menus; never expose the browser's Back/Forward/Reload menu.
document.addEventListener('contextmenu', e => e.preventDefault())
// Ctrl + / Ctrl - / Ctrl 0 and Ctrl+wheel, wherever the keyboard is (the pet draws no words).
if (label !== 'pet') installTextSizeKeys()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {label === 'popup' ? (
      <PopupApp />
    ) : label === 'pet' ? (
      <PetApp />
    ) : label === 'tray' ? (
      <TrayApp />
    ) : (
      <>
        <PixelIconMotion />
        <App />
        {/* Every `title` as Kryoto's own tooltip. */}
        <GlobalTooltip />
      </>
    )}
  </StrictMode>,
)

// The first frame is on screen: tell the Linux build this start drew, so it
// does not fall back to its compatible renderer next time (display_env.rs),
// and which renderer WebGL got, so drawing on the CPU is said (report-A). The
// app says whether to ask: on Linux, once per run (`display_webgl_ask`).
if (label === 'main' && isTauri()) {
  const drew = async () => {
    const ask = await call<boolean>('display_webgl_ask').catch(() => false)
    await call('display_rendered', ask ? webglRenderer() : {})
  }
  requestAnimationFrame(() => requestAnimationFrame(() => void drew().catch(() => {})))
}
