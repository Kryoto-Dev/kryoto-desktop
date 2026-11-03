import { useEffect, useRef, useState } from 'react'
import { Download, Mic } from 'lucide-react'
import { errorText } from '@/lib/bridge'
import { audioDevices, chooseInput, chooseOutput, testMicrophone, type AudioDevice, type AudioDevices } from '@/lib/calls'
import { chatExportHistory, chatSettingsGet, chatSettingsSet, here, useChatStatus, type ChatSettings } from '@/lib/chat'
import { asciiTrack, Button, Check as Toggle, Dropdown, Modal, Section, Segmented } from '@/ui'
import { ChatSetup } from './ChatSetup'

/** "The system's" device, as a choice in the lists. */
const SYSTEM = '__system__'

/**
 * Everything about chat and calls in one place: chat on this device (keys,
 * backup, devices), how messages behave, what notifications show, the
 * microphone and speaker calls use, and your history. The same panel in
 * Kryoto Desktop's Settings and behind the friends list's gear.
 */
export function ChatSettingsPanel({ available, calls }: { available: boolean; calls: boolean }) {
  const status = useChatStatus()
  const online = status?.state === 'online'
  const [settings, setSettings] = useState<ChatSettings | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [exported, setExported] = useState<string | null>(null)

  useEffect(() => {
    if (!online) return
    let alive = true
    void chatSettingsGet()
      .then((s) => alive && setSettings(s))
      .catch(() => {})
    return () => {
      alive = false
    }
  }, [online])

  const save = (next: ChatSettings) => {
    setSettings(next)
    void chatSettingsSet(next).catch((e) => setError(errorText(e)))
  }

  return (
    <div className="grid gap-7">
      {/* The card names itself ("Chat on this PC"). */}
      <ChatSetup available={available} />

      {online && settings ? (
        <Section title="Messages">
          <Toggle checked={settings.readReceipts} onChange={(v) => save({ ...settings, readReceipts: v })} label="Read receipts: tell people when you have read their messages (and see theirs)" />
          <Toggle checked={settings.typing} onChange={(v) => save({ ...settings, typing: v })} label='Typing indicators: show "typing..." both ways' />
          <Toggle checked={settings.gifsAuto} onChange={(v) => save({ ...settings, gifsAuto: v })} label="Show GIFs straight away (the GIF provider then sees your IP address)" />
        </Section>
      ) : null}

      {online && settings ? (
        <Section title="Notifications show">
          <Segmented
            value={settings.notificationContent}
            onChange={(v) => save({ ...settings, notificationContent: v })}
            options={[
              { value: 'name', label: 'Who it is from' },
              { value: 'full', label: 'Name and message' },
              { value: 'none', label: 'Nothing' },
            ]}
          />
        </Section>
      ) : null}

      {calls ? <CallDevices /> : null}

      {online ? (
        <Section title="Your messages" hint={`They are kept encrypted on ${here()}, not by Kryoto. Save a copy to keep them somewhere else.`}>
          <div className="flex flex-wrap items-center gap-3">
            <Button
              size="sm"
              onClick={() =>
                void chatExportHistory()
                  .then((where) => setExported(where ? `Saved to ${where}.` : null))
                  .catch((e) => setError(errorText(e)))
              }
            >
              <Download className="size-3" aria-hidden />
              Export history
            </Button>
            {exported ? <span className="text-[11px] text-muted-foreground">{exported}</span> : null}
          </div>
        </Section>
      ) : null}

      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
  )
}

function choices(list: AudioDevice[]): { value: string; label: string }[] {
  return [{ value: SYSTEM, label: 'Same as the system' }, ...list.map((x) => ({ value: x.id, label: x.label }))]
}

function chosen(list: AudioDevice[], id: string | null): string {
  return id && list.some((x) => x.id === id) ? id : SYSTEM
}

/** The microphone and speaker calls use, and a microphone test with its level. */
function CallDevices() {
  const [d, setD] = useState<AudioDevices | null>(null)
  const [testing, setTesting] = useState(false)
  const [level, setLevel] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const stop = useRef<(() => void) | null>(null)

  const load = () => void audioDevices().then(setD).catch(() => setD(null))
  useEffect(() => {
    load()
    navigator.mediaDevices?.addEventListener?.('devicechange', load)
    return () => {
      navigator.mediaDevices?.removeEventListener?.('devicechange', load)
      stop.current?.()
    }
  }, [])

  const toggleTest = async () => {
    setError(null)
    if (stop.current) {
      stop.current()
      stop.current = null
      setTesting(false)
      setLevel(0)
      return
    }
    try {
      stop.current = await testMicrophone(setLevel)
      setTesting(true)
      // Names appear once the microphone has been allowed.
      load()
    } catch (e) {
      setError(errorText(e))
    }
  }

  const restartTest = async () => {
    if (!stop.current) return
    stop.current()
    stop.current = await testMicrophone(setLevel).catch(() => null)
  }

  if (!d) return null
  const unnamed = d.inputs.length > 0 && d.inputs.every((x) => /^Microphone \d+$/.test(x.label))
  return (
    <Section title="Calls" hint="Calls ring at the sound volume in Settings > Sounds, even with everyday sounds off.">
      <div className="grid gap-1.5">
        <span className="text-xs text-foreground">Microphone</span>
        <Dropdown
          label="Microphone"
          value={chosen(d.inputs, d.input)}
          options={choices(d.inputs)}
          onChange={(v) =>
            void chooseInput(v === SYSTEM ? null : v)
              .then(() => audioDevices().then(setD))
              .then(restartTest)
              .catch((e) => setError(errorText(e)))
          }
        />
        {unnamed ? <span className="text-[11px] text-muted-foreground">Test the microphone once to see the devices by name.</span> : null}
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <Button size="sm" onClick={() => void toggleTest()} aria-pressed={testing}>
          <Mic className="size-3" aria-hidden />
          {testing ? 'Stop the test' : 'Test microphone'}
        </Button>
        {testing ? (
          <span className="kryo-ascii-art whitespace-pre text-[11px] leading-none text-muted-foreground" aria-label={level > 0.08 ? 'Hearing you' : 'Quiet'}>
            {asciiTrack(level, 24)}
          </span>
        ) : null}
      </div>
      {d.canPickOutput ? (
        <div className="grid gap-1.5">
          <span className="text-xs text-foreground">Speaker</span>
          <Dropdown
            label="Speaker"
            value={chosen(d.outputs, d.output)}
            options={choices(d.outputs)}
            onChange={(v) =>
              void chooseOutput(v === SYSTEM ? null : v)
                .then(() => audioDevices().then(setD))
                .catch((e) => setError(errorText(e)))
            }
          />
        </div>
      ) : null}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </Section>
  )
}

/** The same panel as a dialog, for the friends list's gear. */
export function ChatSettingsDialog({ available, calls, onClose }: { available: boolean; calls: boolean; onClose: () => void }) {
  return (
    <Modal title="Chat & calls" onClose={onClose}>
      <ChatSettingsPanel available={available} calls={calls} />
    </Modal>
  )
}
