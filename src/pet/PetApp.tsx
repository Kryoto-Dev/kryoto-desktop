import { useEffect, useRef } from 'react'
import { emit } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { call, on } from '@/lib/bridge'
import type { Download, Notice } from '@/lib/downloads'
import type { MascotId, ThemeId } from '@/pet/engine'
import { Stage, petDetector } from '@/pet/stage'

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
 * Drag it anywhere; click or pet it; double-click it to open Kryoto;
 * right-click it for the native menu (pick another mascot, or hide it).
 */
export function PetApp() {
  const ref = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const canvas = ref.current
    if (!canvas) return
    const win = getCurrentWindow()
    const stage = new Stage(canvas, { mascot: 'telly', scale: 3, theme: { theme: 'monochrome', mode: 'dark' } })
    const offs: Promise<() => void>[] = []
    let shown = false
    const show = () => {
      if (shown) return
      shown = true
      void win.show()
    }

    void call<string>('pet_mascot')
      .then((m) => stage.setMascot(m as MascotId))
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
    void emit('pet-hello')

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
          if (d.status === 'installed') stage.play('success', { for: 2.2 })
          else if (d.status === 'failed') stage.play('error', { for: 2.4 })
        }
        const active = list.find((d) => ACTIVE.has(d.status))
        if (active) {
          progress =
            active.status === 'extracting' && active.extractTotal
              ? active.extracted / active.extractTotal
              : active.total
                ? active.received / active.total
                : 0
          if (!downloading) {
            downloading = true
            stage.busy = true
            stage.play('download', { loop: true, progress: () => progress })
          }
        } else if (downloading) {
          downloading = false
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
          if (first) stage.play('playing', { for: 2.4, then: () => void win.hide() })
        } else if (running.delete(e.id) && running.size === 0) {
          void win.show()
          stage.play('happy', { for: 2 })
        }
      }),
    )
    offs.push(on('achievement-unlocked', () => stage.play('achievement', { for: 3 })))
    let saving = 0
    offs.push(
      on<{ phase: string; done: number; total: number }>('cloud-saves-progress', (e) => {
        if (e.phase === 'done') {
          stage.busy = false
          stage.play('success', { for: 2 })
          return
        }
        saving = e.total ? e.done / e.total : 0
        if (stage.anim !== 'syncing') {
          stage.busy = true
          stage.play('syncing', { loop: true, progress: () => saving })
        }
      }),
    )
    offs.push(
      on<Notice>('notify', (n) => {
        if (stage.busy) return
        stage.play(n.sound === 'error' || n.sound === 'downloadFailed' ? 'error' : 'notify', { for: 2 })
      }),
    )

    // The pointer.
    const pet = petDetector(stage)
    let press: { x: number; y: number } | null = null
    const down = (e: PointerEvent) => {
      if (e.button === 0) press = { x: e.clientX, y: e.clientY }
    }
    const move = (e: PointerEvent) => {
      stage.lookAt(e.clientX, e.clientY)
      if (press && Math.hypot(e.clientX - press.x, e.clientY - press.y) > 4) {
        press = null
        const wasBusy = stage.busy
        stage.busy = true
        stage.play('dragged', { loop: true })
        // The system moves the window from here, until the button comes up.
        void win.startDragging().finally(() => {
          stage.busy = wasBusy
          stage.play('landed')
        })
        return
      }
      if (!press && e.clientY < canvas.getBoundingClientRect().height * 0.7) pet(e.clientX)
    }
    const up = () => {
      if (press) stage.poke()
      press = null
    }
    const dbl = () => void call('pet_open_app').catch(() => {})
    const menu = (e: MouseEvent) => {
      e.preventDefault()
      void call('pet_menu').catch(() => {})
    }
    canvas.addEventListener('pointerdown', down)
    canvas.addEventListener('pointermove', move)
    canvas.addEventListener('pointerup', up)
    canvas.addEventListener('dblclick', dbl)
    canvas.addEventListener('contextmenu', menu)

    return () => {
      stage.destroy()
      offs.forEach((off) => void off.then((f) => f()))
      canvas.removeEventListener('pointerdown', down)
      canvas.removeEventListener('pointermove', move)
      canvas.removeEventListener('pointerup', up)
      canvas.removeEventListener('dblclick', dbl)
      canvas.removeEventListener('contextmenu', menu)
    }
  }, [])

  return <canvas ref={ref} aria-label="Kryoto's mascot" style={{ width: 144, height: 144, imageRendering: 'pixelated', display: 'block', cursor: 'grab' }} />
}
