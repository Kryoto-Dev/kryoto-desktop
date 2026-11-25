// Requires Playwright with Chromium. Run against `pnpm dev`: node scripts/check-ui.mjs.
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
const { chromium } = createRequire(import.meta.url)('playwright')
const browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_BROWSER_CHANNEL ? { channel: process.env.PLAYWRIGHT_BROWSER_CHANNEL } : {}) })
const page = await browser.newPage({ viewport: { width: 1280, height: 1000 } })
const base = process.env.DESKTOP_TEST_URL ?? 'http://127.0.0.1:1421'
const failures = []
page.on('pageerror', e => failures.push(e.message))
const check = async (name, fn) => { await fn(); console.log(`PASS ${name}`) }
const expectText = async (selector, text) => page.waitForFunction(([s, t]) => document.querySelector(s)?.textContent?.includes(t), [selector, text])
try {
  await check('pending launch cancellation returns Stop to Play without a process event', async () => {
    const launch = await browser.newPage()
    launch.on('pageerror', e => failures.push(e.message))
    await launch.goto(`${base}/test/launch.html`)
    await launch.waitForFunction(() => document.querySelector('#loaded')?.textContent === 'true')
    const button = launch.locator('#play-stop')
    await button.click()
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Stop')
    await button.click()
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Play')
    assert.equal((await launch.evaluate(() => window.launchFixture.stats())).stops, 1)
    await launch.evaluate(() => window.launchFixture.fail('Launch cancelled.'))
    assert.equal(await launch.locator('#error').textContent(), '')
    await launch.close()
  })
  await check('a late start after cancellation is stopped without reviving the Stop button', async () => {
    const launch = await browser.newPage()
    await launch.goto(`${base}/test/launch.html`)
    await launch.waitForFunction(() => document.querySelector('#loaded')?.textContent === 'true')
    await launch.locator('#play-stop').click()
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Stop')
    await launch.evaluate(() => window.launchFixture.stopError('It is not running.'))
    await launch.locator('#play-stop').click()
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Play')
    await launch.evaluate(() => {
      window.launchFixture.stopError(null)
      window.launchFixture.live(true)
      window.launchFixture.event(true)
      window.launchFixture.acknowledge()
    })
    await launch.waitForFunction(() => window.launchFixture.stats().stops >= 2)
    assert.equal(await launch.locator('#play-stop').textContent(), 'Play')
    assert.equal(await launch.locator('#error').textContent(), '')
    await launch.close()
  })
  await check('Play can retry after cancellation while the old launch finishes', async () => {
    const launch = await browser.newPage()
    await launch.goto(`${base}/test/launch.html`)
    await launch.waitForFunction(() => document.querySelector('#loaded')?.textContent === 'true')
    await launch.locator('#play-stop').click()
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Stop')
    await launch.locator('#play-stop').click()
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Play')
    await launch.locator('#play-stop').click()
    await launch.waitForFunction(() => window.launchFixture.stats().launches === 2)
    await launch.evaluate(() => window.launchFixture.fail('Launch cancelled.'))
    await launch.evaluate(() => new Promise(requestAnimationFrame))
    assert.equal(await launch.locator('#play-stop').textContent(), 'Stop')
    assert.equal(await launch.locator('#error').textContent(), '')
    await launch.evaluate(() => { window.launchFixture.live(true); window.launchFixture.event(true); window.launchFixture.acknowledge() })
    await launch.locator('#play-stop').click()
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Play')
    await launch.close()
  })
  await check('missed start and exit events reconcile with native state; stale reads cannot revive Stop', async () => {
    const launch = await browser.newPage()
    await launch.goto(`${base}/test/launch.html`)
    await launch.waitForFunction(() => document.querySelector('#loaded')?.textContent === 'true')
    await launch.locator('#play-stop').click()
    await launch.evaluate(() => { window.launchFixture.live(true); window.launchFixture.acknowledge() })
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Stop')
    await launch.evaluate(() => window.launchFixture.live(false))
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Play')
    await launch.evaluate(() => { window.launchFixture.live(true); window.launchFixture.event(true); window.launchFixture.deferRunning() })
    await launch.locator('#reload').click()
    await launch.evaluate(() => { window.launchFixture.live(false); window.launchFixture.event(false, true) })
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Play')
    await launch.evaluate(() => window.launchFixture.staleRunning())
    await launch.evaluate(() => new Promise(requestAnimationFrame))
    assert.equal(await launch.locator('#play-stop').textContent(), 'Play')
    assert.equal(await launch.locator('#error').textContent(), '')
    await launch.close()
  })
  await check('while a game runs, the poll asks which games run, not for the whole library', async () => {
    const launch = await browser.newPage()
    await launch.goto(`${base}/test/launch.html`)
    await launch.waitForFunction(() => document.querySelector('#loaded')?.textContent === 'true')
    const lists = () => launch.evaluate(() => window.launchFixture.stats().lists)
    await launch.locator('#play-stop').click()
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Stop')
    let before = await lists()
    // Only starting, not running in Rust yet: the poll keeps it.
    await launch.waitForTimeout(3_000)
    assert.equal(await launch.locator('#play-stop').textContent(), 'Stop')
    assert.equal(await lists(), before)
    await launch.evaluate(() => { window.launchFixture.live(true); window.launchFixture.acknowledge() })
    await launch.waitForFunction(n => window.launchFixture.stats().lists > n, before)
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Stop')
    before = await lists()
    await launch.waitForTimeout(3_000)
    assert.equal(await launch.locator('#play-stop').textContent(), 'Stop')
    assert.equal(await lists(), before)
    // A lost exit event: back to Play, and the list once for the new playtime.
    await launch.evaluate(() => window.launchFixture.live(false))
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Play')
    assert.equal(await lists(), before + 1)
    await launch.close()
  })
  await check('a game started from elsewhere reads the list once, for its new lastPlayed', async () => {
    const launch = await browser.newPage()
    await launch.goto(`${base}/test/launch.html`)
    await launch.waitForFunction(() => document.querySelector('#loaded')?.textContent === 'true')
    const lists = () => launch.evaluate(() => window.launchFixture.stats().lists)
    const before = await lists()
    // Started from the phone: only its event reaches this window.
    await launch.evaluate(() => { window.launchFixture.live(true); window.launchFixture.event(true) })
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Stop')
    await launch.waitForFunction(() => document.querySelector('#last')?.textContent === '1700000000')
    assert.equal(await lists(), before + 1)
    await launch.waitForTimeout(3_000)
    assert.equal(await lists(), before + 1)
    await launch.close()
  })
  await check('a missed start event is caught by the poll, and the list read once for lastPlayed', async () => {
    const launch = await browser.newPage()
    await launch.goto(`${base}/test/launch.html`)
    await launch.waitForFunction(() => document.querySelector('#loaded')?.textContent === 'true')
    const lists = () => launch.evaluate(() => window.launchFixture.stats().lists)
    const before = await lists()
    await launch.locator('#play-stop').click()
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Stop')
    // Running in Rust, but neither its event nor the launch's acknowledgement came.
    await launch.evaluate(() => window.launchFixture.live(true))
    await launch.waitForFunction(() => document.querySelector('#last')?.textContent === '1700000000')
    assert.equal(await lists(), before + 1)
    await launch.waitForTimeout(3_000)
    assert.equal(await launch.locator('#play-stop').textContent(), 'Stop')
    assert.equal(await lists(), before + 1)
    await launch.close()
  })
  await check('failed launch and failed termination are visible and retryable', async () => {
    const launch = await browser.newPage()
    await launch.goto(`${base}/test/launch.html`)
    await launch.waitForFunction(() => document.querySelector('#loaded')?.textContent === 'true')
    await launch.locator('#play-stop').click()
    await launch.evaluate(() => window.launchFixture.fail('Could not start the executable.'))
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Play')
    assert.equal(await launch.locator('#error').textContent(), 'Could not start the executable.')
    await launch.locator('#play-stop').click()
    await launch.evaluate(() => { window.launchFixture.live(true); window.launchFixture.event(true); window.launchFixture.acknowledge() })
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Stop')
    await launch.evaluate(() => window.launchFixture.stopError('Access denied.'))
    await launch.locator('#play-stop').click()
    await launch.waitForFunction(() => document.querySelector('#error')?.textContent === 'Access denied.')
    assert.equal(await launch.locator('#play-stop').textContent(), 'Stop')
    await launch.evaluate(() => window.launchFixture.stopError(null))
    await launch.locator('#play-stop').click()
    await launch.waitForFunction(() => document.querySelector('#play-stop')?.textContent === 'Play')
    await launch.close()
  })
  await check('desktop hover menus survive live download updates and retain selection', async () => {
    const nav = await browser.newPage()
    nav.on('pageerror', e => failures.push(e.message))
    await nav.goto(`${base}/test/nav.html`)
    const library = nav.getByRole('button', { name: 'library', exact: true })
    await library.hover()
    const item = nav.getByRole('menuitem', { name: 'Downloads', exact: true })
    await item.waitFor(); await item.hover(); await item.focus()
    for (let i = 1; i <= 30; i++) {
      await nav.evaluate(() => window.navFixture.tick())
      await nav.waitForFunction(n => document.querySelector('#progress')?.textContent === String(n), i)
      assert.equal(await library.getAttribute('aria-expanded'), 'true')
      assert.equal(await nav.evaluate(() => document.activeElement?.textContent), 'Downloads')
      await nav.waitForTimeout(50)
    }
    assert.equal(await nav.evaluate(() => window.navFixture.calls.filter(c => c.startsWith('menu_')).length), 0)
    await nav.keyboard.press('Enter')
    assert.equal(await nav.evaluate(() => window.navFixture.selected()), 30)
    await library.focus(); await nav.keyboard.press('ArrowDown')
    await item.waitFor(); await nav.keyboard.press('Escape')
    assert.equal(await nav.getByRole('menu').count(), 0)
    assert.equal(await nav.evaluate(() => document.activeElement?.textContent), 'library')
    await library.hover(); await item.waitFor(); await nav.locator('#progress').hover()
    await nav.waitForFunction(() => !document.querySelector('[role="menu"]'))
    await nav.getByRole('button', { name: 'store', exact: true }).click()
    await library.hover()
    await nav.waitForFunction(() => window.navFixture.calls.includes('menu_open'))
    assert.equal(await nav.getByRole('menu').count(), 0)
    await nav.locator('#local').click()
    await library.hover(); await item.waitFor()
    assert(await nav.evaluate(() => window.navFixture.calls.includes('menu_close')))
    await nav.close()
  })
  await check('the sign-in box shows its page again when sent back to a sign-in page', async () => {
    const signin = await browser.newPage({ viewport: { width: 1280, height: 1000 } })
    signin.on('pageerror', e => failures.push(e.message))
    await signin.goto(`${base}/test/signin.html`)
    await signin.waitForFunction(() => window.signinFixture.shown() === true)
    // Signed in: the page moves on, and the box says so over a hidden page.
    await signin.evaluate(() => window.signinFixture.at('https://kryo.to/'))
    await signin.waitForFunction(() => window.signinFixture.shown() === false)
    // The site sends it back to have the email checked: shown once it has loaded there.
    await signin.evaluate(() => window.signinFixture.at('https://kryo.to/verify-email', true))
    await signin.waitForTimeout(300)
    assert.equal(await signin.evaluate(() => window.signinFixture.shown()), false)
    await signin.evaluate(() => window.signinFixture.at('https://kryo.to/verify-email'))
    await signin.waitForFunction(() => window.signinFixture.shown() === true, null, { timeout: 5_000 })
    await signin.close()
  })
  await page.goto(`${base}/test/ui.html`)
  await check('loading is distinct from an empty download list', async () => {
    await expectText('#downloads', 'Loading downloads')
    assert(!await page.locator('#downloads').innerText().then(t => t.includes('No downloads')))
    await page.evaluate(() => window.fixture.reject('downloads_list'))
    await expectText('#downloads', 'Downloads unavailable')
    await page.locator('#downloads').getByRole('button', { name: 'Retry', exact: true }).click()
    await expectText('#downloads', 'Loading downloads')
    await page.evaluate(() => window.fixture.resolve('downloads_list', []))
    await expectText('#downloads', 'No downloads')
  })
  await check('quick-exit explanation survives a successful refresh', async () => {
    await page.evaluate(() => window.fixture.resolve('library_list', []))
    await page.evaluate(() => window.fixture.emit('game-state', { id: 'fixture', running: false, seconds: 1, code: 0 }))
    await expectText('#library-error', 'closed again')
    await page.evaluate(() => window.fixture.resolve('library_list', []))
    await expectText('#library-error', 'closed again')
  })
  await check('a stale library response cannot replace the latest one', async () => {
    await page.locator('#reload-library').click(); await page.locator('#reload-library').click()
    await page.evaluate(() => window.fixture.resolve('library_list', [{ ...window.fixture.game, title: 'Newest' }], true))
    await expectText('#library-games', 'Newest')
    await page.evaluate(() => window.fixture.resolve('library_list', [{ ...window.fixture.game, title: 'Stale' }]))
    assert.equal(await page.locator('#library-games').textContent(), 'Newest')
  })
  await check('dialogs isolate the background, wrap focus, and restore the trigger', async () => {
    await page.locator('#open').click()
    assert(await page.locator('#root').evaluate(el => el.inert))
    await page.locator('#last').focus(); await page.keyboard.press('Tab')
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute('aria-label')), 'Close')
    await page.keyboard.press('Shift+Tab')
    assert.equal(await page.evaluate(() => document.activeElement?.id), 'last')
    await page.locator('#nested').click()
    await page.locator('#nested-last').focus(); await page.keyboard.press('Tab')
    assert(await page.getByRole('dialog', { name: 'Nested fixture', exact: true }).evaluate(el => el.contains(document.activeElement)))
    await page.keyboard.press('Escape')
    assert.equal(await page.evaluate(() => document.activeElement?.id), 'nested')
    await page.keyboard.press('Escape')
    assert.equal(await page.evaluate(() => document.activeElement?.id), 'open')
    assert(!await page.locator('#root').evaluate(el => el.inert))
  })
  await check('choice and menu keyboard behavior', async () => {
    await page.locator('#open').click()
    await page.getByRole('radio', { name: 'Alpha', exact: true }).focus()
    await page.keyboard.press('ArrowRight')
    assert.equal(await page.getByRole('radio', { name: 'Beta', exact: true }).getAttribute('aria-checked'), 'true')
    assert.equal(await page.getByRole('radio', { name: 'Alpha', exact: true }).getAttribute('tabindex'), '-1')
    const dropdown = page.getByRole('button', { name: 'Fixture dropdown', exact: true })
    await dropdown.focus(); await page.keyboard.press('ArrowDown')
    assert.equal(await page.evaluate(() => document.activeElement?.textContent?.trim()), 'Beta')
    await page.keyboard.press('Home'); await page.keyboard.press('Enter')
    assert.equal(await page.getByRole('radio', { name: 'Alpha', exact: true }).getAttribute('aria-checked'), 'true')
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute('aria-label')), 'Fixture dropdown')
    await dropdown.click(); await page.keyboard.press('Escape')
    assert.equal(await page.getByRole('dialog').count(), 1)
    const menu = page.getByRole('button', { name: 'Fixture menu', exact: true })
    await menu.focus(); await page.keyboard.press('ArrowDown'); await page.keyboard.press('End')
    assert.equal(await page.evaluate(() => document.activeElement?.textContent?.trim()), 'Last action')
    await page.keyboard.press('Escape')
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute('aria-label')), 'Fixture menu')
    await page.keyboard.press('Escape')
  })
  await check('Play is visible on keyboard focus and touch', async () => {
    const play = page.locator('#library').getByRole('button', { name: 'Play Fixture game', exact: true })
    await play.focus()
    await page.waitForFunction(() => {
      const el = [...document.querySelectorAll('#library button')].find(b => b.getAttribute('aria-label') === 'Play Fixture game')
      return el && getComputedStyle(el).opacity === '1'
    })
    const context = await browser.newContext({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 844 } })
    const touch = await context.newPage(); await touch.goto(`${base}/test/ui.html`)
    const target = touch.locator('#library').getByRole('button', { name: 'Play Fixture game', exact: true })
    await target.waitFor()
    assert.equal(await target.evaluate(el => getComputedStyle(el).opacity), '1')
    await touch.waitForFunction(() => {
      const el = document.querySelector('[aria-label="Play Fixture game"]')
      const box = el?.getBoundingClientRect()
      return box && box.width >= 44 && box.height >= 44
    })
    const box = await target.boundingBox(); assert(box && box.width >= 44 && box.height >= 44)
    await context.close()
  })
  await check('failed search offers Retry instead of saying no games matched', async () => {
    let failed = true
    await page.route('**/api/games/search?**', route => route.fulfill({ status: failed ? 503 : 200, contentType: 'application/json', body: JSON.stringify(failed ? { error: 'Search unavailable' } : { results: [] }) }))
    await page.locator('#search').click()
    await page.getByPlaceholder(/Search|search/).fill('Fixture title')
    await expectText('[role="dialog"]', 'Try again')
    failed = false
    await page.getByRole('dialog').getByRole('button', { name: 'Retry', exact: true }).click()
    await expectText('[role="dialog"]', 'Nothing on kryo.to by that name')
    await page.keyboard.press('Escape')
  })
  await check("What's new counts as seen when it opens, and stays until it is closed (KB-0259)", async () => {
    // A window that went down while it was up (the Linux video crash) opened it again, in a loop.
    const news = await browser.newPage({ viewport: { width: 1280, height: 900 } })
    news.on('pageerror', e => failures.push(e.message))
    // The boot's game-runtimes question comes first otherwise, as in the checks below.
    await news.addInitScript(() => { performance.setResourceTimingBufferSize(4000); localStorage.setItem('kryo:runtimes-ask', 'never') })
    try {
      await news.goto(`${base}/?whatsnew`)
      const dialog = news.getByRole('dialog', { name: /^What's new in Kryoto Desktop/ })
      await dialog.waitFor()
      const seen = () => news.evaluate(async () => (await (await import('/src/lib/bridge.ts')).call('settings_get')).seenNotices)
      let saved = []
      for (let i = 0; i < 20 && !saved.includes('setup-0-3-0'); i++) {
        await news.waitForTimeout(100)
        saved = await seen()
      }
      assert.ok(saved.includes('setup-0-3-0'), `saved as seen while open: ${JSON.stringify(saved)}`)
      assert.equal(await dialog.count(), 1)
    } finally {
      await news.close()
    }
  })
  await check('prompts that open by themselves take turns and never land on a dialog', async () => {
    const want = id => page.evaluate(i => document.getElementById(`want-${i}`).click(), id)
    const only = async title => {
      await page.getByRole('dialog', { name: title }).waitFor()
      assert.equal(await page.getByRole('dialog').count(), 1)
    }
    const notice = page.locator('#notice-shown')
    await want('notice')
    await notice.waitFor()
    await want('found')
    await only('Prompt found')
    assert.equal(await notice.count(), 0)
    await want('intro')
    await want('update')
    await only('Prompt found')
    await page.keyboard.press('Escape')
    await only('Prompt update')
    await page.keyboard.press('Escape')
    await only('Prompt intro')
    await page.keyboard.press('Escape')
    await notice.waitFor()
    await page.locator('#open').click()
    await only('Fixture controls')
    assert.equal(await notice.count(), 0)
    await want('found')
    await page.waitForTimeout(100)
    await only('Fixture controls')
    await page.keyboard.press('Escape')
    await only('Prompt found')
    await page.evaluate(() => window.holdPrompts(true))
    await page.getByRole('dialog').waitFor({ state: 'detached' })
    assert.equal(await notice.count(), 0)
    await page.evaluate(() => window.holdPrompts(false))
    await only('Prompt found')
    await page.keyboard.press('Escape')
    await notice.waitFor()
    await want('notice')
  })
  await check('native and web bridges never load preview fixtures or start simulation timers', async () => {
    for (const mode of ['native', 'web', 'preview']) {
      const probe = await browser.newPage()
      const requests = []
      probe.on('request', r => { if (r.url().includes('/src/lib/preview.ts')) requests.push(r.url()) })
      await probe.goto(`${base}/test/bridge.html?mode=${mode}`)
      await probe.waitForFunction(() => document.querySelector('#result')?.textContent === 'Ready')
      const state = await probe.evaluate(() => window.bridgeResult)
      assert.equal(state.intervals, mode === 'preview' ? 1 : 0)
      assert.equal(state.cleared, mode === 'preview' ? 1 : 0)
      assert.equal(requests.length, mode === 'preview' ? 1 : 0)
      if (mode !== 'preview') assert.equal(state.result, mode)
      await probe.close()
    }
  })
  await check('settings recovery precedes normal controls and retains privacy choices on failure', async () => {
    const recovery = await browser.newPage({ viewport: { width: 1100, height: 800 } })
    recovery.on('pageerror', e => failures.push(e.message))
    await recovery.goto(`${base}/test/recovery.html`)
    const button = recovery.getByRole('button', { name: 'Use defaults', exact: true })
    await button.waitFor()
    assert.equal(await recovery.getByRole('button', { name: 'Quit Kryoto', exact: true }).count(), 1)
    await button.click()
    await recovery.getByText('Synthetic permission refusal. Original kept.', { exact: true }).waitFor()
    assert.equal(await button.count(), 1)
    await button.click()
    await recovery.waitForFunction(() => window.recoveryFixture.saved?.recoveryError === null)
    const saved = await recovery.evaluate(() => window.recoveryFixture.saved)
    assert.equal(saved.sendReports, false); assert.equal(saved.sharePlaytime, false)
    await recovery.waitForFunction(() => ![...document.querySelectorAll('button')].some(b => b.textContent === 'Use defaults'))
    await recovery.close()
  })
  await check('Play starts without a launch-choice dialog and alternate modes remain available', async () => {
    const game = await browser.newPage({ viewport: { width: 1280, height: 1000 } })
    game.on('pageerror', e => failures.push(e.message))
    await game.addInitScript(() => { performance.setResourceTimingBufferSize(4000); localStorage.setItem('kryo:runtimes-ask', 'never') })
    try {
      await game.goto(base)
      await game.getByRole('complementary', { name: 'Your games' }).getByRole('button', { name: 'Captain Hardcore', exact: true }).click()
      const region = game.getByRole('region', { name: 'Captain Hardcore', exact: true })
      await region.getByRole('button', { name: 'Ways to play', exact: true }).click()
      await game.getByText('Captain Hardcore Desktop Mode', { exact: true }).waitFor()
      await game.keyboard.press('Escape')
      await region.getByRole('button', { name: 'Play', exact: true }).click()
      // The sample game has hours its folder remembers: Play asks about those first.
      await game.getByRole('dialog', { name: 'Welcome back', exact: true }).getByRole('button', { name: 'Later', exact: true }).click()
      await region.getByRole('button', { name: 'Stop', exact: true }).waitFor()
      assert.equal(await game.getByRole('dialog', { name: 'Play Captain Hardcore', exact: true }).count(), 0)
      await game.getByRole('complementary', { name: 'Your games' }).getByRole('button', { name: 'Stop Captain Hardcore', exact: true }).click()
      await region.getByRole('button', { name: 'Play', exact: true }).waitFor()
    } finally { await game.close() }
  })
  await check('a failed Play is told where it was pressed, and only its own game page keeps it', async () => {
    const game = await browser.newPage({ viewport: { width: 1280, height: 800 } })
    game.on('pageerror', e => failures.push(e.message))
    await game.addInitScript(() => { performance.setResourceTimingBufferSize(4000); localStorage.setItem('kryo:runtimes-ask', 'never') })
    try {
      await game.goto(base)
      const rail = game.getByRole('complementary', { name: 'Your games' })
      await rail.waitFor()
      await game.evaluate(async () => {
        // The addresses the app loaded: an edit gives them a ?t= and a plain import a second copy.
        const loaded = name => performance.getEntriesByType('resource').map(r => r.name).find(url => url.includes(`/src/lib/${name}.ts`))
        const bridge = await import(loaded('bridge'))
        const preview = await import(loaded('preview'))
        bridge.useWebBackend({
          call: (c, a) => (c === 'game_launch' ? Promise.reject(`Synthetic ${c} failure`) : preview.previewCall(c, a)),
          on: (e, h) => preview.previewBus.on(e, h),
        })
      })
      await game.getByRole('button', { name: 'Play Hades', exact: true }).click()
      const notice = game.locator('.kryo-toast').filter({ hasText: 'Synthetic game_launch failure' })
      await notice.getByText('Hades', { exact: true }).waitFor()
      await rail.getByRole('button', { name: 'Celeste', exact: true }).click()
      const other = game.getByRole('region', { name: 'Celeste', exact: true })
      await other.waitFor()
      assert.equal(await other.getByText('Synthetic game_launch failure').count(), 0)
      await notice.getByRole('button').first().click()
      await game.getByRole('region', { name: 'Hades', exact: true }).getByText('Synthetic game_launch failure').waitFor()
    } finally { await game.close() }
  })
  await check('a question Play asks in Big Picture opens over it and B answers the question', async () => {
    const game = await browser.newPage({ viewport: { width: 1280, height: 800 } })
    game.on('pageerror', e => failures.push(e.message))
    await game.addInitScript(() => { performance.setResourceTimingBufferSize(4000); localStorage.setItem('kryo:runtimes-ask', 'never') })
    try {
      await game.goto(base)
      await game.getByRole('complementary', { name: 'Your games' }).waitFor()
      await game.getByRole('button', { name: 'View', exact: true }).click()
      await game.getByRole('menuitem', { name: 'Big Picture' }).click()
      const couch = game.getByRole('dialog', { name: 'Big Picture', exact: true })
      await couch.waitFor()
      await game.keyboard.press('Shift')
      await couch.getByRole('region', { name: 'All games' }).getByRole('button', { name: /Captain Hardcore/ }).click()
      await couch.getByRole('button', { name: /Play/ }).click()
      const question = game.getByRole('dialog', { name: 'Welcome back', exact: true })
      await question.waitFor()
      const open = name => game.evaluate(n => window.dispatchEvent(new CustomEvent(n, { cancelable: true })), name)
      assert.equal(await open('kryo-pad-back'), true)
      assert.equal(await open('kryo-pad-guide'), false)
      assert.equal(await game.locator('[aria-label="Big Picture"]').count(), 1)
      // Inert layers do not take hits, so what is on top only shows with the app's lifted.
      assert(await question.evaluate(dialog => {
        const root = document.getElementById('root')
        const box = dialog.getBoundingClientRect()
        root.inert = false
        const top = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2)
        root.inert = true
        return dialog.contains(top)
      }))
      await question.getByRole('button', { name: 'Later', exact: true }).click()
      await couch.getByRole('button', { name: /Stop/ }).click()
      await couch.getByRole('button', { name: /Play/ }).waitFor()
    } finally { await game.close() }
  })
  await check('Controller settings: a failed save says so, and a pad asked for opens its setup', async () => {
    const pads = await browser.newPage({ viewport: { width: 1280, height: 800 } })
    pads.on('pageerror', e => failures.push(e.message))
    await pads.addInitScript(() => { performance.setResourceTimingBufferSize(4000); localStorage.setItem('kryo:runtimes-ask', 'never') })
    try {
      await pads.goto(base)
      await pads.getByRole('complementary', { name: 'Your games' }).waitFor()
      await pads.evaluate(async () => {
        const loaded = name => performance.getEntriesByType('resource').map(r => r.name).find(url => url.includes(name))
        const bridge = await import(loaded('/src/lib/bridge.ts'))
        const preview = await import(loaded('/src/lib/preview.ts'))
        window.padFails = new Set()
        window.padStore = await import(loaded('/src/hooks/usePads.ts'))
        bridge.useWebBackend({
          call: (c, a) => (window.padFails.has(c) ? Promise.reject(`Synthetic ${c} failure`) : preview.previewCall(c, a)),
          on: (e, h) => preview.previewBus.on(e, h),
        })
      })
      const open = async () => {
        await pads.getByRole('button', { name: 'Account menu' }).click()
        await pads.getByRole('menuitem', { name: /Settings/ }).click()
        await pads.getByRole('button', { name: 'Devices', exact: true }).click()
      }
      await pads.evaluate(() => window.padStore.requestPad('03008231110100003414000000000000'))
      await open()
      const setup = pads.getByRole('dialog', { name: 'Set up SteelSeries controller' })
      await setup.waitFor()
      await setup.getByRole('button', { name: 'Close' }).click()
      assert.equal(await pads.getByRole('tab', { name: /SteelSeries/ }).getAttribute('aria-selected'), 'true')
      await pads.evaluate(() => window.padFails.add('pad_config_set'))
      const notify = pads.getByRole('checkbox', { name: 'Show a notification when a controller connects' })
      await notify.click()
      await pads.getByRole('alert').getByText('Synthetic pad_config_set failure').waitFor()
      assert.equal(await notify.isChecked(), false)
    } finally { await pads.close() }
  })
  await check('a Settings switch does not undo where saves are kept, chosen on the same page', async () => {
    const cloud = await browser.newPage({ viewport: { width: 1280, height: 900 } })
    cloud.on('pageerror', e => failures.push(e.message))
    await cloud.addInitScript(() => { performance.setResourceTimingBufferSize(4000); localStorage.setItem('kryo:runtimes-ask', 'never') })
    try {
      await cloud.goto(base)
      await cloud.getByRole('complementary', { name: 'Your games' }).waitFor()
      const saveOver = (fields, read) => cloud.evaluate(async ([fields, read]) => {
        const loaded = n => performance.getEntriesByType('resource').map(r => r.name).find(url => url.includes(n))
        const { settingsApi } = await import(loaded('/src/lib/settings.ts'))
        const latest = await settingsApi.get()
        await settingsApi.save({ ...latest, ...fields })
        return latest[read]
      }, [fields, read])
      // Saves kept with the account, as the app's own settings always say.
      await saveOver({ savesMode: '', savesLocalDir: '' }, 'savesMode')
      await cloud.getByRole('button', { name: 'Account menu' }).click()
      await cloud.getByRole('menuitem', { name: /Settings/ }).click()
      await cloud.getByRole('button', { name: 'Cloud', exact: true }).click()
      const onExit = cloud.getByRole('checkbox', { name: "Back up a game's saves when it closes" })
      await onExit.waitFor()
      // As "Where your saves are kept" and a notice marked seen save: on top of what is saved, past the page.
      const was = await saveOver({ savesMode: 'local', savesLocalDir: 'D:/Saves', seenNotices: ['check-seen'] }, 'cloudSavesOnExit')
      await onExit.click()
      const saved = () => cloud.evaluate(async () => (await (await import('/src/lib/bridge.ts')).call('settings_get')))
      let now = await saved()
      for (let i = 0; i < 20 && now.cloudSavesOnExit === was; i++) {
        await cloud.waitForTimeout(100)
        now = await saved()
      }
      assert.equal(now.cloudSavesOnExit, !was)
      assert.deepEqual([now.savesMode, now.savesLocalDir], ['local', 'D:/Saves'])
      assert.ok(now.seenNotices.includes('check-seen'), `seen: ${JSON.stringify(now.seenNotices)}`)
    } finally { await cloud.close() }
  })
  await check('the Troubleshooting log stays where it was scrolled, and Refresh goes to the newest lines', async () => {
    const logs = await browser.newPage({ viewport: { width: 1280, height: 900 } })
    logs.on('pageerror', e => failures.push(e.message))
    await logs.addInitScript(() => { performance.setResourceTimingBufferSize(4000); localStorage.setItem('kryo:runtimes-ask', 'never') })
    try {
      await logs.goto(base)
      await logs.getByRole('complementary', { name: 'Your games' }).waitFor()
      // A log longer than the 400 lines the box shows, so each new line drops one off the top.
      await logs.evaluate(async () => {
        const loaded = name => performance.getEntriesByType('resource').map(r => r.name).find(url => url.includes(name))
        const bridge = await import(loaded('/src/lib/bridge.ts'))
        const preview = await import(loaded('/src/lib/preview.ts'))
        window.logLines = Array.from({ length: 500 }, (_, i) => `2026-09-25 14:03:07Z INFO  preview: line ${i}`)
        window.addLog = n => { for (let i = 0; i < n; i++) window.logLines.push(`2026-09-25 14:03:08Z INFO  preview: line ${window.logLines.length}`) }
        bridge.useWebBackend({
          call: (c, a) => (c === 'logs_tail' ? Promise.resolve(window.logLines.slice(-a.lines).join('\n')) : preview.previewCall(c, a)),
          on: (e, h) => preview.previewBus.on(e, h),
        })
      })
      await logs.getByRole('button', { name: 'Account menu' }).click()
      await logs.getByRole('menuitem', { name: /Settings/ }).click()
      await logs.getByRole('button', { name: 'Troubleshooting', exact: true }).click()
      const box = logs.locator('pre', { hasText: 'preview: line 499' })
      await box.waitFor()
      const pre = await box.elementHandle()
      const atBottom = () => logs.waitForFunction(el => el.scrollHeight - el.scrollTop - el.clientHeight < 2, pre)
      const shows = line => logs.waitForFunction(([el, line]) => el.textContent.includes(line), [pre, line])
      // The first line in view, read from the rows themselves.
      const top = () => box.evaluate(el => [...el.children].find(r => r.getBoundingClientRect().bottom > el.getBoundingClientRect().top + 1).textContent)
      await atBottom()
      // Read from the middle while new lines come in: the same line stays at the top.
      await box.evaluate(el => { el.style.height = '120px'; el.scrollTop = el.children[50].offsetTop - el.firstElementChild.offsetTop })
      const reading = await top()
      assert.match(reading, /line 1[45]\d$/)
      await logs.evaluate(() => window.addLog(5))
      await shows('line 504')
      await logs.waitForTimeout(100)
      assert.equal(await top(), reading)
      // A selection dragged into the box from outside it holds the box as it is.
      await box.evaluate(el => document.getSelection().setBaseAndExtent(el.parentElement, 0, el.children[60].firstChild, 5))
      await logs.evaluate(() => window.addLog(1))
      await logs.waitForTimeout(3500)
      assert.ok(!(await box.textContent()).includes('line 505'))
      await logs.evaluate(() => document.getSelection().removeAllRanges())
      await shows('line 505')
      // At the bottom, the box follows new lines.
      await box.evaluate(el => { el.scrollTop = el.scrollHeight })
      await logs.evaluate(() => window.addLog(3))
      await shows('line 508')
      await atBottom()
      // Refresh goes to the newest lines from anywhere.
      await box.evaluate(el => { el.scrollTop = 0 })
      await logs.getByRole('button', { name: 'Refresh', exact: true }).click()
      await atBottom()
    } finally { await logs.close() }
  })
  await check('achievement repair offers gbe_fork and every RUNE profile with Undo between attempts', async () => {
    const achievements = await browser.newPage({ viewport: { width: 1100, height: 860 } })
    // This journey starts after startup setup; Windows otherwise waits at the
    // preview's missing-runtime prompt before the library can open.
    await achievements.addInitScript(() => { performance.setResourceTimingBufferSize(4000); localStorage.setItem('kryo:runtimes-ask', 'never') })
    achievements.on('pageerror', e => failures.push(e.message))
    try {
      await achievements.goto(base)
      await achievements.getByRole('complementary', { name: 'Your games' }).getByRole('button', { name: 'Hades', exact: true }).click()
      const notice = achievements.getByText('Achievement tracking is unavailable', { exact: true })
      await notice.waitFor()
      await achievements.getByRole('button', { name: 'Try another emulator', exact: true }).click()
      const dialog = achievements.getByRole('dialog', { name: 'Switch emulator for achievements', exact: true })
      const dropdown = dialog.getByRole('button', { name: 'Achievement emulator', exact: true })
      const labels = ['gbe_fork', 'RUNE', 'RUNE (Steakclient)', 'RUNE (Steamclient)']
      await dropdown.getByText('gbe_fork', { exact: true }).waitFor()
      await dropdown.focus(); await achievements.keyboard.press('ArrowDown')
      await achievements.getByRole('listbox', { name: 'Achievement emulator', exact: true }).waitFor()
      await achievements.getByRole('option', { name: 'RUNE (Steamclient)', exact: true }).waitFor()
      assert.deepEqual(await achievements.getByRole('option').allTextContents(), labels)
      await achievements.keyboard.press('Home'); await achievements.keyboard.press('ArrowDown'); await achievements.keyboard.press('Enter')
      await dropdown.getByText('RUNE', { exact: true }).waitFor()
      assert.equal(await dropdown.innerText(), 'RUNE')
      for (const label of ['RUNE', 'gbe_fork', 'RUNE (Steakclient)', 'RUNE (Steamclient)']) {
        await dropdown.click()
        await achievements.getByRole('option', { name: label, exact: true }).click()
        const apply = dialog.getByRole('button', { name: `Switch to ${label}`, exact: true })
        await apply.click()
        const undo = dialog.getByRole('button', { name: 'Undo repair', exact: true })
        await undo.waitFor()
        await achievements.getByText('1 of 2', { exact: true }).waitFor()
        await achievements.getByRole('region', { name: 'Hades', exact: true }).getByText(`Steam + ${label}`, { exact: true }).waitFor()
        assert.equal(await notice.count(), 0)
        assert.equal(await dropdown.isDisabled(), true)
        assert.equal(await apply.isDisabled(), true)
        await undo.click()
        await notice.waitFor()
      }
      await achievements.keyboard.press('Escape')
      await dialog.waitFor({ state: 'hidden' })
      assert.equal(await dialog.count(), 0)
    } finally { await achievements.close() }
  })
  await check('a DRM-free game turns achievements on with one button, and one with no Steam API shows no card', async () => {
    const page = await browser.newPage({ viewport: { width: 1100, height: 860 } })
    await page.addInitScript(() => { performance.setResourceTimingBufferSize(4000); localStorage.setItem('kryo:runtimes-ask', 'never') })
    page.on('pageerror', e => failures.push(e.message))
    try {
      await page.goto(base)
      const sidebar = page.getByRole('complementary', { name: 'Your games' })
      await sidebar.getByRole('button', { name: 'Hollow Knight', exact: true }).click()
      await page.getByRole('region', { name: 'Hollow Knight', exact: true }).waitFor()
      await page.waitForTimeout(300)
      assert.equal(await page.getByText('This copy does not record achievements', { exact: true }).count(), 0)
      assert.equal(await page.getByText('Achievement tracking is unavailable', { exact: true }).count(), 0)
      await sidebar.getByRole('button', { name: 'Captain Hardcore', exact: true }).click()
      const region = page.getByRole('region', { name: 'Captain Hardcore', exact: true })
      await region.getByText('This copy does not record achievements', { exact: true }).waitFor()
      await region.getByText('2 on Steam', { exact: true }).waitFor()
      assert.equal(await region.getByRole('button', { name: 'Try another emulator', exact: true }).count(), 0)
      await region.getByRole('button', { name: 'Turn on achievements', exact: true }).click()
      await region.getByText('1 of 2', { exact: true }).waitFor()
      await region.getByText('Steam + gbe_fork', { exact: true }).waitFor()
      assert.equal(await region.getByText('This copy does not record achievements', { exact: true }).count(), 0)
    } finally { await page.close() }
  })
  await check('a game shows its Linux compatibility and setup through the shared renderer, and an untested game shows none', async () => {
    const context = await browser.newContext({ viewport: { width: 1280, height: 1000 }, userAgent: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36' })
    const linux = await context.newPage()
    linux.on('pageerror', e => failures.push(e.message))
    const stranger = []
    linux.on('request', r => { if (r.url().includes('evil.example')) stranger.push(r.url()) })
    const empty = { status: 'untested', reports: 0, older: 0, detail: null }
    const report = (id, extra) => ({ id, platform: 'linux', outcome: 'setup', version: 'b1', current: true, env: { layer: 'proton_ge', layerVersion: '9-20', osVersion: 'Arch Linux' }, instructions: null, tester: { name: 'Tess', username: 'tess_t' }, testedAt: '2026-10-10T00:00:00Z', ...extra })
    await linux.route('**/api/games/*/compatibility', route => {
      if (route.request().url().includes('/hades/')) return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ summary: { headline: 'Works on Linux.', platforms: { windows: empty, linux: { ...empty, status: 'works', reports: 1 }, steamdeck: empty, macos: empty }, unspecified: empty }, reports: [report('3', { outcome: 'works', instructions: 'Nothing to change.' })] }) })
      if (!route.request().url().includes('/hollow-knight/')) return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ summary: { headline: 'Nobody has tested it yet.', platforms: { windows: empty, linux: empty, steamdeck: empty, macos: empty }, unspecified: empty }, reports: [] }) })
      return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({
        summary: { headline: 'Works on Windows and Linux.', platforms: { windows: { ...empty, status: 'works', reports: 1 }, linux: { ...empty, status: 'setup', reports: 2 }, steamdeck: empty, macos: empty }, unspecified: empty },
        reports: [
          report('2', { instructions: 'Set **PROTON_USE_WINED3D=1**\n![shot](https://evil.example/x.png) <img src="https://evil.example/y.png">', tester: { name: 'Anonymous tester', username: null } }),
          report('1', { current: false, instructions: 'older steps' }),
        ],
      }) })
    })
    try {
      await linux.goto(base)
      const sidebar = linux.getByRole('complementary', { name: 'Your games' })
      await sidebar.getByRole('button', { name: 'Hollow Knight', exact: true }).click()
      const region = linux.getByRole('region', { name: 'Hollow Knight', exact: true })
      await region.getByText('On Linux', { exact: true }).waitFor()
      await region.getByText('Working, setup needed', { exact: true }).waitFor()
      await region.getByText('Works on Windows and Linux.', { exact: true }).waitFor()
      assert.equal(await region.getByRole('link', { name: '2 reports', exact: true }).getAttribute('href'), 'https://kryo.to/game/hollow-knight#compatibility-linux')
      await region.locator('strong', { hasText: 'PROTON_USE_WINED3D=1' }).waitFor()
      assert.equal(await region.getByText(/\*\*PROTON/).count(), 0)
      assert.equal(await region.locator('img[src*="evil.example"]').count(), 0)
      assert.equal(await region.getByRole('link', { name: 'Anonymous tester' }).count(), 0)
      await region.getByText(/GE-Proton 9-20, Arch Linux, build b1/).waitFor()
      assert.deepEqual(stranger, [])
      await sidebar.getByRole('button', { name: 'Hades', exact: true }).click()
      const hades = linux.getByRole('region', { name: 'Hades', exact: true })
      await hades.getByText('Setup that worked').click()
      assert.equal(await hades.getByRole('link', { name: 'Tess', exact: true }).getAttribute('href'), 'https://kryo.to/user/tess_t')
      await sidebar.getByRole('button', { name: 'Celeste', exact: true }).click()
      await linux.getByRole('region', { name: 'Celeste', exact: true }).getByText('Launch command').waitFor()
      await linux.waitForTimeout(300)
      assert.equal(await linux.getByText('On Linux', { exact: true }).count(), 0)
    } finally { await context.close() }
  })
  await check('Art and Contribute open from the Community menu and light its tab', async () => {
    const shell = await browser.newPage({ viewport: { width: 1280, height: 800 } })
    shell.on('pageerror', e => failures.push(e.message))
    await shell.addInitScript(() => localStorage.setItem('kryo:runtimes-ask', 'never'))
    try {
      await shell.goto(base)
      await shell.getByRole('complementary', { name: 'Your games' }).waitFor()
      const community = shell.getByRole('button', { name: 'Community', exact: true })
      assert.equal(await community.getAttribute('aria-current'), null)
      await community.hover()
      await shell.getByRole('menuitem', { name: 'Contribute', exact: true }).waitFor()
      await shell.getByRole('menuitem', { name: 'Art', exact: true }).click()
      // kryo.to/art is a Community page, not the Store's.
      await shell.waitForFunction(() => [...document.querySelectorAll('button[aria-current="page"]')].some(b => b.textContent === 'Community'))
    } finally { await shell.close() }
  })
  await check('a Preview report keeps its text when closed, for that account only, and starts empty once sent', async () => {
    const ui = await browser.newPage()
    ui.on('pageerror', e => failures.push(e.message))
    try {
      await ui.goto(`${base}/test/ui.html`)
      await ui.locator('#report').click()
      const dialog = ui.getByRole('dialog', { name: 'Report a bug', exact: true })
      await dialog.getByRole('textbox', { name: 'What happened', exact: true }).fill('The Store went white')
      await dialog.getByRole('textbox', { name: 'Steps to see it', exact: true }).fill('Open Downloads')
      // Closed at once, before the pause that saves it.
      await dialog.getByRole('button', { name: 'Close', exact: true }).click()
      await dialog.waitFor({ state: 'detached' })
      // Another account signed in on this PC starts empty: the report would go out under its name.
      await ui.locator('#report-other').click()
      assert.equal(await dialog.getByRole('textbox', { name: 'What happened', exact: true }).inputValue(), '')
      await dialog.getByRole('button', { name: 'Close', exact: true }).click()
      await dialog.waitFor({ state: 'detached' })
      await ui.locator('#report').click()
      assert.equal(await dialog.getByRole('textbox', { name: 'What happened', exact: true }).inputValue(), 'The Store went white')
      assert.equal(await dialog.getByRole('textbox', { name: 'Steps to see it', exact: true }).inputValue(), 'Open Downloads')
      await dialog.getByRole('button', { name: 'Send the report' }).click()
      await dialog.getByText('Sent. Thank you.').waitFor()
      await dialog.getByRole('button', { name: 'Close', exact: true }).click()
      await dialog.waitFor({ state: 'detached' })
      await ui.locator('#report').click()
      assert.equal(await dialog.getByRole('textbox', { name: 'What happened', exact: true }).inputValue(), '')
    } finally { await ui.close() }
  })
  await check("a game kryo.to will not put in a collection says why, in kryo.to's words", async () => {
    const shelf = await browser.newPage({ viewport: { width: 1280, height: 800 } })
    shelf.on('pageerror', e => failures.push(e.message))
    await shelf.addInitScript(() => localStorage.setItem('kryo:runtimes-ask', 'never'))
    try {
      await shelf.goto(base)
      const rail = shelf.getByRole('complementary', { name: 'Your games' })
      const collect = async (title, list) => {
        await rail.getByRole('button', { name: title, exact: true }).click({ button: 'right' })
        await shelf.getByRole('menuitem', { name: 'Add to collection' }).click()
        await shelf.getByRole('menuitem', { name: list }).click()
      }
      // The preview's kryo.to refuses Hollow Knight, as it once did a game (plan 01).
      await collect('Hollow Knight', 'Cozy nights')
      await shelf.locator('.kryo-toast').filter({ hasText: 'A game slug is required.' }).waitFor()
      // Another game goes in, and the menu shows it there.
      await collect('Hades', 'Finished in a weekend')
      await rail.getByRole('button', { name: 'Hades', exact: true }).click({ button: 'right' })
      await shelf.getByRole('menuitem', { name: /^Add to collection\s*1$/ }).waitFor()
    } finally { await shelf.close() }
  })
  await check('a blocked network shows how to change DNS on this PC, without kryo.to', async () => {
    // Windows 10 and 11 both say "Windows NT 10.0": only the platform version tells them apart.
    for (const [os, platformVersion] of [['Windows NT 10.0; Win64; x64', '15.0.0'], ['Windows NT 10.0; Win64; x64', '10.0.0'], ['X11; Linux x86_64', null]]) {
      const context = await browser.newContext({ userAgent: `Mozilla/5.0 (${os}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36` })
      if (platformVersion) {
        await context.addInitScript((v) => Object.defineProperty(navigator, 'userAgentData', { value: { getHighEntropyValues: async () => ({ platformVersion: v }) } }), platformVersion)
      }
      const ui = await context.newPage()
      ui.on('pageerror', e => failures.push(e.message))
      try {
        await ui.goto(`${base}/test/ui.html`)
        await ui.locator('#blocked').click()
        const dialog = ui.getByRole('dialog', { name: 'Your network is blocking kryo.to', exact: true })
        const how = dialog.getByText('Show me how', { exact: true })
        assert.equal(await dialog.getByText('SNI or IP block', { exact: false }).isVisible(), false)
        await how.click()
        await dialog.getByText('SNI or IP block', { exact: false }).waitFor()
        const eleven = platformVersion === '15.0.0'
        const ten = platformVersion === '10.0.0'
        if (eleven) await dialog.getByText('On Windows 11, encrypted DNS:', { exact: true }).waitFor()
        if (ten) await dialog.getByText('On Windows 10:', { exact: true }).waitFor()
        assert.equal(await dialog.getByText('Preferred DNS: 1.1.1.1', { exact: false }).isVisible(), eleven)
        assert.equal(await dialog.getByText('Turn on IPv6', { exact: false }).isVisible(), eleven)
        assert.equal(await dialog.getByText('netsh dns set encryption', { exact: false }).isVisible(), eleven)
        assert.equal(await dialog.getByText('Network and Sharing Center', { exact: false }).isVisible(), ten)
        assert.equal(await dialog.getByText('netsh interface ipv6 set dnsservers', { exact: false }).isVisible(), eleven || ten)
        assert.equal(await dialog.getByText('DNSOverTLS=yes', { exact: false }).isVisible(), !platformVersion)
      } finally { await context.close() }
    }
  })
  await check('a saved game whose adult lookup failed once is asked again, and its cover clears', async () => {
    const page = await browser.newPage({ viewport: { width: 1280, height: 1000 } })
    page.on('pageerror', e => failures.push(e.message))
    await page.addInitScript(() => { performance.setResourceTimingBufferSize(4000); localStorage.setItem('kryo:runtimes-ask', 'never') })
    let asked = 0
    await page.route('**/api/games/hades-ii', route => {
      asked += 1
      return asked === 1
        ? route.fulfill({ status: 503, body: 'busy' })
        : route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ game: { title: 'Hades II', nsfw: false } }) })
    })
    const pixel = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=', 'base64')
    await page.route('**/api/steam/image.jpg?**', route => route.fulfill({ status: 200, contentType: 'image/png', body: pixel }))
    try {
      await page.goto(base)
      await page.getByRole('checkbox', { name: /Show games not on this PC/ }).click()
      const cover = page.getByRole('button', { name: 'Open Hades II', exact: true }).locator('img')
      await cover.waitFor()
      // Unknown counts as adult; the second answer, not a restart, says otherwise.
      await page.waitForFunction(() => {
        const img = document.querySelector('button[aria-label="Open Hades II"] img')
        return !!img && !img.className.includes('blur-2xl')
      }, null, { timeout: 30_000 })
      assert.ok(asked >= 2)
    } finally { await page.close() }
  })
  await check('a saved game that failed once and then turns out gone is not asked about over and over', async () => {
    const page = await browser.newPage({ viewport: { width: 1280, height: 1000 } })
    page.on('pageerror', e => failures.push(e.message))
    await page.addInitScript(() => {
      localStorage.setItem('kryo:runtimes-ask', 'never')
      // A hook that keeps waking itself shows up as timers set back to back.
      const set = window.setTimeout
      window.timersSet = 0
      window.setTimeout = (...a) => { window.timersSet += 1; return set(...a) }
    })
    let asked = 0
    await page.route('**/api/games/hades-ii', route => {
      asked += 1
      return route.fulfill({ status: asked === 1 ? 503 : 404, body: '' })
    })
    try {
      await page.goto(base)
      await page.getByRole('checkbox', { name: /Show games not on this PC/ }).click()
      for (const end = Date.now() + 30_000; asked < 2 && Date.now() < end;) await page.waitForTimeout(250)
      assert.ok(asked >= 2)
      await page.waitForTimeout(500)
      const before = await page.evaluate(() => window.timersSet)
      await page.waitForTimeout(1_000)
      assert.ok(await page.evaluate(n => window.timersSet - n, before) < 50)
    } finally { await page.close() }
  })
  await check('a saved game lookup that fails while the list changes is still asked again', async () => {
    const page = await browser.newPage({ viewport: { width: 1280, height: 1000 } })
    page.on('pageerror', e => failures.push(e.message))
    await page.addInitScript(() => { performance.setResourceTimingBufferSize(4000); localStorage.setItem('kryo:runtimes-ask', 'never') })
    let asked = 0
    let answer = () => {}
    const held = new Promise(resolve => { answer = resolve })
    await page.route('**/api/games/hades-ii', async route => {
      asked += 1
      if (asked === 1) {
        await held
        return route.fulfill({ status: 503, body: 'busy' })
      }
      return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ game: { title: 'Hades II', nsfw: false } }) })
    })
    const pixel = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=', 'base64')
    await page.route('**/api/steam/image.jpg?**', route => route.fulfill({ status: 200, contentType: 'image/png', body: pixel }))
    try {
      await page.goto(base)
      const notHere = page.getByRole('checkbox', { name: /Show games not on this PC/ })
      await notHere.click()
      for (const end = Date.now() + 10_000; asked < 1 && Date.now() < end;) await page.waitForTimeout(100)
      // The list changes while the lookup is out, so the run that asked is gone when it fails.
      await notHere.click()
      await notHere.click()
      await page.getByRole('button', { name: 'Open Hades II', exact: true }).locator('img').waitFor()
      answer()
      await page.waitForFunction(() => {
        const img = document.querySelector('button[aria-label="Open Hades II"] img')
        return !!img && !img.className.includes('blur-2xl')
      }, null, { timeout: 30_000 })
      assert.ok(asked >= 2)
    } finally { await page.close() }
  })
  assert.deepEqual(failures, [])
  console.log('All Desktop UI regressions passed')
} finally { await browser.close() }
