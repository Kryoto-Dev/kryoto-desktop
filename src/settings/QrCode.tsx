import { useMemo } from 'react'
import { create } from 'qrcode'

/**
 * A QR code as an SVG, drawn from the modules rather than inserted as markup.
 * Always black on white with a quiet zone, whatever the palette: scanners read
 * dark on light, and an inverted code in the dark theme often fails.
 */
export function QrCode({ text, label, className }: { text: string; label: string; className?: string }) {
  const { size, path } = useMemo(() => {
    const { modules } = create(text, { errorCorrectionLevel: 'M' })
    const n = modules.size
    let d = ''
    for (let y = 0; y < n; y++) {
      for (let x = 0; x < n; x++) {
        if (modules.get(y, x)) d += `M${x} ${y}h1v1h-1z`
      }
    }
    return { size: n, path: d }
  }, [text])
  const quiet = 4
  const box = size + quiet * 2
  return (
    <svg role="img" aria-label={label} viewBox={`0 0 ${box} ${box}`} shapeRendering="crispEdges" className={className}>
      <rect width={box} height={box} fill="#fff" />
      <path transform={`translate(${quiet} ${quiet})`} d={path} fill="#000" />
    </svg>
  )
}
