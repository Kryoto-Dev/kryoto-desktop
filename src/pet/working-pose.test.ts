// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { render } from './engine.js'
import { DONE_AT, workingPose } from './working-pose.ts'

type Grid = { px: (string | null)[] }
const draw = (p: number, t: number): Grid => {
  const w = workingPose(p)
  return render('telly', 'download', t, { progress: w.progress, override: w.pose })
}
const same = (a: Grid, b: Grid) => a.px.every((k, i) => k === b.px[i])

test('a download still being checked or unpacked keeps moving (KB-0307)', () => {
  // 99%: unpacking the last of a 100 GB game. The engine's own pose at this
  // number is the arrival, arms out and nothing moving, for good.
  const times = [0, 0.25, 0.5, 0.75, 1, 1.5]
  for (const p of [0.98, 0.99, 1]) {
    const arrived = times.map((t) => render('telly', 'download', t, { progress: p }))
    assert.ok(arrived.every((f) => same(f, arrived[0]!)), `the engine's arrival moves at ${p}`)
    const frames = times.map((t) => draw(p, t))
    assert.ok(frames.some((f) => !same(f, frames[0]!)), `still at ${p}`)
    assert.ok(!same(frames[2]!, arrived[2]!), `arrived pose at ${p}`)
  }
})

test('the screen shows the real number, the body never the arrival', () => {
  assert.equal(workingPose(0.995).progress, DONE_AT)
  assert.deepEqual(workingPose(0.995).pose.screen, { mode: 'bar', v: 0.995, icon: 'down' })
  assert.equal(workingPose(0.4).progress, 0.4)
  assert.equal(workingPose(Number.NaN).progress, 0)
  assert.equal(workingPose(2).progress, DONE_AT)
})
