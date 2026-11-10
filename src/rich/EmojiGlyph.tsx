import { useAssetBase } from '@/rich/asset-base'
import { emojiPath, isEmojiName, isMascotEmoji } from '@/rich/rich-catalog'
import { cn } from '@/lib/utils'

/** One of ours, by name or written `:name:`; null for anything else. */
export function ourEmojiName(emoji: string): string | null {
  const name = emoji.startsWith(':') && emoji.endsWith(':') && emoji.length > 2 ? emoji.slice(1, -1) : emoji
  return isEmojiName(name) ? name : null
}

/**
 * A single emoji outside text (a reaction, a status icon), as kryo.to's
 * components/emoji-glyph.tsx: one of ours is its animated image, a standard
 * one is itself, drawn the same size, so the two kinds sit side by side.
 */
export function EmojiGlyph({ emoji, size, className }: { emoji: string; size: number; className?: string }) {
  const base = useAssetBase()
  const name = ourEmojiName(emoji)
  if (!name) {
    return (
      <span aria-hidden style={{ width: size, height: size, fontSize: Math.round(size * 0.82) }} className={cn('pointer-events-none inline-flex shrink-0 items-center justify-center leading-none', className)}>
        <span className="kryo-emoji-char">{emoji}</span>
      </span>
    )
  }
  return <img src={`${base}${emojiPath(name)}`} alt="" width={size} height={size} draggable={false} style={{ width: size, height: size }} className={cn('pointer-events-none inline shrink-0', isMascotEmoji(name) && 'kryo-emoji-mascot', className)} />
}
