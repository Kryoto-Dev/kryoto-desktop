import { useEffect, useState } from 'react'
import { call, isWeb, on } from '@/lib/bridge'

/** "this PC" in the app, "this browser" in the web chat. */
export const here = () => (isWeb() ? 'this browser' : 'this PC')
export const Here = () => (isWeb() ? 'This browser' : 'This PC')

/**
 * Chat on this PC, as the native side reports it (src-tauri/src/chat).
 * The keys and messages stay in the Rust process; the shell only ever sees
 * this status and, later, plaintext for display.
 */
export type ChatStatus =
  | { state: 'off' }
  | { state: 'connecting' }
  | { state: 'online'; userId: string; deviceId: string }
  | { state: 'offline'; retryInSecs: number }
  | { state: 'signInNeeded' }
  | { state: 'anonymousMode' }
  | { state: 'disabled' }
  | { state: 'needsLink' }
  | { state: 'deviceRemoved' }
  | { state: 'locked'; creating: boolean }
  | { state: 'unavailable'; reason: string }

export const chatStatus = () => call<ChatStatus>('chat_status')
export const chatEnable = () => call<ChatStatus>('chat_enable')
export const chatRemoveDevice = () => call<void>('chat_remove_device')
export const chatUnlock = (passphrase: string) => call<ChatStatus>('chat_unlock', { passphrase })
/** Take over the account's chat identity from its key backup. */
export const chatRestore = (code: string) => call<void>('chat_restore', { code })
/** Start a new chat identity for the account (contacts are warned). */
export const chatResetIdentity = () => call<void>('chat_reset_identity')

export function useChatStatus(): ChatStatus | null {
  const [status, setStatus] = useState<ChatStatus | null>(null)
  useEffect(() => {
    let alive = true
    void chatStatus().then((s) => alive && setStatus(s)).catch(() => alive && setStatus({ state: 'off' }))
    const off = on<ChatStatus>('chat-status', (s) => alive && setStatus(s))
    return () => {
      alive = false
      void off.then((f) => f())
    }
  }, [])
  return status
}

/** One line for each state, in the words a player needs. */
export function chatStatusText(s: ChatStatus): string {
  switch (s.state) {
    case 'off':
      return `Chat is off on ${here()}.`
    case 'connecting':
      return 'Connecting...'
    case 'online':
      return 'Chat is on. Messages are end-to-end encrypted.'
    case 'offline':
      return `Offline. Trying again in ${s.retryInSecs}s.`
    case 'signInNeeded':
      return 'Your chat sign-in expired. Turn chat on again to reconnect.'
    case 'anonymousMode':
      return 'You need to turn off anonymous mode to use this feature.'
    case 'disabled':
      return 'Chat is not available for your account yet.'
    case 'needsLink':
      return 'Your account already uses chat on another device. Restore it here with your recovery code, or start a new chat identity.'
    case 'deviceRemoved':
      return `${Here()} was removed from your chat devices. Remove chat here, then turn it on again.`
    case 'locked':
      return s.creating
        ? 'This system has no secure key store, so a passphrase protects chat here. Choose one.'
        : 'Chat is locked. Type your chat passphrase.'
    case 'unavailable':
      return s.reason
  }
}

// ---- messages ---------------------------------------------------------------

export type MessageStatus = 'sending' | 'sent' | 'delivered' | 'read' | 'failed' | 'received'

export type ChatMessage = {
  msgId: string
  conversationId: string
  senderUser: string
  outgoing: boolean
  sentAt: number
  receivedAt: number
  /** "text", or "gif"/"invite"/"file" with `body` as JSON. */
  kind: 'text' | 'gif' | 'invite' | 'file' | 'call'
  body: string
  replyTo: string | null
  editedAt: number | null
  deleted: boolean
  status: MessageStatus
  reactions: { emoji: string; userIds: string[] }[]
}

export type Conversation = {
  id: string
  peerUserId: string | null
  lastAt: number
  unread: number
  last: ChatMessage | null
}

export type Gif = { provider: string; id: string; url: string; width: number; height: number; title: string }

/** A game invite: the game, and the Steam lobby the host is in (Kryoto Online games). */
export type Invite = { slug: string; title: string; steamLobby: string; hostSteamId: string; expiresAt: number }

export type ChatSettings = {
  readReceipts: boolean
  typing: boolean
  notificationContent: 'full' | 'name' | 'none'
  gifsAuto: boolean
}

export const chatConversations = () => call<Conversation[]>('chat_conversations')
export const chatMessages = (peer: string, before?: number) => call<ChatMessage[]>('chat_messages', { peer, before })
export const chatSend = (peer: string, text: string, replyTo?: string | null) =>
  call<ChatMessage>('chat_send', { peer, text, replyTo: replyTo ?? null })
export const chatSendGif = (peer: string, gif: Gif) => call<ChatMessage>('chat_send_gif', { peer, gif })
/** Pick a file and send it (encrypted). Null when the dialog was cancelled. */
export const chatSendFile = (peer: string) => call<ChatMessage | null>('chat_send_file', { peer })
export const chatFileSave = (msg: string) => call<string | null>('chat_file_save', { msg })
export const chatFilePreview = (msg: string) => call<string>('chat_file_preview', { msg })
export type FileInfo = { name: string; mime: string; size: number }
export function parseFile(body: string): FileInfo | null {
  try {
    const f = JSON.parse(body) as FileInfo
    return typeof f.name === 'string' ? f : null
  } catch {
    return null
  }
}
export const chatSendInvite = (peer: string, invite: Omit<Invite, 'expiresAt'>) =>
  call<ChatMessage>('chat_send_invite', { peer, invite: { ...invite, expiresAt: 0 } })
/** The Steam lobby a running game is in, if Kryoto Online reported one. */
export const onlineLobby = (id: string) => call<{ lobby: string; hostSteamId: string } | null>('online_lobby', { id })
export const chatEdit = (msg: string, text: string) => call<ChatMessage>('chat_edit', { msg, text })
export const chatDelete = (msg: string) => call<void>('chat_delete', { msg })
export const chatReact = (msg: string, emoji: string, remove: boolean) => call<void>('chat_react', { msg, emoji, remove })
export const chatTyping = (peer: string, active: boolean) => call<void>('chat_typing', { peer, active })
export const chatMarkRead = (peer: string) => call<void>('chat_mark_read', { peer })
export const chatSearch = (query: string) => call<ChatMessage[]>('chat_search', { query })
export const chatSettingsGet = () => call<ChatSettings>('chat_settings_get')
export const chatSettingsSet = (settings: ChatSettings) => call<void>('chat_settings_set', { settings })
export const chatGifSearch = (query: string) => call<{ gifs: Gif[]; attribution?: string }>('chat_gif_search', { query })
export type ChatPerson = {
  id: string
  name: string
  supporter: boolean
  muted: boolean
  /** Their message request is waiting for your answer (no receipts or typing go back). */
  pending?: boolean
}
export const chatSetContext = (people: ChatPerson[], meSupporter: boolean) => call<void>('chat_set_context', { people, meSupporter })

/** Ask to message someone who is not a friend; kryo.to decides from their settings. */
export const chatMessageRequest = (username: string) =>
  call<{ status: 'requested' | 'accepted' | 'friends'; userId: string; username: string; name: string }>('chat_message_request', { username })
export const chatMessageRequestRespond = (user: string, accept: boolean) => call<void>('chat_message_request_respond', { user, accept })

/** Characters a message may have: what the sender may send, checked again by every receiver. */
export const textLimit = (supporter: boolean) => (supporter ? 8000 : 2000)

export const QUICK_REACTIONS = ['👍', '😂', '🔥', '❤️', '😮', '😢']

/** A finished call in the conversation (`call` rows): who called, and how it went. */
export type CallRecord = { callId: string; answered: boolean; seconds: number; caller: string }

export function parseCall(body: string): CallRecord | null {
  try {
    const c = JSON.parse(body) as CallRecord
    return typeof c.answered === 'boolean' && typeof c.caller === 'string' ? { ...c, seconds: Number(c.seconds) || 0 } : null
  } catch {
    return null
  }
}

/** What a call record says, from the reader's side. */
export function callLine(c: CallRecord, myId: string | null, peerName: string): string {
  const mine = c.caller === myId
  if (!c.answered) return mine ? 'You called · no answer' : `Missed call from ${peerName}`
  const s = Math.max(0, Math.round(c.seconds))
  const length = s >= 3600 ? `${Math.floor(s / 3600)}:${String(Math.floor((s % 3600) / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}` : `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
  return `${mine ? 'You called' : `${peerName} called`} · ${length}`
}

export function parseInvite(body: string): Invite | null {
  try {
    const i = JSON.parse(body) as Invite
    return typeof i.slug === 'string' && typeof i.title === 'string' ? i : null
  } catch {
    return null
  }
}

export function parseGif(body: string): Gif | null {
  try {
    const g = JSON.parse(body) as Gif
    return typeof g.url === 'string' ? g : null
  } catch {
    return null
  }
}

// ---- groups --------------------------------------------------------------------

export type GroupView = {
  id: string
  name: string
  members: { userId: string; role: 'owner' | 'admin' | 'member' }[]
  myRole: 'owner' | 'admin' | 'member' | ''
}
export type ChatPersonInfo = { id: string; username: string; displayName: string | null; avatarUrl: string | null; supporter: boolean }

/** How the native side names a group conversation. */
export const groupTarget = (id: string) => `g:${id}`
/** The group's conversation id, as km-core builds it: "grp:" + the id, big-endian. */
export function groupConversationId(id: string): string {
  return `6772703a${BigInt(id).toString(16).padStart(16, '0')}`
}

export const chatGroups = () => call<GroupView[]>('chat_groups')
export const chatGroupCreate = (name: string, members: string[]) => call<GroupView>('chat_group_create', { name, members })
export const chatGroupRename = (group: string, name: string) => call<void>('chat_group_rename', { group, name })
export const chatGroupAdd = (group: string, members: string[]) => call<GroupView>('chat_group_add', { group, members })
/** Remove someone; with your own id, leave the group. */
export const chatGroupRemove = (group: string, user: string) => call<void>('chat_group_remove', { group, user })
export const chatPeople = (ids: string[]) => call<{ people: ChatPersonInfo[] }>('chat_people', { ids })

// ---- verification, backup, devices -------------------------------------------

export type VerifyInfo = {
  /** Twelve groups of five digits; empty until their key is known. */
  safetyNumber: string[]
  verified: boolean
  /** Their security key changed; nothing is sent until you accept it. */
  pendingChange: boolean
}
export type BackupStatus = { exists: boolean; updatedAtMs: number; keptHere: boolean }
export type MyDevice = {
  deviceId: string
  kind: 'desktop' | 'web'
  name: string
  certified: boolean
  createdAtMs: number
  lastSeenDayMs: number
  current: boolean
}

export const chatVerifyInfo = (peer: string) => call<VerifyInfo>('chat_verify_info', { peer })
export const chatVerifyMark = (peer: string, verified: boolean) => call<void>('chat_verify_mark', { peer, verified })
export const chatIdentityAck = (peer: string) => call<void>('chat_identity_ack', { peer })
export const chatBackupStatus = () => call<BackupStatus>('chat_backup_status')

/** Browser notifications for the web chat (an empty push; see src/web/engine.ts). */
export type PushState = {
  supported: boolean
  permission: NotificationPermission | 'unsupported'
  on: boolean
  /** The web chat inside kryo.to: notices use the site's permission, while it is open. */
  embedded?: boolean
}
export const chatPushState = () => call<PushState>('chat_push_state')
/** Call straight from the click: the browser only asks for permission inside one. */
export const chatPushEnable = () => call<void>('chat_push_enable')
export const chatPushDisable = () => call<void>('chat_push_disable')
/** Returns the recovery code, shown once. */
export const chatBackupCreate = () => call<string>('chat_backup_create')
export const chatBackupDelete = () => call<void>('chat_backup_delete')
export const chatDevices = () => call<MyDevice[]>('chat_devices')
export const chatDeviceRevoke = (device: string) => call<void>('chat_device_revoke', { device })
/** Save all conversations on this PC to a file (plain text). Null when cancelled. */
export const chatExportHistory = () => call<string | null>('chat_export_history')
/** Send picked messages (by id; their text is read on the native side) to staff. */
export const chatReport = (peer: string, msgs: string[], reason: string, note: string, block: boolean, conversation?: string) =>
  call<void>('chat_report', { report: { peer, msgs, reason, note, block, conversation: conversation ?? null } })

/**
 * While someone is using this device, tell this account's other devices every
 * 45 seconds (and as soon as it is picked up), so they leave new-message
 * notifications to this one instead of all buzzing at once. `inUse` says
 * whether it is in use right now. Returns the stop.
 */
export function startActiveBeacon(inUse: () => boolean | Promise<boolean>): () => void {
  let last = 0
  const tick = async (force = false) => {
    if (!force && Date.now() - last < 40_000) return
    if (!(await inUse())) return
    last = Date.now()
    void call('chat_active').catch(() => {})
  }
  const timer = window.setInterval(() => void tick(), 45_000)
  const onFocus = () => void tick(true)
  window.addEventListener('focus', onFocus)
  return () => {
    window.clearInterval(timer)
    window.removeEventListener('focus', onFocus)
  }
}
