/** How long the target stays lit. Matches `kryo-deep-link-flash` in styles.css. */
const FLASH_MS = 2000

/** The id a chat message's row carries, for a reply's line to jump to. */
export const messageAnchor = (id: string) => `chat-message-${id}`

/** Scroll to a message and light it up, as a reply's line does. */
export function jumpToMessage(id: string): void {
  const el = document.getElementById(messageAnchor(id))
  if (!el) return
  el.scrollIntoView({ behavior: 'smooth', block: 'center' })
  el.classList.remove('kryo-deep-link-target')
  void el.offsetWidth
  el.classList.add('kryo-deep-link-target')
  window.setTimeout(() => el.classList.remove('kryo-deep-link-target'), FLASH_MS)
}
