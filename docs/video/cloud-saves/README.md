# Cloud saves walkthrough

Recorded with the existing launch-video skill at `kryo.to/.claude/skills/launch-video/SKILL.md` using headless Chrome.

The capture opens Desktop's normal `index.html`, which loads `src/main.tsx`, `App.tsx` and the real `Shell`. All menus, navigation, pages, cloud controls and dialogs are the app's own. No custom shell or replacement page is used, and no native installation is required.

`walkthrough.mjs` seeds the existing browser development backend through its library commands and account event, then drives the real controls. Game art, titles, descriptions and developers come from public Steam app metadata saved in `catalog.json` on 2026-10-20. The save history and account entitlement are local development data; this is a usage tutorial, not evidence of a live two-PC cloud transfer.

Narration uses ElevenLabs `eleven_v4`, Ian (`70HFmYdPmz1aKiKW6Zx3`), selected generation `MFDVf0prVjSanXpIa2tS`. `narration.json` contains the script and word timings. It contains no credentials or expiring media links.

## Reproduce

From `kryoto-desktop`, run `pnpm dev --host 127.0.0.1`. Run the following from the workspace root:

```sh
node kryo.to/.claude/skills/launch-video/scripts/record.mjs --walk kryoto-desktop/docs/video/cloud-saves/walkthrough.mjs --work kryoto-desktop/.tmp/cloud-saves-headless-final --dry
node kryo.to/.claude/skills/launch-video/scripts/record.mjs --walk kryoto-desktop/docs/video/cloud-saves/walkthrough.mjs --work kryoto-desktop/.tmp/cloud-saves-headless-final
node kryo.to/.claude/skills/launch-video/scripts/plan-voice.mjs --work kryoto-desktop/.tmp/cloud-saves-headless-final --short kryoto-desktop/docs/video/cloud-saves/short.json
```

Keep the selected audio at `.tmp/cloud-saves-video/voice.mp3`, relative to Desktop. The soundtrack can also be extracted from the checked-in MP4 for a later edit. Embed Teletext into the skill's stage with `embed-fonts.mjs`, sample the planned edit with `sample.mjs`, render disjoint frame ranges with `render.mjs` and encode with `encode.mjs --name cloud-saves-guide --small 0`. The skill documents these steps in full.

The 2560x1440, 60 fps MP4 and its transcript/captions live in `kryo.to/public/blog/cloud-saves-0-13-0/`. The blog remains a draft for later manual publication.
