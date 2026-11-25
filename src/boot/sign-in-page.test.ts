// `pnpm test`: node's own runner, which reads TypeScript as it is.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { onSignInPage, settledOn } from './sign-in-page.ts'

const at = (url: string, more: { loading?: boolean; error?: unknown } = {}) => ({ url, loading: false, error: null, ...more })

test('the sign-in pages keep the box open', () => {
  assert.equal(onSignInPage('https://kryo.to/login?next=/'), true)
  assert.equal(onSignInPage('https://kryo.to/verify-email'), true)
  assert.equal(onSignInPage('https://kryo.to/'), false)
  assert.equal(onSignInPage('https://kryo.to/loginhelp'), false)
})

test('a page sent back to a sign-in page counts as there once it has loaded', () => {
  // Signed in, it moves on; the site then bounces it back to have the email checked.
  assert.equal(settledOn(at('https://kryo.to/'), true), 'elsewhere')
  assert.equal(settledOn(at('https://kryo.to/verify-email', { loading: true }), true), null)
  assert.equal(settledOn(at('https://kryo.to/verify-email'), true), 'sign-in')
  // Not opened yet, or failed: neither.
  assert.equal(settledOn(at('https://kryo.to/login'), false), null)
  assert.equal(settledOn(at('https://kryo.to/login', { error: { message: 'offline' } }), true), null)
})
