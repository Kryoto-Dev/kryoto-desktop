// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { textSizeKey, WheelSteps } from './text-size-keys.ts'

const key = (k: string, more: Partial<Parameters<typeof textSizeKey>[0]> = {}) => ({
  key: k,
  code: '',
  ctrlKey: true,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...more,
})

test('Ctrl with = + - 0 steps the text size', () => {
  assert.equal(textSizeKey(key('=')), 1)
  assert.equal(textSizeKey(key('+', { shiftKey: true })), 1)
  assert.equal(textSizeKey(key('-')), -1)
  assert.equal(textSizeKey(key('0')), 0)
  assert.equal(textSizeKey(key('+', { code: 'NumpadAdd' })), 1)
  assert.equal(textSizeKey(key('-', { code: 'NumpadSubtract' })), -1)
  // AZERTY: the 0 key without Shift is à.
  assert.equal(textSizeKey(key('à', { code: 'Digit0' })), 0)
})

test('typing never steps it', () => {
  // No Ctrl: just the character.
  assert.equal(textSizeKey(key('=', { ctrlKey: false })), null)
  // AltGr is Ctrl+Alt: AltGr+0 types } on a German keyboard.
  assert.equal(textSizeKey(key('}', { altKey: true, code: 'Digit0' })), null)
  assert.equal(textSizeKey(key('0', { altKey: true })), null)
  // Mid-composition, or a key the page already used.
  assert.equal(textSizeKey(key('=', { isComposing: true })), null)
  assert.equal(textSizeKey(key('=', { defaultPrevented: true })), null)
  // Other shortcuts are left alone.
  assert.equal(textSizeKey(key('f')), null)
  assert.equal(textSizeKey(key(')', { shiftKey: true, code: 'Digit0' })), null)
})

test('macOS uses Cmd, not Ctrl', () => {
  assert.equal(textSizeKey(key('='), true), null)
  assert.equal(textSizeKey(key('=', { ctrlKey: false, metaKey: true }), true), 1)
  assert.equal(textSizeKey(key('=', { ctrlKey: false, metaKey: true })), null)
})

test('one wheel notch is one step, a pinch adds up, turning back starts over', () => {
  const w = new WheelSteps()
  assert.equal(w.push(-100, 0, 0), 1)
  // Too soon after the last step.
  assert.equal(w.push(-100, 0, 50), null)
  assert.equal(w.push(-100, 0, 400), 1)
  assert.equal(w.push(100, 0, 800), -1)
  // Lines (WebKitGTK, Firefox): 3 lines is a notch.
  assert.equal(w.push(3, 1, 1200), -1)
  // A trackpad pinch: small turns add up.
  const p = new WheelSteps()
  assert.equal(p.push(-10, 0, 0), null)
  assert.equal(p.push(-10, 0, 10), null)
  assert.equal(p.push(20, 0, 20), null)
  assert.equal(p.push(-60, 0, 30), 1)
})
