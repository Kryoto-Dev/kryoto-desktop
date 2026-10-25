/**
 * The web chat's answers to the calls the shared chat UI makes (the same
 * command names Kryoto Desktop's Rust side implements), and the events it
 * listens for. Registered with `useWebBackend` in main.tsx.
 */

import type { Backend } from '@/lib/bridge'
import { api } from './api'
import { parseTarget, WebChat, type ChatStatus, type Person } from './engine'
import { Store } from './store'

type Handler = (payload: unknown) => void

export function createBackend(token: string, userId: string, onSignedOut: () => void) {
  const listeners = new Map<string, Set<Handler>>()
  const emit = (event: string, payload: unknown) => listeners.get(event)?.forEach((fn) => fn(payload))
  let chat: WebChat | null = null
  let status: ChatStatus = { state: 'connecting' }
  const ready = (async () => {
    const store = await Store.open()
    chat = await WebChat.open(store, token, userId, (e, p) => {
      if (e === 'chat-status') status = p as ChatStatus
      emit(e, p)
    })
    chat.run()
    return chat
  })().catch((e: unknown) => {
    status = { state: 'unavailable', reason: e instanceof Error ? e.message : String(e) }
    emit('chat-status', status)
    return null
  })

  const need = async (): Promise<WebChat> => {
    const c = await ready
    if (!c) throw new Error('Chat is not on.')
    return c
  }

  // The friends list, like the Store page reports it on the desktop.
  let friendsTimer: number | undefined
  const refreshFriends = async () => {
    try {
      const [f, h] = await Promise.all([
        api<Record<string, any>>(token, 'GET', '/api/friends'),
        api<{ usernames: string[] }>(token, 'GET', '/api/blocks/hidden').catch(() => ({ usernames: [] })),
      ])
      const face = (p: any) => ({ id: p.id, username: p.username, displayName: p.displayName ?? null, avatarUrl: p.avatarUrl ?? null, supporter: !!p.supporter })
      emit('friends-state', {
        hidden: (h.usernames ?? []).map(String),
        friends: (f.friends ?? []).map((p: any) => ({ ...face(p), favourite: !!p.favourite, nickname: p.nickname ?? null, muted: !!p.muted, online: !!p.online, playing: p.playing ?? null })),
        incoming: (f.incoming ?? []).map((p: any) => ({ ...face(p), requestId: p.requestId })),
        outgoing: (f.outgoing ?? []).length,
        sent: (f.outgoing ?? []).map((p: any) => ({ ...face(p), requestId: p.requestId })),
        messageRequests: {
          incoming: (f.messageRequests?.incoming ?? []).map(face),
          outgoing: (f.messageRequests?.outgoing ?? []).map(face),
          accepted: (f.messageRequests?.accepted ?? []).map(face),
        },
      })
    } catch (e) {
      if (e instanceof Error && 'status' in e && (e as { status: number }).status === 401) onSignedOut()
    }
  }
  void refreshFriends()
  friendsTimer = window.setInterval(() => void refreshFriends(), 30_000)

  const download = (name: string, data: BlobPart, mime: string) => {
    const url = URL.createObjectURL(new Blob([data], { type: mime }))
    const a = document.createElement('a')
    a.href = url
    a.download = name
    a.click()
    setTimeout(() => URL.revokeObjectURL(url), 10_000)
  }

  const pickFile = (): Promise<File | null> =>
    new Promise((resolve) => {
      const input = document.createElement('input')
      input.type = 'file'
      input.onchange = () => resolve(input.files?.[0] ?? null)
      input.addEventListener('cancel', () => resolve(null))
      input.click()
    })

  const handlers: Record<string, (a: Record<string, any>) => unknown> = {
    chat_status: () => status,
    chat_enable: async () => {
      if (typeof Notification !== 'undefined' && Notification.permission === 'default') void Notification.requestPermission()
      const c = await need()
      c.run()
      return c.status
    },
    chat_unlock: () => status,
    chat_remove_device: async () => {
      const c = await need()
      await c.removeDevice()
      window.clearInterval(friendsTimer)
      onSignedOut()
    },
    chat_set_context: async (a) => {
      const c = await need()
      c.people = new Map((a.people as Person[]).map((p) => [p.id, p]))
      c.meSupporter = !!a.meSupporter
    },
    chat_conversations: async () => (await need()).conversations(),
    chat_messages: async (a) => (await need()).messages(parseTarget(a.peer), a.before ?? null),
    chat_send: async (a) => (await need()).sendText(parseTarget(a.peer), a.text, a.replyTo ?? null),
    chat_send_gif: async (a) => (await need()).sendGif(parseTarget(a.peer), a.gif),
    chat_send_invite: async (a) => (await need()).sendInvite(parseTarget(a.peer), a.invite),
    // No await before the picker: it must open inside the click.
    chat_send_file: (a) =>
      pickFile().then(async (f) => (f ? (await need()).sendFile(parseTarget(a.peer), f) : null)),
    chat_file_save: async (a) => {
      const f = await (await need()).fetchFile(a.msg)
      download(f.name, f.bytes as BlobPart, f.mime)
      return f.name
    },
    chat_file_preview: async (a) => {
      const f = await (await need()).fetchFile(a.msg)
      if (!f.mime.startsWith('image/')) throw new Error('No preview for this file.')
      return URL.createObjectURL(new Blob([f.bytes as BlobPart], { type: f.mime }))
    },
    chat_edit: async (a) => (await need()).edit(a.msg, a.text),
    chat_delete: async (a) => (await need()).remove(a.msg),
    chat_react: async (a) => (await need()).react(a.msg, a.emoji, a.remove),
    chat_typing: async (a) => (await need()).typing(parseTarget(a.peer), a.active).catch(() => {}),
    chat_mark_read: async (a) => (await need()).markRead(parseTarget(a.peer)),
    chat_search: async (a) => (await need()).search(a.query),
    chat_settings_get: async () => (await need()).settings(),
    chat_settings_set: async (a) => (await need()).setSettings(a.settings),
    chat_gif_search: (a) => api(token, 'GET', `/api/gifs/search?q=${encodeURIComponent(a.query ?? '')}`),
    chat_message_request: (a) => api(token, 'POST', '/api/chat/message-requests', { username: a.username }),
    chat_message_request_respond: (a) => api(token, 'POST', `/api/chat/message-requests/${encodeURIComponent(a.user)}`, { action: a.accept ? 'accept' : 'decline' }),
    chat_people: (a) => api(token, 'GET', `/api/chat/people?ids=${(a.ids as string[]).join(',')}`),
    chat_verify_info: async (a) => (await need()).verifyInfo(a.peer),
    chat_verify_mark: async (a) => (await need()).verifyMark(a.peer, a.verified),
    chat_identity_ack: async (a) => (await need()).identityAck(a.peer),
    chat_backup_status: async () => (await need()).backupStatus(),
    chat_push_state: async () => (await need()).pushState(),
    chat_push_enable: async () => (await need()).pushEnable(),
    chat_push_disable: async () => (await need()).pushDisable(),
    chat_backup_create: async () => (await need()).backupCreate(),
    chat_backup_delete: async () => (await need()).backupDelete(),
    chat_devices: async () => (await need()).myDevices(),
    chat_device_revoke: async (a) => (await need()).revokeDevice(a.device),
    chat_restore: async (a) => (await need()).restore(a.code),
    chat_reset_identity: async () => (await need()).resetIdentity(),
    chat_groups: async () => (await need()).groupList(),
    chat_group_create: async (a) => (await need()).groupCreate(a.name, a.members),
    chat_group_rename: async (a) => (await need()).groupRename(a.group, a.name),
    chat_group_add: async (a) => (await need()).groupAdd(a.group, a.members),
    chat_group_remove: async (a) => (await need()).groupRemove(a.group, a.user),
    chat_report: async (a) => {
      const r = a.report
      const messages = await (await need()).reportMessages(r.peer, r.msgs, r.conversation ?? null)
      if (messages.length === 0) throw new Error('Those messages are not in this browser.')
      return api(token, 'POST', '/api/chat/reports', { userId: r.peer, reason: r.reason, note: r.note ?? '', messages, block: !!r.block })
    },
    chat_export_history: async () => {
      download('kryoto-chat-history.json', JSON.stringify(await (await need()).exportHistory(), null, 2), 'application/json')
      return 'your downloads'
    },
    chat_call_signal: async (a) => (await need()).sendCall(a.peer, a.callId, a.kind, a.payload),
    chat_ice_servers: () => api(token, 'GET', '/api/chat/ice'),
    online_lobby: () => null,
    room_list: (a) => api(token, 'GET', a.after ? `/api/chat/room?after=${encodeURIComponent(a.after)}` : '/api/chat/room'),
    room_post: (a) => api(token, 'POST', '/api/chat/room', { text: a.text }),
    room_delete: (a) => api(token, 'DELETE', `/api/chat/room/${encodeURIComponent(a.id)}`),
    store_refresh_account: () => refreshFriends(),
    // The friends list's own actions (src/lib/friends.ts builds the route).
    friends_fetch: async (a) => {
      const r = await api<Record<string, unknown>>(token, String(a.method), String(a.path), a.body ?? undefined)
      void refreshFriends()
      return r
    },
  }

  const backend: Backend = {
    call: async (command, args) => {
      const h = handlers[command]
      if (!h) throw new Error(`${command} is not available in the web chat.`)
      return h(args)
    },
    on: (event, handler) => {
      if (!listeners.has(event)) listeners.set(event, new Set())
      listeners.get(event)!.add(handler)
      return () => listeners.get(event)?.delete(handler)
    },
  }
  return backend
}
