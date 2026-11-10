import { useEffect, useState } from 'react'
import { ChevronDown, FolderOpen, Play, RotateCcw, Upload } from '@/ui/pixel-icons'
import { Button, Check, Section } from '@/ui'
import {
  BIG_PICTURE_SOUNDS,
  bigPictureSoundsOn,
  clearCustomSound,
  customSounds,
  onCustomSounds,
  play,
  setBigPictureSoundsOn,
  setCustomSound,
  setSoundsOn,
  setSoundVolume,
  SOUNDS,
  soundsOn,
  soundVolume,
  type SoundName,
} from '@/lib/sound'
import { call, errorText } from '@/lib/bridge'
import { library } from '@/lib/library'
import { pickPath } from '@/lib/pick'

const AUDIO = ['wav', 'mp3', 'ogg', 'flac', 'm4a']

/**
 * Settings > General, Sounds: the two switches (everyday sounds, off by default; Big
 * Picture's, on), one volume for all of them, and every sound with a play
 * button, so a choice is made knowing what it sounds like. Any of them can be
 * a file of the player's own, kept in a folder of its own (custom_sounds.rs).
 */
export function SoundsPane() {
  const [on, setOn] = useState(soundsOn)
  const [bp, setBp] = useState(bigPictureSoundsOn)
  const [volume, setVolume] = useState(soundVolume)
  const [note, setNote] = useState<string | null>(null)
  const everyday = (Object.keys(SOUNDS) as SoundName[]).filter((n) => !BIG_PICTURE_SOUNDS.includes(n))

  return (
    <>
      <Section title="Sounds" hint="They only play while Kryoto Desktop is in front: behind other windows, your system's notification makes its own sound.">
        <Check
          checked={on}
          onChange={(v) => {
            setOn(v)
            setSoundsOn(v)
            if (v) play('ready', { force: true })
          }}
          label="Play sounds for downloads, notifications, achievements and messages"
        />
        <Check
          checked={bp}
          onChange={(v) => {
            setBp(v)
            setBigPictureSoundsOn(v)
            if (v) play('bpSelect', { force: true })
          }}
          label="Play sounds in Big Picture"
        />
      </Section>
      <Section title="Volume">
        <label className="flex items-center gap-4 text-xs text-muted-foreground">
          <input
            type="range"
            min={0}
            max={100}
            step={5}
            value={Math.round(volume * 100)}
            aria-label="Sound volume"
            onChange={(e) => {
              const v = Number(e.target.value) / 100
              setVolume(v)
              setSoundVolume(v)
            }}
            // Hear the new level once the slider is let go.
            onPointerUp={() => play('notify', { force: true })}
            onKeyUp={() => play('notify', { force: true })}
            className="kryo-range w-64 accent-primary"
          />
          <span className="w-10 tabular-nums text-foreground">{Math.round(volume * 100)}%</span>
        </label>
      </Section>
      <details className="group grid">
        <summary className="flex w-fit cursor-pointer list-none items-center gap-1.5 text-[10px] uppercase tracking-wider text-muted-foreground transition-colors hover:text-foreground">
          <ChevronDown className="size-3 transition-transform group-open:rotate-180" aria-hidden />
          Hear or change each sound
        </summary>
        <div className="grid gap-7 pt-5">
      <Section title="Hear them" hint="Use my own plays a file of yours instead: WAV, MP3, OGG, FLAC or M4A, up to 5 MB, and at most the first 8 seconds of it.">
        <SoundList names={everyday} onNote={setNote} />
      </Section>
      <Section title="Big Picture">
        <SoundList names={[...BIG_PICTURE_SOUNDS]} onNote={setNote} />
      </Section>
      <Section title="Your own sounds" hint="Copied into a folder of their own, apart from your games and Kryoto's caches, so clearing those never removes them. The files you picked stay where they were.">
        <Button
          size="sm"
          className="w-fit"
          onClick={() =>
            void call<string>('sound_custom_folder')
              .then((dir) => library.openFolder(dir, true))
              .catch((e: unknown) => setNote(errorText(e)))
          }
        >
          <FolderOpen className="size-3" />
          Open folder
        </Button>
        {note ? <span className="text-[11px] text-destructive">{note}</span> : null}
      </Section>
        </div>
      </details>
    </>
  )
}

function useCustomSounds() {
  const [list, setList] = useState(customSounds)
  useEffect(() => onCustomSounds(() => setList(customSounds())), [])
  return list
}

function SoundList({ names, onNote }: { names: SoundName[]; onNote: (note: string | null) => void }) {
  const own = useCustomSounds()
  const choose = (name: SoundName) => {
    onNote(null)
    void pickPath({ title: `A sound for: ${SOUNDS[name].label}`, extensions: AUDIO, filterName: 'Sounds' })
      .then((path) => (path ? setCustomSound(name, path).then(() => play(name, { force: true })) : undefined))
      .catch((e: unknown) => onNote(errorText(e)))
  }
  return (
    <ul className="grid gap-1">
      {names.map((n) => {
        const mine = own.find((c) => c.name === n)
        return (
          <li key={n} className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => play(n, { force: true })}
              className="kryo-pill flex h-9 min-w-0 flex-1 items-center gap-3 px-3 text-left text-xs text-foreground hover:bg-secondary focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
            >
              <Play className="size-3.5 shrink-0 text-muted-foreground" />
              <span className="truncate">{SOUNDS[n].label}</span>
              {mine ? <span className="ml-auto truncate text-[10px] uppercase tracking-wider text-primary">{mine.file}</span> : null}
            </button>
            {mine ? (
              <Button size="sm" variant="ghost" onClick={() => clearCustomSound(n).catch((e: unknown) => onNote(errorText(e)))}>
                <RotateCcw className="size-3" />
                Reset
              </Button>
            ) : (
              <Button size="sm" variant="ghost" onClick={() => choose(n)}>
                <Upload className="size-3" />
                Use my own
              </Button>
            )}
          </li>
        )
      })}
    </ul>
  )
}
