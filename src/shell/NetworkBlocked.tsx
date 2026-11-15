import { useEffect, useState } from 'react'
import { pokeHealth, type ServiceHealth } from '@/lib/health'
import { Button, Modal } from '@/ui'

/** What gets around a network that blocks kryo.to, in the order most players can try them. */
export const WAYS_AROUND = [
  'Turn on a VPN.',
  "Use your phone's hotspot.",
  'Change your DNS to 1.1.1.1 (Cloudflare) or 8.8.8.8 (Google).',
  'Ask your internet provider, or whoever runs this network, to unblock kryo.to.',
]

/** The one line that says it, for the notice above the footer and the Store. */
export const BLOCKED_LINE = 'Your network seems to be blocking kryo.to. A VPN, your phone’s hotspot or other DNS gets around it.'

/**
 * Said once a run, the first time this PC reaches the internet but not
 * kryo.to (health.rs `blocked`): internet providers in some countries block
 * sites like Kryoto, and so do some routers, schools and workplaces. Nothing
 * on Kryoto's side can fix that, so the player is told plainly what can.
 */
export function NetworkBlocked({ health }: { health: ServiceHealth }) {
  const [shown, setShown] = useState(false)
  const [open, setOpen] = useState(false)
  useEffect(() => {
    if (health.blocked && !shown) {
      setShown(true)
      setOpen(true)
    }
  }, [health.blocked, shown])
  if (!open) return null
  return (
    <Modal
      title="Your network is blocking kryo.to"
      onClose={() => setOpen(false)}
      footer={
        <>
          <Button
            onClick={() => {
              pokeHealth()
              setOpen(false)
            }}
          >
            Try again
          </Button>
          <Button variant="primary" onClick={() => setOpen(false)}>
            Got it
          </Button>
        </>
      }
    >
      <p className="text-sm text-foreground">
        Kryoto Desktop reaches the internet, but not kryo.to. Internet providers in some countries block sites like Kryoto, and some
        routers, school and work networks do too. Kryoto itself is up.
      </p>
      <p className="text-xs font-bold uppercase tracking-wider text-muted-foreground">What gets around it</p>
      <ul className="grid list-disc gap-1 pl-5 text-sm text-foreground marker:text-muted-foreground">
        {WAYS_AROUND.map((w) => (
          <li key={w}>{w}</li>
        ))}
      </ul>
      <p className="text-xs text-muted-foreground">
        Your library and installed games still work. Downloads, cloud saves and chat need kryo.to, and carry on by themselves once it
        answers.
      </p>
    </Modal>
  )
}
