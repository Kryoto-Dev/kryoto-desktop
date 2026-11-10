const selector = 'button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])'
const fieldSelector = 'input:not([type]), input[type="text"], input[type="search"], input[type="url"], input[type="email"], input[type="password"], input[type="number"], textarea'
const stack: HTMLElement[] = []
const previousInert = new Map<HTMLElement, boolean>()
const watchers = new Set<() => void>()
let previousOverflow = ''

/** How many dialogs are open now. */
export const dialogCount = () => stack.length

/** Told whenever a dialog opens or closes. Returns how to stop. */
export function watchDialogs(watcher: () => void) {
  watchers.add(watcher)
  return () => {
    watchers.delete(watcher)
  }
}

function isolate() {
  const top = stack.at(-1)
  for (const child of document.body.children) {
    if (!(child instanceof HTMLElement)) continue
    if (!previousInert.has(child)) previousInert.set(child, child.inert)
    // A list or menu a control in the dialog opened (ui/index.tsx `Floating`) stays usable.
    child.inert = !!top && child !== top && !child.hasAttribute('data-kryo-float')
  }
}

export function containDialog(overlay: HTMLElement, dialog: HTMLElement, close: () => void) {
  const trigger = document.activeElement instanceof HTMLElement ? document.activeElement : null
  if (!stack.length) previousOverflow = document.body.style.overflow
  stack.push(overlay)
  watchers.forEach(watcher => watcher())
  document.body.style.overflow = 'hidden'
  isolate()
  const observer = new MutationObserver(isolate)
  observer.observe(document.body, { childList: true })
  const focusable = () => [...dialog.querySelectorAll<HTMLElement>(selector)].filter(el => el.tabIndex >= 0 && el.getClientRects().length && !el.closest('[inert]'))
  // A dialog with somewhere to type starts there, not on its close button;
  // prefilled text (a rename) is selected so typing replaces it.
  const initial = () => {
    const items = focusable()
    const field = items.find(el => el.matches(fieldSelector)) as HTMLInputElement | HTMLTextAreaElement | undefined
    if (field) {
      field.focus()
      if (field.value) field.select()
    } else dialog.focus()
  }
  if (!dialog.contains(document.activeElement)) initial()
  const key = (e: KeyboardEvent) => {
    if (stack.at(-1) !== overlay || e.defaultPrevented) return
    if (e.key === 'Escape') {
      if (dialog.querySelector('[aria-expanded="true"]')) return
      e.preventDefault()
      e.stopPropagation()
      close()
    }
    if (e.key === 'Tab') {
      const items = focusable()
      const index = items.indexOf(document.activeElement as HTMLElement)
      if (!items.length || index < 0 || (!e.shiftKey && index === items.length - 1) || (e.shiftKey && index === 0)) {
        e.preventDefault()
        ;(e.shiftKey ? items.at(-1) ?? dialog : items[0] ?? dialog).focus()
      }
    }
  }
  const focus = (e: FocusEvent) => {
    if (stack.at(-1) === overlay && !dialog.contains(e.target as Node) && !(e.target as Element).closest?.('[data-kryo-float]')) initial()
  }
  document.addEventListener('keydown', key)
  document.addEventListener('focusin', focus)
  return () => {
    observer.disconnect()
    document.removeEventListener('keydown', key)
    document.removeEventListener('focusin', focus)
    stack.splice(stack.indexOf(overlay), 1)
    watchers.forEach(watcher => watcher())
    if (stack.length) isolate()
    else {
      for (const [el, inert] of previousInert) el.inert = inert
      previousInert.clear()
      document.body.style.overflow = previousOverflow
    }
    if (trigger?.isConnected && !trigger.closest('[inert]')) trigger.focus()
  }
}
