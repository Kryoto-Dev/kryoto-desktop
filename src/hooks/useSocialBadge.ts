import { useEffect, useState } from 'react'
import { on } from '@/lib/bridge'
import { chatConversations, type Conversation } from '@/lib/chat'
import { useFriends } from '@/hooks/useFriends'

export type SocialBadge = { friendRequests: number; messageRequests: number; unread: number; total: number }

/**
 * How many things in Friends & chat wait for you: friend requests, message
 * requests and unread messages (not from people you muted). The number on
 * the Friends & chat button, here and on kryo.to.
 */
export function useSocialBadge(): SocialBadge {
  const friends = useFriends()
  const [conversations, setConversations] = useState<Conversation[]>([])
  useEffect(() => {
    let alive = true
    const refresh = () =>
      void chatConversations()
        .then((list) => alive && setConversations(list))
        .catch(() => alive && setConversations([]))
    refresh()
    const stops = [on('chat-message', refresh), on('chat-read', refresh), on('chat-updated', refresh), on('chat-status', refresh)]
    return () => {
      alive = false
      stops.forEach((p) => void p.then((f) => f()))
    }
  }, [])
  const muted = new Set((friends?.friends ?? []).filter((f) => f.muted).map((f) => f.id))
  const unread = conversations.reduce((n, c) => n + (c.peerUserId && muted.has(c.peerUserId) ? 0 : c.unread), 0)
  const friendRequests = friends?.incoming.length ?? 0
  const messageRequests = friends?.messageRequests.incoming.length ?? 0
  return { friendRequests, messageRequests, unread, total: friendRequests + messageRequests + unread }
}
