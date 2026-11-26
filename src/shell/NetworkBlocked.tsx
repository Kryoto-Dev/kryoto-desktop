import { useEffect, useState } from 'react'
import { pokeHealth, type ServiceHealth } from '@/lib/health'
import { isLinuxHost, isWindows11, isWindowsHost } from '@/lib/library'
import { ChevronDown } from '@/ui/pixel-icons'
import { Button, Modal } from '@/ui'

/** The DNS way, which the pop-up follows with how to do it. */
const DNS_WAY = 'Change your DNS to 1.1.1.1 (Cloudflare) or 8.8.8.8 (Google).'

/** What gets around a network that blocks kryo.to, in the order most players can try them. */
export const WAYS_AROUND = [
  'Turn on a VPN.',
  "Use your phone's hotspot.",
  DNS_WAY,
  'Ask your internet provider, or whoever runs this network, to unblock kryo.to.',
]

/**
 * The three things that matter, said on the screen itself: a player whose
 * network blocks kryo.to may not reach a help page on kryo.to either.
 */
export const ESSENTIAL_STEPS = [
  'Try another DNS, such as 1.1.1.1 (Cloudflare).',
  'Or turn on a VPN.',
  'Your Library and installed games keep working meanwhile.',
]

/**
 * kryo.to's page on what to do when a network blocks it ("More on kryo.to",
 * for when it loads). A constant, so its final address can be set in one place.
 */
export const BLOCKED_HELP_URL = 'https://kryo.to/help/blocked'

/** The one line that says it, for the notice above the footer and the Store. */
export const BLOCKED_LINE = 'Your network seems to be blocking kryo.to. A VPN, your phone’s hotspot or other DNS gets around it.'

/**
 * Windows 11's encrypted DNS, step by step (Settings, or four lines in a
 * terminal). Encrypted, so a provider that rewrites plain DNS answers cannot
 * see or change it. 1.1.1.1's DNS over HTTPS address is built into Windows 11.
 * IPv6 too: a network that gives it out would otherwise keep asking the
 * provider's own DNS.
 */
export const WINDOWS_DNS_STEPS = [
  'Open Settings, then Network & internet, then your connection (Wi-Fi or Ethernet). For Wi-Fi, open Hardware properties.',
  'Next to DNS server assignment, choose Edit, then Manual.',
  'Turn on IPv4. Preferred DNS: 1.1.1.1, Alternate DNS: 1.0.0.1, each with DNS over HTTPS: On (automatic template).',
  'Turn on IPv6. Preferred DNS: 2606:4700:4700::1111, Alternate DNS: 2606:4700:4700::1001, each with DNS over HTTPS: On (automatic template). Then Save.',
]
const DOH = 'dohtemplate=https://cloudflare-dns.com/dns-query autoupgrade=yes udpfallback=no'
const SET_DNS = [
  'netsh interface ipv4 set dnsservers name="Wi-Fi" source=static address=1.1.1.1 validate=no',
  'netsh interface ipv6 set dnsservers name="Wi-Fi" source=static address=2606:4700:4700::1111 validate=no',
]
export const WINDOWS_DNS_COMMANDS = [...SET_DNS, `netsh dns set encryption server=1.1.1.1 ${DOH}`, `netsh dns set encryption server=2606:4700:4700::1111 ${DOH}`]
/**
 * Windows 10 has no encrypted DNS: its settings and netsh change plain DNS
 * only, which still gets around a provider that does no more than answer
 * wrongly for kryo.to.
 */
export const WINDOWS_10_DNS_STEPS = [
  'Open Control Panel, then Network and Sharing Center, then Change adapter settings.',
  'Right-click your connection, choose Properties, then Internet Protocol Version 4 (TCP/IPv4), then Properties.',
  'Choose Use the following DNS server addresses: 1.1.1.1 and 1.0.0.1. Then OK.',
  'Do the same for Internet Protocol Version 6 (TCP/IPv6) with 2606:4700:4700::1111 and 2606:4700:4700::1001.',
]
export const WINDOWS_10_DNS_COMMANDS = SET_DNS
/** systemd-resolved, which most Linux desktops use, does DNS over TLS. */
export const LINUX_DNS_LINE =
  'On Linux with systemd-resolved: in /etc/systemd/resolved.conf set DNS=1.1.1.1#cloudflare-dns.com 2606:4700:4700::1111#cloudflare-dns.com and DNSOverTLS=yes, then run sudo systemctl restart systemd-resolved.'
/** What DNS cannot do, so a player whose block is deeper does not keep trying it. */
export const DNS_LIMIT =
  'Your provider can intercept a plain DNS change, but not an encrypted one (DNS over HTTPS or TLS). If the block is on kryo.to’s name in the connection itself or on its address (an SNI or IP block), no DNS gets around it: use a VPN or another network, such as your phone’s hotspot.'

/** The same change as lines to paste, for a player who would rather. */
function Commands({ lines }: { lines: string[] }) {
  return (
    <>
      <p>Or in a terminal run as administrator, with your connection’s name in place of Wi-Fi:</p>
      <code className="kryo-radius block select-all whitespace-pre-wrap break-all border border-border bg-background px-2 py-1.5 font-mono text-[11px] text-foreground">
        {lines.join('\n')}
      </code>
    </>
  )
}

/**
 * How to change DNS, on the screen itself: the help page on kryo.to is on the
 * very name the network blocks, so everything needed is said here. Windows 10
 * gets its own steps: it has no encrypted DNS, and 11's are not there.
 */
function DnsHowTo() {
  const windows = isWindowsHost()
  const [eleven, setEleven] = useState(true)
  useEffect(() => {
    if (windows) void isWindows11().then(setEleven)
  }, [windows])
  return (
    <details className="group mt-1">
      <summary className="inline-flex cursor-pointer list-none items-center gap-1.5 text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary group-open:text-foreground [&::-webkit-details-marker]:hidden">
        <ChevronDown className="size-3 transition-transform group-open:rotate-180" aria-hidden />
        Show me how
      </summary>
      <div className="mt-1.5 grid gap-1.5 text-xs leading-relaxed text-muted-foreground">
        {windows ? (
          <>
            <p className="text-foreground">{eleven ? 'On Windows 11, encrypted DNS:' : 'On Windows 10:'}</p>
            <ol className="grid list-decimal gap-0.5 pl-4">
              {(eleven ? WINDOWS_DNS_STEPS : WINDOWS_10_DNS_STEPS).map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ol>
            <Commands lines={eleven ? WINDOWS_DNS_COMMANDS : WINDOWS_10_DNS_COMMANDS} />
          </>
        ) : null}
        {isLinuxHost() ? <p>{LINUX_DNS_LINE}</p> : null}
        <p>{DNS_LIMIT}</p>
      </div>
    </details>
  )
}

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
    // kryo.to answers again: nothing is blocking it any more, so the
    // pop-up does not stay saying it is.
    if (!health.blocked && open) setOpen(false)
  }, [health.blocked, shown, open])
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
          <li key={w}>
            {w}
            {w === DNS_WAY ? <DnsHowTo /> : null}
          </li>
        ))}
      </ul>
      <p className="text-xs text-muted-foreground">
        Your library and installed games still work. Downloads, cloud saves and chat need kryo.to, and carry on by themselves once it
        answers.
      </p>
    </Modal>
  )
}
