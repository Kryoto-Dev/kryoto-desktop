import { useState } from 'react'
import { Play } from 'lucide-react'
import { Check, Section } from '@/ui'
import {
  BIG_PICTURE_SOUNDS,
  bigPictureSoundsOn,
  play,
  setBigPictureSoundsOn,
  setSoundsOn,
  setSoundVolume,
  SOUNDS,
  soundsOn,
  soundVolume,
  type SoundName,
} from '@/lib/sound'

/**
 * Settings > Sounds: the two switches (everyday sounds, off by default; Big
 * Picture's, on), one volume for all of them, and every sound with a play
 * button, so a choice is made knowing what it sounds like.
 */
export function SoundsPane() {
  const [on, setOn] = useState(soundsOn)
  const [bp, setBp] = useState(bigPictureSoundsOn)
  const [volume, setVolume] = useState(soundVolume)
  const everyday = (Object.keys(SOUNDS) as SoundName[]).filter((n) => !BIG_PICTURE_SOUNDS.includes(n))

  return (
    <>
      <Section title="Sounds" hint="Short tones for what happens in Kryoto Desktop. They only play while Kryoto Desktop is in front: behind other windows, your system's notification makes its own sound.">
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
      <Section title="Hear them">
        <SoundList names={everyday} />
      </Section>
      <Section title="Big Picture">
        <SoundList names={[...BIG_PICTURE_SOUNDS]} />
      </Section>
    </>
  )
}

function SoundList({ names }: { names: SoundName[] }) {
  return (
    <ul className="grid gap-1">
      {names.map((n) => (
        <li key={n}>
          <button
            type="button"
            onClick={() => play(n, { force: true })}
            className="kryo-pill flex h-9 w-full items-center gap-3 px-3 text-left text-xs text-foreground hover:bg-secondary focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
          >
            <Play className="size-3.5 text-muted-foreground" />
            {SOUNDS[n].label}
          </button>
        </li>
      ))}
    </ul>
  )
}
