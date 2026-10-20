import fs from 'node:fs'
const metadata = JSON.parse(fs.readFileSync(new URL('./catalog.json', import.meta.url), 'utf8'))

// The app's NORMAL index.html -> main.tsx -> App.tsx -> Shell entry point.
// Only its existing browser development backend is seeded; no replacement UI.
export const config = { base: 'http://127.0.0.1:1421', width: 1600, height: 900, dpr: 1.5, port: 9563 }

async function seed(c, conflict = false) {
  await c.evaluate(`(async () => {
    const p = await import('/src/lib/preview.ts');
    p.stopPreview();
    const original = await p.previewCall('library_list');
    const meta = ${JSON.stringify(metadata)};
    const order = ${JSON.stringify(conflict ? ['Celeste', 'Hollow Knight', 'Stardew Valley'] : ['Hollow Knight', 'Celeste', 'Stardew Valley'])};
    const selected = order.map(title => { const g=original.find(g=>g.title===title); const m=meta.find(m=>m.title===title); return {...g,...m,slug:null}; });
    for (const g of original) await p.previewCall('library_remove',{id:g.id});
    for (const g of selected) await p.previewCall('library_add',{game:g,exePath:g.installDir+'\\\\'+g.executable});
    const downloads=await p.previewCall('downloads_list');
    for (const d of downloads) await p.previewCall('download_cancel',{id:d.id});
    p.previewBus.emit('account-state',{username:'cloudguide',displayName:'Cloud Guide',avatarUrl:null,supporter:true,cloudSaves:true,controller:false,chat:false,friends:false,appearance:null});
    p.previewBus.emit('news-state',{version:'0.13.0',date:'2026-10-20'});
    p.previewBus.emit('inbox-state',{unreadCount:0,notifications:[]});
    p.previewBus.emit('library-changed',null);
    return true;
  })()`)
  await c.pause(800)
}

export async function prepare(c) {
  await c.goto(`${c.base}/`)
  await c.waitFor("document.body.innerText.includes('LATER') || document.body.innerText.includes('LIBRARY')",15000)
  if (await c.evaluate("document.body.innerText.includes('GAME RUNTIMES')")) await c.clickOn('exact','Later')
  await c.waitFor("document.body.innerText.includes('LIBRARY')",10000)
  await seed(c)
  await c.waitImages()
}

async function openGame(c) { await c.clickOn('exact','Library'); await c.clickOn('exact','Hollow Knight'); await c.waitImages(); await c.pause(700) }
async function saves(c) {
  await c.evaluate("[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Versions')?.closest('.grid')?.scrollIntoView({block:'center',behavior:'smooth'})")
  await c.pause(1200)
}

export default async function walk(c) {
  c.begin('library'); await c.pause(4400); c.cut()

  c.begin('settings')
  await c.clickOn('exact','Kryoto')
  await c.clickOn('text','Settings')
  await c.clickOn('exact','Cloud')
  await c.pause(4000); c.cut()

  await openGame(c)
  c.begin('backup')
  await c.clickOn('exact','Play')
  await c.pause(1000)
  await c.clickOn('exact','Stop')
  await saves(c)
  await c.pause(3200); c.cut()

  await c.clickOn('exact','Kryoto'); await c.clickOn('text','Settings'); await c.clickOn('exact','Cloud')
  c.begin('new-pc')
  await c.hoverOn('exact','Put them here',{stay:1800})
  await openGame(c)
  await c.clickOn('exact','Play')
  await c.pause(2500); c.cut()

  // Seed a second local development situation using the app's existing fixture
  // rule: the second library game has conflicting progress. Same actual pages.
  await seed(c,true)
  await openGame(c); await saves(c)
  c.begin('conflict')
  await c.pause(2600)
  await c.hoverOn('exact',"Use the account's",{stay:2200})
  c.cut()

  await seed(c,false); await openGame(c); await saves(c)
  c.begin('versions')
  await c.clickOn('exact','Versions')
  await c.pause(700)
  await c.clickOn('exact','Restore')
  await c.clickOn('exact','Press again')
  await c.pause(3000); c.cut()
}
