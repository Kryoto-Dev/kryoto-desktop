/** Steam's wording for time played and when. */

export function formatPlaytime(seconds: number): string {
  if (seconds <= 0) return 'None'
  if (seconds < 60) return 'Under a minute'
  const minutes = Math.round(seconds / 60)
  if (minutes < 120) return minutes === 1 ? '1 minute' : `${minutes} minutes`
  return `${(seconds / 3600).toFixed(1)} hours`
}

export function formatLastPlayed(unix: number | null): string {
  if (!unix) return 'Never'
  const days = Math.floor((Date.now() / 1000 - unix) / 86400)
  if (days <= 0) return 'Today'
  if (days === 1) return 'Yesterday'
  if (days < 7) return `${days} days ago`
  return new Date(unix * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short' })
}
