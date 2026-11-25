// The cell engine on its own, for checks that mount a screensaver and watch what it does.
import { createElement } from 'react'
import { createRoot } from 'react-dom/client'
import * as engine from '@/bigpicture/screensavers/engine'
import * as savers from '@/bigpicture/screensavers/index'
import { SaverCanvas } from '@/bigpicture/screensavers/SaverCanvas'

type Fixture = {
  engine: typeof engine
  savers: typeof savers
  frames: number
  underlays: ('splash' | null)[]
  mount: (spec: string | engine.VariantSpec, opts?: Partial<engine.MountOptions>) => () => void
  show: (id: savers.SaverId) => () => void
}

const fixture: Fixture = {
  engine,
  savers,
  frames: 0,
  underlays: [],
  /** Mount `spec` (or a saver's id) over the whole window; `frames` counts the frames drawn. */
  mount(spec, opts = {}) {
    fixture.frames = 0
    fixture.underlays = []
    const canvas = document.getElementById('saver') as HTMLCanvasElement
    return engine.mountSaver(canvas, typeof spec === 'string' ? savers.saverSpec(spec) : spec, {
      fps: 20,
      titles: () => ['Hades', 'Celeste'],
      lastPlayed: () => 'Hades',
      onUnderlay: (kind) => fixture.underlays.push(kind),
      onFrame: () => fixture.frames++,
      ...opts,
    })
  },
  /** The whole screensaver as Big Picture shows it, Field's start-screen scene and all. */
  show(id) {
    // The two layout classes it uses, without the app's stylesheet.
    const style = document.head.appendChild(document.createElement('style'))
    style.textContent = '.absolute{position:absolute}.inset-0{inset:0}'
    const box = document.body.appendChild(document.createElement('div'))
    box.id = 'shown'
    box.style.cssText = 'position:fixed;inset:0;color:#ededed'
    const root = createRoot(box)
    root.render(createElement(SaverCanvas, { id, fps: 20, titles: ['Hades', 'Celeste'], lastPlayed: 'Hades' }))
    return () => {
      root.unmount()
      box.remove()
      style.remove()
    }
  },
}
;(window as unknown as { saverFixture: Fixture }).saverFixture = fixture
document.getElementById('loaded')!.textContent = 'true'
