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

Not in v1: receiving text messages as notes, the reverse-download (browser) mode, and a PIN of this PC's own.

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
  Someone else on the same network could claim it. LocalSend's own "quick save for favorites" has the same
  weakness. What limits the harm:
  - files only ever land in the station's folder, named by Glance and marked as from the Internet
  - sizes are capped
  - the settings say so in one line
- **Glance's Android app** always presents its client certificate. Its fingerprint is told to the helper over the
  pairing channel (HMAC-signed with the phone's key), so the paired phone is trusted, verified, with no prompt.

## Processes

- **`glance-remote.exe`** (runs as the user, never elevated) stays the only process that opens a port. It gains the
  module `nearby`: discovery, the HTTPS server, the client, and the trusted list. It runs while the remote or nearby
  transfer is on (its task starts it at sign-in). Nearby on with the remote off starts it with phone control off.
  The firewall rule is the helper's program-scoped rule, local subnet only: TCP on its ports, UDP 53317.
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
- **Recent files.** It leaves out `AppData`, so arrivals are added to it directly (`recent::came`) with the device
  as their source.
- **Edge notice.** A small layered window at the panel's edge, drawn in the skin like the plates. Its states:
  request, receiving, received. Pressed, it opens the panel, and the picture is carried to the station's head (the
  carry window used by drags). It is not shown while a game is in front.
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
- **Settings page.** As listed above. Turning it on adds the firewall rule and the task (the same path as the remote).

## Android

- A `JobService` with `TriggerContentUri` on `MediaStore.Images.Media.EXTERNAL_CONTENT_URI` (descendants)
  rescheduled each time. It sends the images added since the last one sent, in their order. Screenshots are told
  apart by their relative path (`Screenshots`) and left out when the switch for them is off.
- **Client certificate.** An EC P-256 key in the Android KeyStore, with its self-signed certificate. Its fingerprint
  is sent to the helper once over the signed pairing channel.
- **Finding the PC.** The paired PC's address (as the remote finds it today), plus the nearby port, which the helper
  says in its signed status.
- **Sending.** LocalSend v2 over HTTPS: pinned to the PC's fingerprint (told at pairing, in the signed status),
  presenting the client certificate.
- A short notice after sending, which can be switched off.

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
