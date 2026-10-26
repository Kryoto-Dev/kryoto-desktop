import { call, on } from '@/lib/bridge'
import { play } from '@/lib/sound'

/**
 * One-to-one voice calls (WebRTC in this view; signalling through the
 * end-to-end encrypted chat, so the DTLS fingerprints - and with them the
 * media - are end-to-end protected too).
 *
 * One call at a time. State lives here, outside React, so a call keeps going
 * while the person moves between pages; components subscribe.
 *
 * Signals, all inside the encrypted chat: `offer`, `answer`, `ice`, `hangup`,
 * `decline`, `busy`, plus `restart`/`restarted` (a new ICE round when the
 * network changes mid-call) and `muted`/`unmuted` (so the other side can show
 * it). An older app ignores the kinds it does not know, and the call simply
 * does without them.
 */

export type CallState =
  | { phase: 'idle' }
  | { phase: 'outgoing'; peer: string; callId: string }
  | { phase: 'incoming'; peer: string; callId: string }
  | { phase: 'connecting'; peer: string; callId: string }
  | { phase: 'active'; peer: string; callId: string; since: number; reconnecting: boolean }
  | { phase: 'ended'; peer: string; reason: string }

/** The call, with this side's switches and whether the other side muted. */
export type CallView = CallState & { muted: boolean; deafened: boolean; peerMuted: boolean }

type Signal = { userId: string; callId: string; kind: string; payload: string }

const RING_MS = 45_000
/** A dropped connection gets this long to come back before the call ends. */
const RECONNECT_MS = 20_000
const INPUT_KEY = 'kryo.call.input'
const OUTPUT_KEY = 'kryo.call.output'

let state: CallState = { phase: 'idle' }
let muted = false
let deafened = false
let mutedBeforeDeafen = false
let peerMuted = false
/** Who sent the first offer: only the caller starts a new ICE round, so the two never collide. */
let role: 'caller' | 'callee' = 'caller'
const listeners = new Set<(s: CallView) => void>()
let pc: RTCPeerConnection | null = null
let local: MediaStream | null = null
let remote: MediaStream | null = null
let remoteAudio: HTMLAudioElement | null = null
let pendingOffer: string | null = null
let earlyIce: RTCIceCandidateInit[] = []
let ringTimer: number | undefined
let toneTimer: number | undefined
let dropTimer: number | undefined
let restartTimer: number | undefined
let started = false

function view(): CallView {
  return { ...state, muted, deafened, peerMuted }
}

function publish() {
  const v = view()
  for (const l of listeners) l(v)
}

/** The tone that goes with each phase: ringing in, calling out, joined, ended. */
function tones(prev: CallState, next: CallState) {
  if (prev.phase === next.phase) return
  window.clearInterval(toneTimer)
  toneTimer = undefined
  // A call sounds whatever the everyday-sounds switch says (at the sound
  // volume): a silent ring is a missed call.
  if (next.phase === 'incoming') {
    play('ring', { force: true })
    toneTimer = window.setInterval(() => play('ring', { force: true }), 2600)
  } else if (next.phase === 'outgoing') {
    play('calling', { force: true })
    toneTimer = window.setInterval(() => play('calling', { force: true }), 3200)
  } else if (next.phase === 'active') {
    play('callJoin', { force: true })
  } else if (next.phase === 'ended' && prev.phase === 'active') {
    play('callEnd', { force: true })
  }
}

function set(next: CallState) {
  const prev = state
  state = next
  tones(prev, next)
  publish()
  if (next.phase === 'ended') {
    window.setTimeout(() => {
      if (state === next) set({ phase: 'idle' })
    }, 4000)
  }
}

export function callState(): CallView {
  return view()
}

export function subscribe(l: (s: CallView) => void): () => void {
  listeners.add(l)
  return () => listeners.delete(l)
}

function newCallId(): string {
  const b = new Uint8Array(16)
  crypto.getRandomValues(b)
  return Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('')
}

const signal = (peer: string, callId: string, kind: string, payload = '') =>
  call<void>('chat_call_signal', { peer, callId, kind, payload })

// ---- devices -----------------------------------------------------------------

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function write(key: string, value: string | null) {
  try {
    if (value) localStorage.setItem(key, value)
    else localStorage.removeItem(key)
  } catch {
    /* storage unavailable: the default device next time */
  }
}

export type AudioDevice = { id: string; label: string }
export type AudioDevices = { inputs: AudioDevice[]; outputs: AudioDevice[]; input: string | null; output: string | null; canPickOutput: boolean }

/** The microphones and speakers here, and which ones calls use (null: the system's). */
export async function audioDevices(): Promise<AudioDevices> {
  const all = await navigator.mediaDevices.enumerateDevices().catch(() => [] as MediaDeviceInfo[])
  const named = (kind: MediaDeviceKind, word: string) =>
    all
      .filter((d) => d.kind === kind && d.deviceId && d.deviceId !== 'default' && d.deviceId !== 'communications')
      .map((d, i) => ({ id: d.deviceId, label: d.label || `${word} ${i + 1}` }))
  return {
    inputs: named('audioinput', 'Microphone'),
    outputs: named('audiooutput', 'Speaker'),
    input: read(INPUT_KEY),
    output: read(OUTPUT_KEY),
    canPickOutput: typeof HTMLMediaElement !== 'undefined' && 'setSinkId' in HTMLMediaElement.prototype,
  }
}

async function microphone(): Promise<MediaStream> {
  const base = { echoCancellation: true, noiseSuppression: true, autoGainControl: true }
  const chosen = read(INPUT_KEY)
  if (chosen) {
    try {
      return await navigator.mediaDevices.getUserMedia({ audio: { ...base, deviceId: { exact: chosen } } })
    } catch (e) {
      // The chosen one was unplugged: the default, rather than no call.
      if (!(e instanceof Error) || !['OverconstrainedError', 'NotFoundError'].includes(e.name)) throw e
    }
  }
  return navigator.mediaDevices.getUserMedia({ audio: base })
}

/** Use this microphone from now on (and at once, in a call). */
export async function chooseInput(id: string | null): Promise<void> {
  write(INPUT_KEY, id)
  if (!pc || !local) return
  const next = await microphone()
  const track = next.getAudioTracks()[0]
  if (!track) return
  track.enabled = !muted
  await pc.getSenders().find((s) => s.track?.kind === 'audio')?.replaceTrack(track)
  local.getTracks().forEach((t) => t.stop())
  local = next
  meter.local = null
}

/** Play calls through this speaker from now on (where the browser allows it). */
export async function chooseOutput(id: string | null): Promise<void> {
  write(OUTPUT_KEY, id)
  await applyOutput()
}

async function applyOutput() {
  const el = remoteAudio as (HTMLAudioElement & { setSinkId?: (id: string) => Promise<void> }) | null
  if (!el?.setSinkId) return
  await el.setSinkId(read(OUTPUT_KEY) ?? '').catch(() => el.setSinkId?.('').catch(() => {}))
}

// ---- levels ------------------------------------------------------------------

const meter: { ctx: AudioContext | null; local: AnalyserNode | null; remote: AnalyserNode | null; data: Uint8Array<ArrayBuffer> | null } = {
  ctx: null,
  local: null,
  remote: null,
  data: null,
}

function analyse(stream: MediaStream | null): AnalyserNode | null {
  if (!stream || stream.getAudioTracks().length === 0) return null
  try {
    meter.ctx ??= new AudioContext()
    if (meter.ctx.state === 'suspended') void meter.ctx.resume()
    const node = meter.ctx.createAnalyser()
    node.fftSize = 512
    meter.ctx.createMediaStreamSource(stream).connect(node)
    return node
  } catch {
    return null
  }
}

function level(node: AnalyserNode | null): number {
  if (!node) return 0
  if (meter.data?.length !== node.fftSize) meter.data = new Uint8Array(node.fftSize)
  node.getByteTimeDomainData(meter.data)
  let sum = 0
  for (const v of meter.data) sum += ((v - 128) / 128) ** 2
  // RMS, stretched so ordinary speech reads around the middle.
  return Math.min(1, Math.sqrt(sum / meter.data.length) * 4)
}

/** How loud each side is right now, 0 to 1 (0 while muted or deafened). */
export function levels(): { local: number; remote: number } {
  if (state.phase !== 'active' && state.phase !== 'connecting') return { local: 0, remote: 0 }
  meter.local ??= analyse(local)
  meter.remote ??= analyse(remote)
  return { local: muted ? 0 : level(meter.local), remote: deafened ? 0 : level(meter.remote) }
}

/**
 * Try the chosen microphone outside a call: `onLevel` gets 0 to 1 about eight
 * times a second until the returned stop is called. Rejects with a sentence
 * when there is no microphone or no permission. During a call it only reads
 * the call's own microphone.
 */
export async function testMicrophone(onLevel: (level: number) => void): Promise<() => void> {
  if (state.phase === 'active' || state.phase === 'connecting') {
    const t = window.setInterval(() => onLevel(levels().local), 120)
    return () => window.clearInterval(t)
  }
  let stream: MediaStream
  try {
    stream = await microphone()
  } catch (e) {
    throw new Error(micError(e) === 'The call could not start.' ? 'The microphone could not be opened.' : micError(e))
  }
  const node = analyse(stream)
  const t = window.setInterval(() => onLevel(level(node)), 120)
  return () => {
    window.clearInterval(t)
    stream.getTracks().forEach((track) => track.stop())
  }
}

// ---- the connection ----------------------------------------------------------

function micError(e: unknown): string {
  const name = e instanceof Error ? e.name : ''
  if (name === 'NotAllowedError') return 'Kryoto has no access to your microphone.'
  if (name === 'NotFoundError' || name === 'OverconstrainedError') return 'No microphone was found.'
  if (name === 'NotReadableError') return 'Your microphone is in use by another app.'
  return 'The call could not start.'
}

function connected(peer: string, callId: string) {
  window.clearTimeout(dropTimer)
  window.clearTimeout(restartTimer)
  dropTimer = restartTimer = undefined
  if (state.phase === 'connecting' || state.phase === 'outgoing') set({ phase: 'active', peer, callId, since: Date.now(), reconnecting: false })
  else if (state.phase === 'active' && state.reconnecting) set({ ...state, reconnecting: false })
}

/** The network dropped mid-call: say so, try a new route, and give up after a while. */
function dropped(failed: boolean) {
  if (state.phase !== 'active') {
    if (failed && (state.phase === 'connecting' || state.phase === 'outgoing')) end('The call could not connect.', true)
    return
  }
  if (!state.reconnecting) set({ ...state, reconnecting: true })
  if (role === 'caller' && restartTimer === undefined) {
    // A short drop often heals by itself; a failed route needs a new one now.
    restartTimer = window.setTimeout(() => {
      restartTimer = undefined
      void restartIce()
    }, failed ? 0 : 2000)
  }
  dropTimer ??= window.setTimeout(() => {
    dropTimer = undefined
    if (state.phase === 'active' && state.reconnecting) end('The connection was lost.', true)
  }, RECONNECT_MS)
}

async function restartIce() {
  if (!pc || state.phase !== 'active') return
  const { peer, callId } = state
  try {
    const offer = await pc.createOffer({ iceRestart: true })
    await pc.setLocalDescription(offer)
    await signal(peer, callId, 'restart', offer.sdp ?? '')
  } catch {
    /* the give-up timer ends it if nothing comes back */
  }
}

async function connection(peer: string, callId: string): Promise<RTCPeerConnection> {
  const { iceServers } = await call<{ iceServers: RTCIceServer[] }>('chat_ice_servers').catch(() => ({
    iceServers: [{ urls: 'stun:stun.cloudflare.com:3478' }],
  }))
  const conn = new RTCPeerConnection({ iceServers })
  conn.onicecandidate = (e) => {
    if (e.candidate) void signal(peer, callId, 'ice', JSON.stringify(e.candidate.toJSON())).catch(() => {})
  }
  conn.ontrack = (e) => {
    remote = e.streams[0] ?? new MediaStream([e.track])
    meter.remote = null
    remoteAudio ??= new Audio()
    remoteAudio.autoplay = true
    remoteAudio.muted = deafened
    remoteAudio.srcObject = remote
    void applyOutput()
    void remoteAudio.play().catch(() => {})
  }
  conn.onconnectionstatechange = () => {
    if (pc !== conn) return
    if (conn.connectionState === 'connected') connected(peer, callId)
    else if (conn.connectionState === 'disconnected') dropped(false)
    else if (conn.connectionState === 'failed') dropped(true)
  }
  local = await microphone()
  meter.local = null
  for (const t of local.getTracks()) {
    t.enabled = !muted
    conn.addTrack(t, local)
  }
  return conn
}

function cleanup() {
  window.clearTimeout(ringTimer)
  window.clearTimeout(dropTimer)
  window.clearTimeout(restartTimer)
  dropTimer = restartTimer = undefined
  pc?.close()
  pc = null
  local?.getTracks().forEach((t) => t.stop())
  local = null
  remote = null
  meter.local = meter.remote = null
  if (remoteAudio) remoteAudio.srcObject = null
  pendingOffer = null
  earlyIce = []
  peerMuted = false
}

function end(reason: string, tell: boolean) {
  const peer = 'peer' in state ? state.peer : ''
  const callId = 'callId' in state ? state.callId : ''
  if (tell && peer && callId) void signal(peer, callId, 'hangup').catch(() => {})
  // The caller puts the call in the conversation, answered or not, so the
  // other person sees they were called (once per call: only the caller does).
  if (role === 'caller' && peer && callId && state.phase !== 'ended' && state.phase !== 'idle') {
    const answered = state.phase === 'active'
    const seconds = state.phase === 'active' ? Math.round((Date.now() - state.since) / 1000) : 0
    void call('chat_call_log', { peer, callId, answered, seconds }).catch(() => {})
  }
  // How long it went on, on the card that says it ended.
  const full = state.phase === 'active' && reason === 'Call ended.' ? `Call ended · ${clock(Date.now() - state.since)}` : reason
  cleanup()
  set(peer ? { phase: 'ended', peer, reason: full } : { phase: 'idle' })
}

/** Minutes and seconds (and hours, when it gets that long). */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const sec = String(s % 60).padStart(2, '0')
  return h ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`
}

/** Call someone. */
export async function startCall(peer: string): Promise<void> {
  if (state.phase !== 'idle' && state.phase !== 'ended') throw new Error('You are already in a call.')
  const callId = newCallId()
  role = 'caller'
  set({ phase: 'outgoing', peer, callId })
  try {
    pc = await connection(peer, callId)
    const offer = await pc.createOffer()
    await pc.setLocalDescription(offer)
    await signal(peer, callId, 'offer', offer.sdp ?? '')
    ringTimer = window.setTimeout(() => state.phase === 'outgoing' && end('No answer.', true), RING_MS)
  } catch (e) {
    end(micError(e), true)
  }
}

export async function accept(): Promise<void> {
  if (state.phase !== 'incoming' || !pendingOffer) return
  const { peer, callId } = state
  role = 'callee'
  window.clearTimeout(ringTimer)
  set({ phase: 'connecting', peer, callId })
  try {
    pc = await connection(peer, callId)
    await pc.setRemoteDescription({ type: 'offer', sdp: pendingOffer })
    for (const c of earlyIce) await pc.addIceCandidate(c).catch(() => {})
    earlyIce = []
    const answer = await pc.createAnswer()
    await pc.setLocalDescription(answer)
    await signal(peer, callId, 'answer', answer.sdp ?? '')
    if (muted) void signal(peer, callId, 'muted').catch(() => {})
  } catch (e) {
    end(micError(e), true)
  }
}

export function decline() {
  if (state.phase !== 'incoming') return
  void signal(state.peer, state.callId, 'decline').catch(() => {})
  cleanup()
  set({ phase: 'idle' })
}

export function hangUp() {
  if (state.phase === 'incoming') return decline()
  end(state.phase === 'outgoing' ? 'Call cancelled.' : 'Call ended.', true)
}

export function toggleMute() {
  if (state.phase === 'idle' || state.phase === 'ended') return
  muted = !muted
  if (!muted && deafened) {
    // Speaking again means hearing again.
    deafened = false
    if (remoteAudio) remoteAudio.muted = false
  }
  local?.getAudioTracks().forEach((t) => (t.enabled = !muted))
  if (state.phase !== 'incoming') void signal(state.peer, state.callId, muted ? 'muted' : 'unmuted').catch(() => {})
  publish()
}

/** Stop hearing the call, and stop sending too, like taking headphones off. */
export function toggleDeafen() {
  if (state.phase === 'idle' || state.phase === 'ended') return
  if (!deafened) {
    mutedBeforeDeafen = muted
    if (!muted) toggleMute()
    deafened = true
  } else {
    deafened = false
    if (muted && !mutedBeforeDeafen) toggleMute()
  }
  if (remoteAudio) remoteAudio.muted = deafened
  publish()
}

async function onSignal(s: Signal) {
  const mine = 'callId' in state && state.callId === s.callId
  switch (s.kind) {
    case 'offer':
      if (state.phase !== 'idle' && state.phase !== 'ended') {
        // Already in a call: say so, without disturbing this one.
        void signal(s.userId, s.callId, 'busy').catch(() => {})
        return
      }
      pendingOffer = s.payload
      earlyIce = []
      peerMuted = false
      set({ phase: 'incoming', peer: s.userId, callId: s.callId })
      ringTimer = window.setTimeout(() => state.phase === 'incoming' && end('Missed call.', false), RING_MS)
      return
    case 'answer':
      if (mine && state.phase === 'outgoing' && pc) {
        window.clearTimeout(ringTimer)
        set({ phase: 'connecting', peer: s.userId, callId: s.callId })
        await pc.setRemoteDescription({ type: 'answer', sdp: s.payload }).catch(() => end('The call could not connect.', true))
        if (muted) void signal(s.userId, s.callId, 'muted').catch(() => {})
      }
      return
    case 'restart':
      // The caller's new route after a network change: answer it.
      if (mine && pc && state.phase === 'active') {
        try {
          await pc.setRemoteDescription({ type: 'offer', sdp: s.payload })
          const answer = await pc.createAnswer()
          await pc.setLocalDescription(answer)
          await signal(s.userId, s.callId, 'restarted', answer.sdp ?? '')
        } catch {
          /* the give-up timer ends it */
        }
      }
      return
    case 'restarted':
      if (mine && pc) await pc.setRemoteDescription({ type: 'answer', sdp: s.payload }).catch(() => {})
      return
    case 'ice': {
      if (!mine) return
      let cand: RTCIceCandidateInit
      try {
        cand = JSON.parse(s.payload) as RTCIceCandidateInit
      } catch {
        return
      }
      if (pc?.remoteDescription) await pc.addIceCandidate(cand).catch(() => {})
      else earlyIce.push(cand)
      return
    }
    case 'muted':
    case 'unmuted':
      if (mine) {
        peerMuted = s.kind === 'muted'
        publish()
      }
      return
    case 'hangup':
      if (mine) end(state.phase === 'incoming' ? 'Missed call.' : 'Call ended.', false)
      return
    case 'elsewhere':
      // Picked up or turned down on another of this account's devices: stop ringing here.
      if (mine && state.phase === 'incoming') {
        cleanup()
        set({ phase: 'idle' })
      }
      return
    case 'decline':
      if (mine) end('They declined.', false)
      return
    case 'busy':
      if (mine) end('They are in another call.', false)
      return
  }
}

/** Listen for incoming signalling (once, for the life of the app). */
export function startCallListener() {
  if (started) return
  started = true
  void on<Signal>('chat-call', (s) => void onSignal(s))
  // Back online after the network dropped: try a new route at once.
  window.addEventListener('online', () => {
    if (state.phase === 'active' && role === 'caller') void restartIce()
  })
}
