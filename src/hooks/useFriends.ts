import { useEffect, useState } from 'react'
import { isTauri, isWeb, on } from '@/lib/bridge'

/**
 * The friends list, as the Store's page reports it from kryo.to's
 * `/api/friends` with the reader's own session (like the inbox: nothing here
 * holds a token). Only sent for accounts the friends feature is rolled out to.
 */
export type FriendFace = {
  id: string
  username: string
  displayName: string | null
  avatarUrl: string | null
  supporter: boolean
}
export type Friend = FriendFace & {
  favourite: boolean
  nickname: string | null
  muted: boolean
  /** Active in the last few minutes, if they share it. */
  online?: boolean
  /** What Kryoto Desktop says they are playing, if they share it. */
  playing?: { slug: string; title: string; cover: string | null } | null
}
export type FriendsState = {
  friends: Friend[]
  incoming: (FriendFace & { requestId: string })[]
  outgoing: number
  /** Requests you sent that are still waiting, so they can be taken back. */
  sent: (FriendFace & { requestId: string })[]
  /** Lower-cased usernames of people with a block either way: left out of every list. */
  hidden: string[]
  /** People who are not friends but may chat: requests both ways, and accepted ones. */
  messageRequests: MessageRequests
}
export type MessageRequests = { incoming: FriendFace[]; outgoing: FriendFace[]; accepted: FriendFace[] }
const NO_REQUESTS: MessageRequests = { incoming: [], outgoing: [], accepted: [] }

export const PREVIEW_FRIENDS: FriendsState = {
  friends: [
    { id: '2', username: 'bo', displayName: 'Bo', avatarUrl: null, supporter: true, favourite: true, nickname: 'Bobby', muted: false, online: true, playing: { slug: 'lethal-company', title: 'Lethal Company', cover: null } },
    { id: '3', username: 'ana', displayName: 'Ana', avatarUrl: null, supporter: false, favourite: false, nickname: null, muted: false, online: true },
    { id: '4', username: 'kai', displayName: null, avatarUrl: null, supporter: false, favourite: false, nickname: null, muted: true },
  ],
  incoming: [{ id: '5', username: 'cy', displayName: 'Cy', avatarUrl: null, supporter: false, requestId: '9' }],
  outgoing: 1,
  sent: [{ id: '7', username: 'nova', displayName: 'Nova', avatarUrl: null, supporter: false, requestId: '10' }],
  hidden: ['blockedperson'],
  messageRequests: {
    incoming: [{ id: '6', username: 'dee', displayName: 'Dee', avatarUrl: null, supporter: false }],
    outgoing: [],
    accepted: [],
  },
}

/** The last report, so a page opened later starts with it instead of nothing. */
let last: FriendsState | null = null

export function useFriends(): FriendsState | null {
  const [state, setState] = useState<FriendsState | null>(isTauri() || isWeb() ? last : (last ?? PREVIEW_FRIENDS))
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    void on<FriendsState>('friends-state', (s) => {
      last = {
        friends: s.friends ?? [],
        incoming: s.incoming ?? [],
        outgoing: s.outgoing ?? 0,
        sent: s.sent ?? [],
        hidden: s.hidden ?? [],
        messageRequests: { ...NO_REQUESTS, ...s.messageRequests },
      }
      setState(last)
    }).then((fn) => (cancelled ? fn() : (stop = fn)))
    return () => {
      cancelled = true
      stop?.()
    }
  }, [])
  return state
}

/** What to call a friend: your nickname for them, else their name. */
export function friendName(f: FriendFace & { nickname?: string | null }): string {
  return f.nickname || f.displayName || f.username
}
