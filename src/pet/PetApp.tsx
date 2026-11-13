import { useEffect, useRef, useState } from 'react'
import { emit } from '@tauri-apps/api/event'
import { cursorPosition, getCurrentWindow } from '@tauri-apps/api/window'
import { call, on } from '@/lib/bridge'
import { progressOf, type Download, type Notice } from '@/lib/downloads'
import { isOnline, onOnlineChange } from '@/lib/online'
import type { MascotId, ThemeId } from '@/pet/engine'
import { Stage, petDetector } from '@/pet/stage'
import { workingPose } from '@/pet/working-pose'
import { PET_SIZES, type PetSize } from '@/lib/settings'

type Look = { palette: string; dark: boolean; reducedMotion: boolean }

const ACTIVE = new Set(['resolving', 'downloading', 'verifying', 'extracting'])

/**
 * The desktop pet's page, in its own see-through window (src-tauri/src/pet.rs).
 *
 * It acts out what Kryoto is doing: a download's progress on the way in, a
 * cheer when it is installed and a wince when it fails, a trophy for an
 * achievement, the cloud for a save backup. It steps out of the way while a
 * game runs, since it sits above every other window.
 *
 * It behaves as the site's mascot does (kryo.to components/mascot): only its
 * drawn pixels take the pointer, its eyes follow the pointer, a stroke over
 * its head pets it, a click pokes it (now and then it takes the sponsor's
 * taco break), a double-click spins it into a dance, and it dangles its legs
 * while it is carried. Right-click it for the native menu (another mascot,
 * open Kryoto, or hide it).
 */
export function PetApp() {
  const ref = useRef<HTMLCanvasElement>(null)
  // How big it is drawn (Settings > Desktop pet > Size). The window stays 144
  // px; a smaller pet stands at its bottom, on the taskbar like the big one.
  const [size, setSize] = useState<PetSize>('normal')
  useEffect(() => {
    void call<string>('pet_size').then((z) => setSize(z as PetSize)).catch(() => {})
    const off = on<string>('pet-size', (z) => setSize(z as PetSize))
    return () => void off.then((f) => f()).catch(() => {})
  }, [])
  const px = 144 * (PET_SIZES.find((z) => z.id === size)?.scale ?? 1)

  useEffect(() => {
    const canvas = ref.current
    if (!canvas) return
    const win = getCurrentWindow()
    const stage = new Stage(canvas, { mascot: 'telly', scale: 3, theme: { theme: 'monochrome', mode: 'dark' } })
    // Every message to the native side can fail while this window is being
    // closed (the pet put away, Kryoto quitting): "failed to send message to
    // the webview", 357 reports in two weeks. None of them matters then.
    const quiet = <T,>(p: Promise<T>) => p.catch(() => undefined as T)
    const offs: Promise<() => void>[] = []
    let shown = false
    const show = () => {
      if (shown) return
      shown = true
      void quiet(win.show())
    }

    void call<string>('pet_mascot')
      .then((m) => stage.setMascot(m as MascotId))
      .catch(() => {})
      .finally(show)
    offs.push(
      on<string>('pet-mascot', (m) => {
        stage.setMascot(m as MascotId)
        stage.play('happy', { for: 1.8 })
      }),
    )
    // The palette is the main window's (App.tsx); it is asked for once here.
    offs.push(
      on<Look>('pet-look', (look) => {
        stage.theme = { theme: look.palette as ThemeId, mode: look.dark ? 'dark' : 'light' }
        if (look.reducedMotion) document.documentElement.setAttribute('data-motion', 'reduced')
        else document.documentElement.removeAttribute('data-motion')
        stage.redraw()
      }),
    )
    void quiet(emit('pet-hello'))

    // Being carried: nothing else plays until it lands.
    let held = false
    // The loop it was busy with (a download, a save backup), to go back to
    // after it has been carried and landed.
    let task: (() => void) | null = null

    // Downloads: the busiest one's progress, then how it ended.
    let progress = 0
    let downloading = false
    const seen = new Map<string, string>()
    offs.push(
      on<Download[]>('downloads', (list) => {
        for (const d of list) {
          const before = seen.get(d.id)
          seen.set(d.id, d.status)
          if (!before || before === d.status) continue
          if (held) continue
          if (d.status === 'installed') stage.play('success', { for: 2.2 })
          else if (d.status === 'failed') stage.play('error', { for: 2.4 })
        }
        const active = list.find((d) => ACTIVE.has(d.status))
        if (active) {
          // The Downloads page's own split (fetching, checking, unpacking).
          // Checking and unpacking used to read the fetched bytes, 100%, so
          // a big game held the "arrived" pose for minutes (KB-0307).
          progress = progressOf(active)
          if (!downloading) {
            downloading = true
            task = () =>
              stage.play('download', {
                loop: true,
                progress: () => workingPose(progress).progress,
                pose: () => workingPose(progress).pose,
              })
            stage.busy = true
            if (!held) task()
          }
        } else if (downloading) {
          downloading = false
          task = null
          if (held) return
          stage.busy = false
          if (stage.anim === 'download') stage.back()
        }
      }),
    )

    // A game on screen: wave it off, step aside, come back when it closes.
    const running = new Set<string>()
    offs.push(
      on<{ id: string; running: boolean }>('game-state', (e) => {
        if (e.running) {
          const first = running.size === 0
          running.add(e.id)
          if (first && held) void quiet(win.hide())
          else if (first) stage.play('playing', { for: 2.4, then: () => void quiet(win.hide()) })
        } else if (running.delete(e.id) && running.size === 0) {
          void quiet(win.show())
          if (!held) stage.play('happy', { for: 2 })
        }
      }),
    )
    offs.push(
      on('achievement-unlocked', () => {
        if (!held) stage.play('achievement', { for: 3 })
      }),
    )
    let saving = 0
    let backingUp = false
    offs.push(
      on<{ phase: string; done: number; total: number }>('cloud-saves-progress', (e) => {
        if (e.phase === 'done') {
          backingUp = false
          if (!downloading) task = null
          if (held) return
          stage.busy = task !== null
          stage.play('success', { for: 2, then: () => (task ? task() : stage.back()) })
          return
        }
        saving = e.total ? e.done / e.total : 0
        if (!backingUp) {
          backingUp = true
          task = () => stage.play('syncing', { loop: true, progress: () => saving })
          stage.busy = true
          if (!held) task()
        }
      }),
    )
    // The PC's memory nearly full (pet.rs reads it): it eats some, when it
    // is not busy with something of Kryoto's.
    offs.push(
      on('pet-hungry', () => {
        if (stage.busy || held) return
        stage.play('ramsnack')
      }),
    )
    offs.push(
      on<Notice>('notify', (n) => {
        if (stage.busy || held) return
        stage.play(n.sound === 'error' || n.sound === 'downloadFailed' ? 'error' : 'notify', { for: 2 })
      }),
    )

    // Gone offline, as the site's mascot shows it.
    const offline = () => {
      if (held) return
      stage.busy = true
      stage.play('offline', { loop: true })
    }
    const online = () => {
      if (stage.anim !== 'offline') return
      stage.busy = task !== null
      stage.play('success', { for: 2, then: () => (task ? task() : stage.back()) })
    }
    // Through lib/online.ts, which confirms "offline" with a real request:
    // WebKitGTK says offline on Linux without NetworkManager while the
    // network works, and the pet sat in its offline loop for good.
    const stopOnline = onOnlineChange((up) => (up ? online() : offline()))
    if (!isOnline()) offline()

    // Carried: the system moves the window (so it works on Wayland too) and
    // pet.rs says when the button comes up. Should that command be missing
    // (a page newer than the app around it), the window still moves, and it
    // lands at the first pointer move after.
    const drop = () => {
      if (!held) return
      held = false
      stage.busy = task !== null
      stage.play('landed', { then: () => (task ? task() : stage.back()) })
    }
    const pickUp = () => {
      held = true
      stage.busy = true
      stage.play('dragged', { loop: true })
      void call('pet_drag').catch(() => quiet(win.startDragging()))
    }
    offs.push(on('pet-dropped', drop))

    // The pointer. Only the drawn pixels take it, as on the site. The page
    // hears it only over its own small window, so the eyes ask where it is.
    const pet = petDetector(stage)
    let press: { x: number; y: number } | null = null
    const down = (e: PointerEvent) => {
      if (held) return drop()
      if (e.button === 0 && stage.opaqueAt(e.clientX, e.clientY)) press = { x: e.clientX, y: e.clientY }
    }
    const move = (e: PointerEvent) => {
      // The pointer moving with no button down: the carrying is over.
      if (held) return e.buttons === 0 ? drop() : undefined
      stage.lookAt(e.clientX, e.clientY)
      const over = stage.opaqueAt(e.clientX, e.clientY)
      canvas.style.cursor = over || press ? 'grab' : 'default'
      if (press && Math.hypot(e.clientX - press.x, e.clientY - press.y) > 4) {
        press = null
        return pickUp()
      }
      if (!press && over && e.clientY < canvas.getBoundingClientRect().height * 0.7) pet(e.clientX)
    }
    const up = () => {
      if (press) stage.poke()
      press = null
    }
    const dbl = (e: MouseEvent) => {
      if (stage.opaqueAt(e.clientX, e.clientY)) stage.play('spin', { then: () => stage.play('dance', { for: 3 }) })
    }
    const menu = (e: MouseEvent) => {
      e.preventDefault()
      void call('pet_menu').catch(() => {})
    }
    let scale = window.devicePixelRatio || 1
    void quiet(win.scaleFactor()).then((f) => (scale = f || scale))
    offs.push(win.onScaleChanged(({ payload }) => (scale = payload.scaleFactor)))
    const follow = window.setInterval(() => {
      if (held || press || document.hidden) return
      void Promise.all([cursorPosition(), win.innerPosition()])
        .then(([c, at]) => stage.lookAt((c.x - at.x) / scale, (c.y - at.y) / scale))
        .catch(() => {})
    }, 150)
    canvas.addEventListener('pointerdown', down)
    canvas.addEventListener('pointermove', move)
    canvas.addEventListener('pointerup', up)
    canvas.addEventListener('dblclick', dbl)
    canvas.addEventListener('contextmenu', menu)

    return () => {
      window.clearInterval(follow)
      stopOnline()
      stage.destroy()
      offs.forEach((off) => void quiet(off.then((f) => f())))
      canvas.removeEventListener('pointerdown', down)
      canvas.removeEventListener('pointermove', move)
      canvas.removeEventListener('pointerup', up)
      canvas.removeEventListener('dblclick', dbl)
      canvas.removeEventListener('contextmenu', menu)
    }
  }, [])

  return (
    <div style={{ width: 144, height: 144, display: 'flex', alignItems: 'flex-end', justifyContent: 'center' }}>
      <canvas ref={ref} aria-label="Kryoto's mascot" style={{ width: px, height: px, imageRendering: 'pixelated', display: 'block', cursor: 'grab' }} />
    </div>
  )
}
