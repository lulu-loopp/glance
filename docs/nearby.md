# Nearby transfer (compatible with LocalSend)

Agreed with the owner on 2026-11-12, from the approved motion mockup
(https://claude.ai/artifact/MD5MXDL4z24DJxrFEMFTPP). A photo taken on a phone shows up on the PC by itself: in the
station and in recent files. Glance speaks LocalSend's open protocol, so any phone or computer that has LocalSend can
send to it and receive from it without Glance's own app. Glance's Android app adds one thing LocalSend cannot do:
it sends each new photo by itself.

The mockup's defaults were kept, because the owner did not choose otherwise:
- Photos from one device within a minute of each other join one item.
- The edge notice is on.
- Auto-send includes screenshots.

## What it does (v1)

1. **Receive.** LocalSend senders find this PC under its name.
   - A device not trusted yet is asked about: a notice at the screen's edge names the device, the files and their
     sizes, with Accept and Refuse and a box "以后直接接收这台设备的文件".
   - A trusted device's files are taken without asking.
   - What comes lands in the station's own folder, marked as from the Internet (`Zone.Identifier`, ZoneId=3). It
     shows at the head of the station with the device's mark, and at the head of recent files with the device as
     its source.
2. **Notice.** With the panel closed, a notice slides out at the panel's edge: a request, then progress, then the
   picture. It is pressed to open the panel, or it goes back after 4 s. It never shows over a game.
3. **In the panel.** The station makes room at its head, the new item shows a ring of progress, and then the picture.
4. **Send.** While an item of the station is dragged, the devices seen nearby rise at the station's foot. Dropped on
   one, the item is sent. The device shows "等待对方接受", then a ring of progress, then a check. The item stays on
   the station.
5. **Android auto-send.** Glance's Android app has a switch, off by default: "新照片自动发到这台电脑". With it on,
   each new photo (and screenshot) is sent to the paired PC within seconds, with no app open.
6. **Settings.** A page "附近传输":
   - receive on/off (off by default)
   - this PC's name
   - the trusted devices, each with a way to remove it
   - what to do with others: ask or refuse
   - the edge notice on/off
   - where files go (the station's folder)

7. **Words.** Text a device sends as a message (LocalSend's text) lands on the station as words: a web address alone
   as a link, other text as text, and text too long for the shelf as a saved text file that is still words. The
   notice shows the words themselves, two lines at most (a link without its scheme, the rest cut short with "…"),
   by a mark of text or of a link, "文字 · 来自 <device>" or "链接 · 来自 <device>" under them, and a button "复制" that
   copies all of them as the station does: it says "已复制" and the notice goes 1.2 s later, held while the pointer is
   on it (approved 2026-11-14, mockup Y2oGbrLvJWnZYVxTwiAhsP). Asking about a device not trusted shows the first
   words ("想发送一段文字", "想发送一个链接").
8. **Android, by hand.** The app sends chosen photos, videos and files, and what other apps share to it (words too),
   to a paired PC's station (approved 2026-11-13, mockup BBWmD378gVMdRXrA8hhj9H). See "Android, by hand" below.

Not yet: the reverse-download (browser) mode, a PIN of this PC's own, and the phone receiving from the PC (the
LocalSend app does that for now).

## The protocol (LocalSend v2, as the protocol document and the app's source say)

- **Discovery.** UDP multicast `224.0.0.167:53317`. An announcement is JSON:
  - `alias`, `version` "2.1", `deviceModel`, `deviceType`, `fingerprint`, `port`, `protocol` (`https`), `download`
    (false), `announce` (true)
  - A device that hears `announce: true` answers with `POST /api/localsend/v2/register` to the announcer, over the
    announcer's protocol and port, carrying its own info. Glance does the same both ways. It announces at start, on
    a network change, and when the send strip opens (a burst at 100, 500 and 2000 ms).
- **Endpoints served** (prefix `/api/localsend/v2`):
  - `POST register`: their info in, ours out.
  - `GET info`
  - `POST prepare-upload`: `{info, files: {id: {id, fileName, size, fileType, sha256?, preview?, metadata?}}}`.
    Answers:
    - `{sessionId, files: {id: token}}`
    - 204 nothing wanted
    - 403 refused
    - 409 another session is going
    - 429 too many asks
  - `POST upload?sessionId&fileId&token`: the raw body. Answers 403 for a bad token or address, 409 for another
    session, 422 when the sha256 does not match.
  - `POST cancel?sessionId`
- **TLS.** HTTPS with a self-signed certificate. Our `fingerprint` is the SHA-256 of the certificate's DER, in
  upper-case hex: LocalSend's sender pins it during the handshake (`PinnedServerCertVerifier`), so it must be exact.
  - The server asks for a client certificate but does not require one. When one is presented, its DER SHA-256 is
    the sender's verified fingerprint. LocalSend's newer Rust core sends one. Older senders do not.
  - The certificate is ECDSA P-256 made by `rcgen`, CN "Glance", effectively never expiring. Its key is kept in the
    helper's folder in the user's roaming profile.
- **Port.** 53317 when free, else a port of the system's choosing, announced. (A LocalSend app on this PC holds
  53317. Multicast is shared with `SO_REUSEADDR`.)
- **Sending.** We are a LocalSend client:
  - rustls, with our certificate as the client certificate, pinned to the device's announced fingerprint
  - `prepare-upload`, then one `upload` per file, streamed from disk
  - `cancel` when the user cancels: with the session's id once there is one; while the device's user is still asked
    (no session yet), without one, which LocalSend takes as the sender's from that address. The offer's own
    connection is then given up at once (`CancelIoEx` on its socket: shutting it down wakes no blocked read on
    Windows). This PC as a receiver does the same for a sender: a `cancel` without a session, from the address being
    asked about, takes the asking back from the notice.
  - a 401 to `prepare-upload` asks for the receiver's PIN: the state `pin {tried}` is told to Glance, which asks the
    user for it, and the offer is made again as `prepare-upload?pin=…`. A PIN not right is answered at once with 401
    again (`tried` counts them). One right is held for the device's user like any offer: "waiting" is told only once
    the device has held it for a second, so the notice stays while a PIN is tried.

## Trust

- A device is trusted by its fingerprint, never by its name or address. Trusted devices are kept in the helper's
  configuration (name shown, fingerprint, kind, when last seen). Glance's settings list and remove them by asking
  the helper, as they do for paired phones.
- **Verified** (a client certificate was presented): auto-accept is sound.
- **Claimed** (no client certificate, older LocalSend): the fingerprint in `info` is only what the sender says.
  Someone else on the same network could claim it (LocalSend's own "quick save for favorites" has that weakness).
  Such a device is never trusted: it is asked about each time, and the notice offers no trust for it. Its files,
  once taken, are handled as any: only in the station's folder, named by Glance, marked as from the Internet, and
  capped in size.
- **Glance's Android app** always presents its client certificate. Its fingerprint is told to the helper over the
  pairing channel (HMAC-signed with the phone's key), so the paired phone is trusted, verified, with no prompt.

## Processes

- **`glance-remote.exe`** (runs as the user, never elevated) stays the only process that opens a port. It gains the
  module `nearby`: discovery, the HTTPS server, the client, and the trusted list. It runs while the remote or nearby
  transfer is on (its task starts it at sign-in). Nearby on with the remote off starts it with phone control off.
  The firewall rule "Glance Nearby" is the helper's program-scoped rule, local subnet only: TCP (any port: it
  receives on another where 53317 is taken), and UDP 53317. The uninstaller removes it with the remote's.
- **`glance.exe`** serves a named pipe `\\.\pipe\Glance.Nearby.<user SID>`:
  - DACL: the user and SYSTEM only. `PIPE_REJECT_REMOTE_CLIENTS`.
  - The helper connects and keeps the connection. Messages are JSON lines.
  - helper → Glance:
    - `ask {id, from, files[{name, size, type}]}`, which Glance answers with `answer {id, take, trust}` (60 s,
      else refused)
    - `coming {session, from, files}`
    - `progress {session, done, of}` (at most 10 a second)
    - `came {session, files[{file, name, size}]}`
    - `failed {session, why}`
    - `devices [{id, alias, kind, ...}]` (when the list changes)
    - `sent {job, ...}` (progress and result of a send)
  - Glance → helper: `send {job, to, files[{path, name}]}` (a folder sent as its files, each named by its place in
    it, "folder/inner/file", as LocalSend names them), `cancel {job}`, `pin {job, pin}`, `announce`
  - Everything the helper says is data, checked: a `came` file must be a plain name, and the file must exist in the
    station folder.
- **Glance not running.** Unknown devices are refused, trusted ones taken. The helper writes each finished session
  to `Station\arrived\<session>.json`. Glance takes those in at start and deletes them.
- **Writing a file.** The helper writes it as `Station\<random>.part`. When it is whole (and its sha256 matches,
  where one was given), it is renamed to `<random>.<ext>` (the extension from the sent name: letters and digits
  only), and its mark of the web is written. The name it came with is kept only as the shown name, stripped of any
  path, and is never used as a path. Caps: each file 4 GB, and the session must fit in the drive's free space minus
  1 GB.

## Glance's side

- **Shelf.** `Item` gains `from: Option<Device>` (alias and kind) for the badge and the meta line. Arrivals from the
  same device within 60 s of the last one join that item (the files of one item). The station's room rules (expiry,
  room) apply as to anything kept.
- **Folders sent.** A file of a folder comes named by its place in it ("folder/inner/name", as LocalSend and Glance's
  app name them). The helper keeps that place (`nearby::shown_path`): each part cleaned as a name is, and a part
  that names no folder of its own ("", ".", "..") left out, so nothing reaches out of where it is put. On the shelf
  the files of one folder are one entry (`shelf::entries`): the tile is named after the folder and says "文件夹", the
  edge notice says "收到 n 个文件夹" (folders and files together: "n 项"). Given out (copied, opened, dragged off), the
  folder is made of hard links to the kept files, each in its place, under `named/<its first file>/<folder>`
  (`Folder::sent_folder`), so it goes with the item. Each file of it goes to recent files where it is in that folder.
  (Approved with the six kinds below, mockup 3qdZ2Lukg953DyDfq6mpyv.)
- **Recent files.** It leaves out `AppData`, so arrivals are added to it directly (`recent::came`) with the device
  as their source.
- **Edge notice.** A small layered window at the panel's edge, drawn in the skin like the plates. Its states:
  request, receiving, received. Pressed, it opens the panel, and the picture is carried to the station's head (the
  carry window used by drags). It is not shown while a game is in front.
  - Received, its picture is dragged off it into any program as a station's thing is (approved mockup
    https://claude.ai/artifact/SNAdNfgttk327DHukW3cet): pressed, the plate is drawn at 0.97, and moved past the
    system's drag distance, the station gives the item (`station::drag_off`) and `carry` carries the picture as the
    notice drew it, its room left outlined (`view::paint_kept_room`). Taken, the files are copied and stay on the
    station, and the notice goes 1.2 s later. Not taken, the picture flies back and the notice stays its 4 s again.
    It stays out while pressed or carried.
- **Station tile.** It has a receiving state with a ring, and its picture comes in through the existing morph. The
  device's mark is a corner badge.
- **Send strip.** It shows during a drag that started on the station, over the station's foot, with the devices from
  the helper's list. Hovering one says "发送到 <name>" on the drag's plate. Dropping sends.
  - Each device is a drop target: a window of the station's thread over it (as the station's own catchers), up while
    the station's drag is under way. It sets the drop description on what is dragged, so the plate says it as it says
    Explorer's.
  - The drag ends taken; the station's thread sends what it gave. Words, links and pictures with no file are written
    as files first (as for a drag into a chat), in a folder of the sending's own, deleted as it ends.
  - The device shows "等待对方接受", a ring, then a tick (or what became of it). The strip goes down 1.1 s after the
    sending is over and the drag is over (2.4 s after one that did not go through).
  - While a sending waits on the device's user, asks for a PIN or goes, the pointer on its device shows a cross in its
    round and "取消发送" under it: pressed, the sending is cancelled ("已取消", the strip down 2.4 s later).
- **Outgoing PIN.** The device shows "需要 PIN 码" in the signal colour, and the edge notice (beside a panel up at
  its edge) asks for it: a padlock, "<device> 需要 PIN 码", a box that has the keys at once (its window taken to the
  front; no input method), what is sent under it, 取消 and 发送. Enter or 发送 gives it; while it is tried the box
  takes nothing more. Turned down, the box shakes (360 ms), what was written is chosen, and "PIN 码不正确" shows under
  it. Escape or 取消 cancels the sending. The PIN is used for that offer only and never kept.
- **Settings page "附近传输"** (its own page under 手机互联, the share icon), rows top to bottom:
  - 接收附近设备的文件: the switch (off by default). Turned through the sensors' process, as the remote is
    (`link::Switch::Nearby`, see `phone::set`): the helper told what it runs for (`remote on|off`, `nearby on|off`),
    the firewall rule "Glance Nearby" (TCP on any port for the helper's program, UDP 53317, local subnet only) added
    or removed beside "Glance Remote" as each is on, and the task kept while either is (run anew on every change:
    a running helper is stopped and waited for first, so it takes the flags as it starts). Without the sensors'
    process it is turned in Glance's own, as the remote is (the firewall then changed only where that has the rights).
  - 这台电脑的名称: a box with the name devices see (the computer's name by default). Pressed, it is written in
    place (the sensors' naming box: Enter or a press elsewhere keeps it, Escape leaves it). Named nothing or as the
    computer is, the alias is cleared. Kept by the helper (`nearby alias`), which says it from then on without a
    restart (`Receiver::me` reads it on each answer), and Glance has it announce itself at once.
  - 来自未信任设备的文件: 每次询问 / 一律拒绝 (`nearby others ask|refuse`, read live by the helper).
  - 在屏幕边缘提示: `Settings::nearby_notice`.
  - 保存位置: "中转站文件夹", with 打开文件夹.
  - 已信任的设备: each device with its kind's round icon. A paired phone's device says "已配对的 Glance 手机" and
    "随配对保留" (unpaired, it goes). Another says "电脑 · M月D日起信任" with 移除 (`nearby forget`). None trusted:
    a line on how one becomes trusted.
  - While receiving is off, every row under the switch is dimmed and does nothing.
  - The preview: a phone with LocalSend at its sending, listing the devices nearby as it would. This PC is first,
    under its name as it is being written, and only while it receives ("这台电脑已关闭接收，不在列表中" otherwise).
    Then the devices the helper has found.
  - The helper is asked in turn on one thread of the window's. Its answer is shown once all asked are answered
    (meanwhile, as changed). It is read again while the page shows, whenever its configuration has been written.
- **Room.** The station tells the helper the most one sending (and one file) may be: all the station may keep less
  what pinned things keep (`Shelf::room`), told whenever that changes (`nearby most`), in order, on a thread.

## Android

In the app's settings, under the name of the PC looked at (the mockup's "Glance 安卓应用 › 这台电脑"):
"新照片自动发到这台电脑" (off), with "拍照后几秒内发出，无需打开本应用。只在与电脑处于同一网络时发送。", and, while it
is on, 包括截图 (on), 包括视频 (off), 发送后显示提示 (on). One PC at a time is sent to: turned on for another, it moves
there (the PC sent to now is named under the switch). Unpairing that PC turns it off. (`AutoSend.kt`, `AutoSendCard.kt`.)

- **Turning it on.**
  - The photos are asked for (and the videos, where they are sent, and the notice): `READ_MEDIA_IMAGES` (Android 13
    on), `READ_EXTERNAL_STORAGE` before. Android 14's partial access is told apart (`READ_MEDIA_VISUAL_USER_SELECTED`
    granted alone): new photos would not be seen, so it is explained and not turned on. Refused: said so.
  - The PC is asked, signed, `GET /v1/nearby`. Off there: "这台电脑尚未开启附近传输…" and it stays off. Not reached,
    or no longer paired: said so.
  - The phone's certificate is told it, `POST /v1/nearby/trust {fingerprint, alias}`: the PC trusts it while the
    phone is paired. The alias is the name the phone's user gave it (`Settings.Global.DEVICE_NAME`).
  - What was taken before is not sent.
  - Where the app is optimised for the battery, the system's own question is asked
    (`ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`): held in the background, its jobs do not run. Not let go, the card
    keeps a row "后台运行" saying photos go only once the app is opened, with "允许" to ask again.
  - Some systems keep a setting of their own and hold the app where it was let go (ColorOS: 耗电管理 › 完全允许后台行为),
    which no API tells. So it is seen as it happens: a camera photo found more than 60 s after it was added means the
    app was held (its watch is told within 8 s); one found in time with the app not in front means it is not (in front,
    its jobs run anyway, so that proves nothing). Videos do not count: one is added as it begins. Held, the row says
    the last photos went only once the app was opened, with "设置" opening the system's settings for the app. Opening
    the settings page looks at what came meanwhile, so it is known there at once.
- **Client certificate.** An EC P-256 key in the Android KeyStore (`glance-nearby`, never leaving it), with the
  self-signed certificate the key store makes for it (CN=Glance, 20 years). Fingerprint: SHA-256 of its DER, capital
  hex. Presented whenever the phone sends (`Own.Presenting`).
- **Noticing.** `PhotoWatch`, a `JobService` with content triggers on `MediaStore.Images` (and `Video`, with videos
  sent), descendants, 1.5 s update delay and 8 s at most (a burst is told as one). Re-armed each time it runs, and
  (with a look at what came meanwhile) as the phone starts, as the app is updated (`Booted`) and as it is opened:
  content-trigger jobs are not persisted, and an app stopped by its user loses its jobs.
- **Queueing** (`AutoSend.look`). What was added (by `DATE_ADDED`) since a day before the last look of its kind
  (pictures and videos each have their own: one may be readable while the other is not), not before
  turning on nor before videos were turned on, and not seen yet, is looked at once and remembered as seen (forgotten
  once older than the next look goes back to). It is queued where it is the camera's (`RELATIVE_PATH` under `DCIM/`,
  below Android 10 its path) or a screen's capture ("screen" in its folder: `DCIM/Screenshots`,
  `Pictures/Screenshots`, screen recordings; below Android 10 its bucket's name) and screenshots are sent.
  - What the system still has pending (being written) is not seen till it is published, which notices it again.
    Seen by date rather than by id, it is not missed for being numbered before something published sooner, and the
    day before the last look covers what was pending through it.
  - However long no look ran (the phone kept from it), the next goes back to the last.
  - Without access to all photos (taken away in the system's settings, or only some given), nothing is looked at and
    nothing sent: the queue is kept, and once access is back what came meanwhile is found. The settings say it is
    paused.
- **Sending** (`PhotoSend`). A persisted job, on an unmetered network, backoff exponential from 30 s. What waits goes
  as one sending, and what is queued meanwhile goes after it in the same run.
  - One run at a time sends: it is taken up as its job starts (before its thread does anything, so a late thread of
    a stopped run cannot take it back), and a look queues and schedules the job only where no run is sending (under
    the same lock), so a running one is never replaced; a run with nothing left lets go in the same
    step it finds so. Turning off, or on for another PC, stops the run (it cancels its sending).
  - What was deleted since, or is in the trash, leaves the queue. The PC is asked where it receives (`/v1/nearby`:
    port, its certificate's fingerprint, and the most it takes: `most_file`, `most_sending`, from the station's room
    and the drive's, and `most_files`, as the helper refuses beyond them). Not answered, or answered unsigned (another machine at its
    old address), it is looked for on the network (the remote's `Pc.find`, the address kept). The job never turns
    sending off by itself.
  - What fits in that is offered, in order. What does not waits till the PC has room.
  - LocalSend v2 over HTTPS: the PC pinned to the fingerprint it said, the phone's certificate presented, one request
    per connection. `prepare-upload` (info: the alias, version 2.1, `deviceModel`, `mobile`, the fingerprint, https),
    then each file's `upload`, streamed at its length.
  - 204: nothing wanted. Refused by its user (403 "refused", or "not taken from devices not trusted"), or a PIN wanted
    (401, which Glance never wants): given up. Anything else (403 for room since it said, 409 busy), or not reached:
    tried again later.
  - Each file leaves the queue as soon as the PC took it. One that fails cancels the sending (`/cancel`): the PC
    keeps what it took, and the rest is offered again later.
- **The notice.** "已发到 <PC>" ("已将 N 项发到 <PC>", as many as went), a quiet notification that goes by itself after 6 s, where the
  notice is on and notifications are allowed.

### Android, by hand

Two ways in: a tile "发送文件" on a PC's page, and the system's share sheet, where Glance takes `SEND` and
`SEND_MULTIPLE` of any type (words: `EXTRA_TEXT` where nothing else is shared).

The tile opens a sheet of the six kinds LocalSend's Android app sends, three to a row in its order (approved
2026-11-13, mockup 3qdZ2Lukg953DyDfq6mpyv, `Kinds.kt`). Each sends as it is chosen: there is no list to gather into.

- **文件 and 照片和视频.** The system's document and photo pickers, any number.
- **剪贴板.** Read once as it is chosen (Android says so in its own toast), shown before it goes with
  "剪贴板中可能有密码等私密内容，发送前请确认。". Words or a link go as words. A picture alone is copied into the app's
  cache as "剪贴板_<yyyy-MM-dd_HH-mm>.<its type>" and goes as that file (the clipboard may change before it is sent).
  Files go as files. Nothing on it: "剪贴板中没有内容".
- **文字.** Typed on a sheet over the keyboard, counted ("n 字"), "发送" faded while there is nothing.
- **文件夹.** The system's folder picker (`OpenDocumentTree`, no storage permission). All of it is read through the
  tree (each folder's files before the folders in it, by name) and shown: its name, "n 个文件 · m 个子文件夹 · size",
  the first four by their place in it, the rest counted. Each file goes named "folder/inner/name". The service is
  given the tree with prefix leave to read (not each file: a large folder would not fit in an intent), again on a
  retry.
- **应用.** A page of its own (`AppsActivity`), as a dialog lay under the gesture bar and the keyboard: search, "显示系统
  应用" (off), a box for each app, "发送 n 个应用 · size" at the foot. Listing needs `QUERY_ALL_PACKAGES`. An app goes as
  "<name> <version>.apk", one split into parts ("由 n 个安装包组成") as all of them in a folder "<name> <version>/". The
  packages are files any app reads, sent by their paths. Only paired PCs are sent to, trusted by the certificate told over the
pairing (the tile's sending tells it again each time, so a PC sent to by hand never asks). (`Hand.kt`,
`HandSheet.kt`, `Nearby.kt`, which auto-send uses too.)

- **Which PC.** On a PC's page, that one. Shared from another app (`ShareActivity`, over that app, its own task): the
  one paired, or, with several, a sheet "发送到" listing them, each reached first (`/v1/nearby`): the one sent to last
  first, those not reached or not receiving last and not to be chosen. None paired: said so.
- **One at a time.** A handing goes on in a foreground service (`HandService`, type dataSync), so it goes on when the
  app is left: its notification says how far ("正在发送到 <PC>", n / m, bytes). Another shared meanwhile is told to
  wait ("正在发送其他内容"), and the tile shows the one going on.
- **Reading.** Each thing by its name, type and size as its provider says them (`OpenableColumns`), else as its file
  is, and where neither is known, it is read once into the app's cache and sent from there. Words go as LocalSend's
  message: one text "file" with the words as its preview, answered 204, nothing uploaded.
- **Sending.** The PC is reached (looked for where it moved), off there: "<PC> 未开启附近传输" with how to turn it
  on. A file larger than `most_file`, or all of them more than `most_sending`: said so, nothing sent. Then as many
  as `most_files` at a time, each file's bytes counted as they go. 409 (the PC receiving another, auto-send's
  among them): "正在等待 <PC> 完成其他接收", tried again every 2 s for up to 2 minutes.
- **The sheet.** What goes (four listed, their pictures where they are photos or videos, the rest counted), each
  with its bar, n / m and bytes under them, and "取消". All there: a tick, "已放入中转站" and what went where, gone by
  itself 2 s later. Put away while it goes, it goes on, and opening its notification brings it back.
- **Short.** "已发送 n 个，m 个未能发送" (or "未能发送到 <PC>") and why: not reached, broken off, the PC busy too long,
  no room, too large, unreadable, refused. "重试" (where trying again may go) sends only what the PC does not have.
  Told in a notification too where the app is not in front ("未能全部发送到 <PC>"), as all there is
  ("已发送到 <PC>", gone after 6 s).

Delivery is "within seconds when the phone and PC are on the same network". Doze may delay it, and away from the
PC's network the backoff grows (a photo taken back on it sends what waits at once).

Seen on an OPPO Find N6 (ColorOS 17, 2026-11-12): with the app optimised for the battery, its process is frozen in
the background (the log's `Hans ... blackList` holds its uid), and the content-trigger job, every constraint met, is
never made ready (`Restricted due to: unknown:104`, then `Ready: false` with each flag true) till the app is opened,
when the photo goes at once. Hence the question above.

## Order of work

1. The helper:
   - certificate
   - HTTPS server and the endpoints
   - multicast
   - client
   - trust
   - tests: a LocalSend-like client and server in the tests, both ways, over TLS with and without a client
     certificate
2. The pipe and the arrivals, into the shelf and recent files.
3. The tile's receiving state and the insertion.
4. The edge notice.
5. The send strip and sending.
6. The settings page, the firewall and the task.
7. Android: auto-send, certificate, settings.
8. A live test with the owner's phone (LocalSend and Glance's app), and an iPhone if one is at hand.

## After the design review (Codex, 2026-11-12)

Taken:
- **Trust is only ever verified.**
  - A device is trusted only by a certificate it presented. LocalSend 1.17 and later present one, and so does
    Glance's app.
  - A sender that presents none is asked about every time, without the "trust" box.
  - A device trusted by its certificate is never taken on a fingerprint it only says.
- **Port.** Glance's own HTTPS port is a stable one of its own, 53320 (another, chosen by the system, if that is
  taken), so a LocalSend app started later still gets 53317. The multicast port stays 53317, shared.
- **The pipe** is made as `src/link.rs` makes the sensors' pipes:
  - SDDL for the user and SYSTEM only, with a medium label
  - `FILE_FLAG_FIRST_PIPE_INSTANCE`, `PIPE_REJECT_REMOTE_CLIENTS`
  - each side checks the other's program (Glance: the client is `glance-remote.exe` beside it; the helper: the
    server is `glance.exe` beside it)
  - messages bounded in size
- **Arrivals are journalled.**
  - Every receiving that ends writes a receipt (`receipts/<session>.json`) beside the inbox, in the helper's
    local folder, before Glance is told.
  - Glance imports a receipt idempotently, by session and file. It deletes the receipt only after the shelf (and
    recent files) are saved.
  - The pipe message is only a nudge to import now.
- **Adopting a file.** Glance opens the inbox file by handle, without following reparse points, and checks it is a
  plain file with one link. It renames it into the station's folder by that handle, under Glance's own name.
  Internal names (`station.json`, `named`) can never be produced, because Glance names the file.
- **Writing a file.**
  - The mark of the web goes on the `.part` before it is renamed. If it cannot be written, the file is refused.
  - Partials left by an ended helper are deleted at start.
  - Received sessions idle for 60 s are given up by a timer, not only by the next offer.
- **Size.** The station's size limit bounds what is offered: Glance keeps the most one file may be, and the most a
  receiving may be, in the helper's configuration. An offer larger than the station can hold is refused (403) up
  front.
- **`stays_here`.** Items that may not leave this PC are never sent. A drag including one shows the devices dimmed,
  with the plate saying why.
- **Send leases.** A send job holds its station files (as a drag does) until it ends, including when the helper
  goes away.
- **Lifecycle.** Separate flags (remote, nearby) in the helper's configuration. The task and the firewall rule come
  from their union. The phone-control endpoints answer only with the remote on. The signed status and discovery
  stay available to the paired app in nearby-only mode.
- **Android.**
  - Media permission is asked when auto-send is turned on: full, partial (Android 14 selected photos: explained,
    and auto-send not possible) or denied.
  - A durable per-photo queue, separate from the content trigger, re-armed at boot.
  - Retries with backoff. The cursor advances only on an acknowledged upload. Delivery is stated as "within
    seconds when the phone and PC are on the same network". Doze may delay it.
  - Below API 29 (no `RELATIVE_PATH`), screenshots are told by their bucket name.
- **Outgoing PIN.** A 401 when sending asks the user for the receiver's PIN in the notice, and retries.

Left as they are: the register flow and the version string ("2.1", which current senders accept; register answers
lack port and protocol, so the announcement's are kept).
