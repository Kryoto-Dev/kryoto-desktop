# Patchbay: your own servers

Kryoto Desktop lets you plug in your own services, in Settings > Cloud > Patchbay and, for debrid, Settings > Downloads > Debrid:

- **Cloud saves server.** Your game saves are backed up to your server instead of your kryo.to account.
- **Achievements server.** The achievements you unlock are kept on your server instead of your kryo.to account.
- **Real-Debrid or TorBox.** Downloads from hosts your debrid service supports go through your account there first. TorBox fetches a file it does not have yet; if that takes more than a minute, Kryoto downloads it the usual way.

While one of your own servers is set, Kryoto Desktop sends kryo.to nothing for that feature: none of your saves, achievements or tokens. Tokens are kept in your computer's secure store (Windows Credential Manager, or GNOME Keyring or KWallet on Linux). They never go in Kryoto's settings file. On a Linux system with neither, Kryoto says so when you try to keep a token. A server without a token still works, so keep yours on your own network.

When you switch back to kryo.to, everything is as you left it there. Each place keeps its own saves and its own history, and switching never copies anything between them.

Settings > Cloud picks the place for saves and for achievements separately: on this PC (a folder you choose, with the same versions and restore, nothing over the network), your own server, or the kryo.to account. The reference server keeps 10 versions per game; `--keep` changes that.

## Run the reference server

[`tools/patchbay-server.mjs`](../tools/patchbay-server.mjs) is a complete server in one file. It keeps everything in a folder and needs nothing but Node 20.

```
node tools/patchbay-server.mjs --dir ./patchbay-data --port 8787 --token "a long random secret"
```

Then in Settings > Cloud > Patchbay:

1. Enter the address `http://<that machine>:8787`. Plain HTTP works on your own PC and your local network. For anywhere else, put it behind HTTPS (a reverse proxy such as Caddy or nginx), and pass `--public-url https://your.domain`. Let the proxy take bodies as big as your biggest backup: nginx stops at 1 MB unless `client_max_body_size` says otherwise, and a proxy that turns the upload away is how a save over its limit fails with a 413 or 502.
2. Paste the same token and choose Keep.
3. Choose Test.

The same server can hold both your saves and your achievements.

## The protocol

A server speaks the same small API as kryo.to, under the address you give. The address may include a path, for example `https://example.com/kryoto`, in which case the API is at `https://example.com/kryoto/api/...`.

Every `/api` request carries `Authorization: Bearer <token>` when a token is set. Answer `401` to a wrong token. Requests and answers are JSON, and an error is `{ "error": "Something a person can read." }`. Kryoto Desktop never follows a redirect from your server, because a token must never travel to somewhere else.

### Cloud saves

Times are Unix seconds. A **version** looks like this:

```json
{ "id": "…", "size": 1234, "sha256": "…", "md5": "…", "fileCount": 3, "machine": "PC name",
  "createdAt": 1767000000, "parentId": "… or null", "manifest": { … as sent … } }
```

| Request | Answer |
|---|---|
| `GET /api/saves` | `{ perk: true, allowance: { totalBytes, versionsPerGame, maxBackupBytes }, usage: { bytes, games, versions }, games: [{ gameKey, aliases, slug, title, bytes, versions: [version, newest first] }] }` |
| `GET /api/saves?gameKey=K` | `{ perk: true, allowance, head: version or null }` |
| `POST /api/saves/prepare` with `{ gameKey, aliases, slug, title, size, md5, sha256, manifest, machine, parentVersion }` | `{ uploadId, url, headers }`. `url` is where Desktop PUTs the backup file with `headers`, **without** the token, so make it a signed or one-time link. Answer `409 { error, head }` when `parentVersion` is not the game's newest version (another PC backed up first). Answer `413` when it is too big. |
| `PUT <url>` | `2xx` once the bytes are stored. Check the size and sha256 against what `prepare` was told. |
| `POST /api/saves/claim` with `{ uploadId }` | `{ version }`: the upload is now the game's newest version. Answer `409 { error, head }` if another PC got there first, and `422` if the upload never arrived. |
| `POST /api/saves/link` with `{ versionId }` | `{ url, size, sha256 }`. Desktop downloads `url` **without** the token, so make it a signed or one-time link. |
| `GET /api/saves/rules?gameKeys=a,b` | `{ rules: {} }`, or known save locations per game in kryo.to's rule format. |
| `POST /api/saves/rules/report` | `{ accepted: 0 }`. Desktop reports save locations it found; you may ignore them. |

`perk` is kryo.to's supporter perk. Your own server answers `true`, or leaves it out.

### Achievements

| Request | Answer |
|---|---|
| `POST /api/achievements/cloud` with `{ games: [{ appid, slug, title, unlocks: [{ name, unlockedAt }] }] }` | `{ games: ["appid", …] }`: the games whose unlocks you kept. Keep every unlock you have not seen before; never remove one. |
| `GET /api/achievements/cloud?appids=a,b` | `{ games: { "a": [{ name, unlockedAt }], … } }`, with every unlock you hold for each game. With no `appids`, answer for all games. |

Desktop sends new unlocks and fetches a few games about every half minute. Any unlock this PC lacks is written back into the game's own save, so the game has it too.
