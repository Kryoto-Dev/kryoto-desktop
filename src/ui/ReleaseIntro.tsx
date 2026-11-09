import { useEffect, useState } from 'react'
import { BookOpen, ScrollText } from '@/ui/pixel-icons'
import { Button, Modal } from '@/ui'
import { catalogApiUrl } from '@/lib/endpoint'
import { useSettings } from '@/lib/settings'
import { logWarn } from '@/lib/log'
import { usePromptTurn } from '@/lib/prompts'
import { markSeen } from '@/ui/FeatureNotice'

/**
 * What a new version is about, told once after Kryoto Desktop updates to it:
 * the introduction video (hosted on kryo.to and played here; a YouTube video
 * also works), then the full changelog or the blog post. The video is set on
 * kryo.to (Admin > Settings, by version, /api/desktop/release-intro), so an
 * update can ship before its video: until one is set the app shows nothing,
 * and asks again at the next start.
 *
 * Not on a fresh install, nor the first time after updating to 0.3.0: the
 * setup (setup/FirstStart.tsx) offers the same video and marks it seen.
 */
export type ReleaseIntroData = {
  /** A kryo.to video's address, or a YouTube id. */
  video: string
  blog: string | null
}

/** Is it a file on kryo.to (played here) rather than a YouTube id? */
const isFile = (video: string) => video.startsWith('/') || video.startsWith('https://')

/** Its id in `settings.seenNotices`, which only takes letters, digits and hyphens. */
export const introId = (version: string = __APP_VERSION__) => `intro-${version.replace(/\./g, '-')}`

/** This version's introduction, or null while it has no video (or kryo.to cannot be reached). */
export async function fetchReleaseIntro(version: string = __APP_VERSION__): Promise<ReleaseIntroData | null> {
  try {
    const url = await catalogApiUrl(`/api/desktop/release-intro?version=${encodeURIComponent(version)}`)
    const res = await fetch(url, { cache: 'no-store', credentials: 'omit' })
    if (!res.ok) return null
    const body = (await res.json()) as { video?: string | null; blog?: string | null }
    const video = body.video ?? ''
    const blog = body.blog ?? null
    // A path is on the kryo.to this app talks to; a full address must be kryo.to's own.
    if (video.startsWith('/')) return { video: await catalogApiUrl(video), blog }
    if (isFile(video)) return /^https:\/\/([\w-]+\.)*kryo\.to\//.test(video) ? { video, blog } : null
    return /^[\w-]{11}$/.test(video) ? { video, blog } : null
  } catch (e) {
    logWarn('intro', `could not ask for the ${version} introduction: ${String(e)}`)
    return null
  }
}

/**
 * The video itself: a kryo.to file in the app's own player, or a YouTube video
 * in its privacy-enhanced player (no cookies until it plays).
 */
export function IntroVideo({ video, className }: { video: string; className?: string }) {
  return (
    <div className={`kryo-radius aspect-video w-full overflow-hidden border border-border bg-black ${className ?? ''}`}>
      {isFile(video) ? (
        <video src={video} controls autoPlay playsInline preload="metadata" className="size-full" aria-label="Kryoto Desktop introduction video" />
      ) : (
        <iframe
          src={`https://www.youtube-nocookie.com/embed/${video}?rel=0&modestbranding=1`}
          title="Kryoto Desktop introduction video"
          className="size-full"
          allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
          allowFullScreen
          referrerPolicy="strict-origin-when-cross-origin"
        />
      )}
    </div>
  )
}

export function ReleaseIntro({ onWeb }: { onWeb: (path: string) => void }) {
  const settings = useSettings()
  const [intro, setIntro] = useState<ReleaseIntroData | null>(null)
  const [closed, setClosed] = useState(false)
  const id = introId()
  const due = !!settings && settings.welcomed && !settings.seenNotices.includes(id)

  useEffect(() => {
    if (!due) return
    let alive = true
    // A moment after start, so it never lands on the boot screen.
    const t = window.setTimeout(() => void fetchReleaseIntro().then((i) => alive && setIntro(i)), 1500)
    return () => {
      alive = false
      window.clearTimeout(t)
    }
  }, [due])

  const turn = usePromptTurn('intro', due && !!intro && !closed)
  if (!intro || !turn) return null
  const close = () => {
    setClosed(true)
    markSeen(id)
  }
  return (
    <Modal
      title={`New in Kryoto Desktop ${__APP_VERSION__}`}
      onClose={close}
      wide
      footer={
        <>
          <Button variant="ghost" onClick={() => { onWeb(intro.blog ?? '/blog'); close() }}>
            <BookOpen className="size-3" />
            Read the blog
          </Button>
          <Button onClick={() => { onWeb('/changelog'); close() }}>
            <ScrollText className="size-3" />
            Read the full changelog
          </Button>
          <Button variant="primary" onClick={close}>
            Got it
          </Button>
        </>
      }
    >
      <IntroVideo video={intro.video} />
    </Modal>
  )
}
