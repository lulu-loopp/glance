<img src="docs/images/banner-en.webp" width="100%"
     alt="Glance: a system monitor at the edge of your screen. Push the
     pointer there; move away and it's gone. Beside the name, the panel's CPU
     lane in frosted glass, over the desktop.">


[![License: MIT](https://img.shields.io/badge/license-MIT-green)](#licence)
[![Release](https://img.shields.io/github/v/release/lulu-loopp/glance?label=release&color=blue)](https://github.com/lulu-loopp/glance/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/lulu-loopp/glance/total?label=downloads&color=pink)](https://github.com/lulu-loopp/glance/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-8a2be2)](#install)

Glance is a free, open-source system monitor for Windows that stays out of
the way. Push the pointer against the edge of the screen and a panel slides
out with everything your PC is doing; move away and it slides back. Since 0.4
it has two more components, a [workbench](#workbench) for the things you
reach for and [music](#music) that sings along, each on whichever edge you
give it, or torn off onto the desktop.

[Try it in the browser](https://glancepc.com/en/#desk) · [Website](https://glancepc.com/en/) · [中文说明](README.zh-CN.md) · [Download](https://github.com/lulu-loopp/glance/releases/latest) · [Hardware](#hardware) · [Build](#build)

![Glance's three components on one desktop in the frosted-glass look: the workbench at the left edge, the monitor at the right, and music as a strip of lyrics](docs/images/desktop-en.webp)

## What it shows

- **CPU**: usage, every thread (point at one for its clock), clock (each kind's on processors with big and little cores), temperature, package power, each chiplet's temperature
- **Graphics**: usage, video memory, clock, temperature, fan, and the card's power on NVIDIA and AMD cards; Intel Arc graphics' temperature, clock, fan and power, from Intel's own driver library; and a hot spot line on the cards whose driver says it apart (NVIDIA's before the RTX 50 series, AMD's)
- **Memory**: use, its speed (and what the modules are rated for, if it runs below that), and each module's temperature
- **Network and disk**: traffic, the name of the Wi-Fi network if you like, and each drive's temperature, every drive on a line of its own
- **Processes**: the busiest programs by CPU, memory, I/O or GPU
- **Storage**: space on each drive
- **Motherboard**: temperatures and fan speeds
- **Battery**, on laptops: its charge, the power going in or out (on battery, what the whole machine draws) and its health
- **Time and media** (off until chosen): the time and the date (on a 12-hour clock if you like), the sound's level on a track you drag, with mute, and what is playing, paused, played and skipped
- **WSL and Docker**, for developers (off until chosen, under *More modules*): WSL 2's processors, memory and GPU and the distributions running, read from Windows alone so that no distribution is started or kept running; the containers Docker's current engine runs, each with its processors, memory, network and disk
- **Games**: while a fullscreen game runs, at the top: its frame rate, 1% low, a chart of its frame times and the screen's refresh rate; the game's own CPU, GPU, memory and video memory; whether the graphics card is held back by its power or temperature limit (NVIDIA); how long you have played; and whether your microphone is muted

## Workbench

A second panel, for the things you reach for, on the edge you give it (the
left, say, with the monitor on the right). Size it freely and it lays its
blocks out for the room.

- **Station**: drop or paste files, pictures, text and links onto it, and
  they wait there until you drag them into another program. *Recently
  copied* keeps what passed through the clipboard (it can be paused), and
  *Recent files* lists what was just downloaded or saved. Double-click to
  open, right-click to pin, as cards or as a list. What sits too long is
  cleared away.
- **Notes**, in Markdown: each note is a `.md` file in `Documents\Glance 便签`
  (or a folder you choose), so any editor opens it too. Headings, emphasis,
  to-do boxes and links show as they read: tick a box or open a link with a
  click, and the marks show only on the line you are writing. A note can be
  stuck on the desktop and written in right there. What another program
  changes in a note is read in, and nothing is written over unread.
- **Quick launch**: programs, files, folders and web links side by side, a
  click away, picked from every program in the Start menu, from what you used
  lately, or dragged in.
- **Time and media**: the time and the date, and what is playing.

![The workbench at the screen's left edge in the frosted-glass look: the time, the station with files and a link, notes, and quick launch's rail](docs/images/workbench-en.webp)

## Music

What is playing, with its cover, controls, position and volume, and lyrics
that follow the song line by line, word by word where the timing is known.
Size it and it changes form: a strip of one line, a player, a card with the
lines around the one sung, and, wide enough, a stage with a spinning record
and a spectrum. Its full screen shows the lines large over smoke taken from
the cover's colours, moving with the beat. With a clear background it is a
line of lyrics floating over the desktop, and each part (the cover, the title,
the translation, the spectrum, the controls) can be switched off.

Lyrics are looked up on NetEase Cloud Music, QQ Music, Kugou, LRCLIB and
Apple Music (through its service PaxSenix, nothing to sign in to) at
once. A local file's own lyrics come first, and a translation is shown where
there is one. Any player whose song shows in Windows' media flyout works.
NetEase Cloud Music tells its position only through the InfLink-rs plug-in,
which the settings install in one click.

![Music on the desktop as a stage: a spinning record, the song's lines with the one sung lit word by word, and a spectrum](docs/images/music.webp)

## Desktop widgets

Drag a panel away from the edge, the monitor, the workbench or music, and it
tears off as a widget that stays on the desktop: pulled by one block's
heading, that block alone, otherwise the whole panel. Size it by any edge or
corner and it lays itself out for the room: the monitor's lanes, tiles, a
chart with the figures beneath, a strip, or a few rings, each turning
smoothly into the next. Bring it near an edge of the screen and it shows
where it would stick. Let go and it sticks there, as large as it is, and
slides along the edge as you drag it. Throw it off the screen to put it away
(the tray icon's menu brings it back). Hover over it for its row of buttons (rest on one, and it says what it does):
the frame rate of the window in front (for a game Glance does not recognise as
one), the settings, a pin that locks its place and size, click-through, and
put away. With click-through on, clicks reach what is behind; to turn it off,
rest the pointer on the widget a moment and press the one button that shows.
Right-click it for the same, or to keep it
on the desktop under your windows rather than above them (Win+D shows it with
the desktop), to put it away, or for the settings. Widgets come back where you left them when Glance
starts, in the panel's look.

![Widgets on the desktop in the frosted-glass look: the panel's lanes, tiles, a chart, a strip and rings](docs/images/widgets-en.webp)

## The overlay

A few readings floating over the screen, all the time or only while a game
runs. Laid out as a card (the frame rate large, its
last minute charted beside it, the rest in tiles) or as a strip of one to
three rows, on tinted glass: dark or light by what is around it, just enough
for every figure to read (what is behind is not blurred: that would cost a
game frames). Drag it into place, drag its
edges to size it. Hover over it for the same row of buttons a widget has (the
frame rate of the window in front, the settings, lock, click-through, close);
its right-click menu has them too. Turn the mouse wheel
over it, or use the slider in the settings, to make the glass clearer, down
to the figures alone over the screen. With click-through on, clicks reach the
game behind; to turn it off, rest the pointer on the overlay a moment and
press the one button that shows. Choose what it shows
group by group, as for the panel (frame rate, CPU, GPU, memory, video memory,
download and upload, microphone, the time), each group's figures one by one, and drag
the groups into the order you want.

![The overlay as a card: the frame rate large, charted beside it, the CPU and GPU in tiles](docs/images/overlay-card-en.webp)

![The overlay as a strip, in one row](docs/images/overlay-strip-en.webp)

Frames are counted from Windows' own event tracing: nothing is injected into
a game and its process is never opened, so anti-cheat has nothing to object
to. Games using DirectX are covered; Vulkan and OpenGL games show no frame
rate yet.

## The phone remote

From bed, with the PC still downloading: a small app on your phone turns
its screens and case lights off, sets the volume and each monitor's
brightness, pauses or skips what is playing, changes the power mode, puts
the PC to sleep, and shows how warm it runs. Night mode does the first
three at once, and ends at a touch of the mouse.

- **Turn it on** in the settings, under *Phone*. It is off until then, and
  Glance itself never opens a port: the remote is a second small program
  (`glance-remote.exe`) that runs as you, never elevated, and is let
  through the firewall for the local network only.
- **Get the app** at [glancepc.com/app](https://glancepc.com/app) (Android
  8.0 and later, an APK attached to each release here). An iPhone can use
  the web page the remote itself serves, until there is an app for it.
- **Pair** by scanning the code the settings show. Each phone gets a key of
  its own, good for that phone alone, which the settings can remove. Every
  request is signed with it, nothing goes through a server, and nothing
  works from outside your network.
- **On the PC too**: the right of the settings' *Phone* page is the app itself, pressed and slid there. The colours you keep for the lights are kept on the PC: the phone and the PC show the same ones.
- **Lights**: a motherboard's and a graphics card's lights are set through
  [OpenRGB](https://openrgb.org), an open-source program under the GPL,
  which is downloaded from its own project (about 22 MB) only when you turn
  *Control the lights* on, and removed when you turn it off. A Thermalright
  cooler's display goes out with them.
- **Laptops**: the app draws a laptop, and its lights are the light under
  its keys (ASUS laptops, for now): its levels, and on a keyboard that takes
  a colour, its colour and what it does, with no need of OpenRGB. With the
  screens put out the laptop does not stand by: touch it and they are on,
  without signing in again.
- The web page is served over plain HTTP: use it only on a network you
  trust. The app's requests cannot be forged or replayed by someone
  listening, but what the PC answers (temperatures, the song playing) is
  not hidden from them.

## Three looks

Chart paper, frosted glass (the desktop behind bends at the rim of each
piece) and Windows 11 (acrylic, as the Start menu draws it). Light or dark,
or following the brightness of your wallpaper. Chinese or English.

![The three looks, dark: chart paper, frosted glass, Windows 11](docs/images/looks-en.webp)

## Small and quiet

One executable of about 2 MB, drawn natively with Direct2D and
DirectComposition. About 55 MB of memory and well under 1% of a CPU core
while the panel is hidden. No account, no ads, and no network access but
one daily question about a newer version (to Glance's own site, and to
GitHub and Gitee), which the settings can turn off; the diagnostics you choose to send from the
settings (shown to you whole first, with a few words of your own and
screenshots if you like); and the phone remote, if you turn it
on.

## Install

1. Download `Glance_<version>_x64-setup.exe` from the
   [latest release](https://github.com/lulu-loopp/glance/releases/latest) and run it
   (the same installer is mirrored on [Gitee](https://gitee.com/lulu-loopp/glance/releases)
   for mainland China).
2. The installer is signed (publisher: Weiyi Shi). As the signature is new,
   Windows SmartScreen may still say "Windows protected your PC" for the first
   downloads; choose **More info → Run anyway**.
3. Glance asks for administrator rights once, on its first start: reading
   temperatures, power and fans needs them, through the signed
   [PawnIO](https://github.com/namazso/PawnIO) driver the installer sets up.
   After that it starts without asking, and can start with Windows.

**Where to install.** Glance starts with administrator rights without asking
only from a folder no ordinary program can change: Program Files (the
default), or a new folder at the root of a drive, such as `D:\Glance`. The
installer explains if you choose somewhere else; installed there, Glance asks
for administrator rights at every start and cannot start with Windows.

## Use

- **Open the panel**: push the pointer against the screen's right edge (the
  push needed, where the panel appears, how many columns it takes and how
  large it is are in the settings). The settings' *Layout* page shows your
  screens as Windows arranges them: drag the monitor, the workbench or music
  onto any edge, several onto one if you like. An edge holding nothing opens
  nothing. A deliberate push is needed, so scroll
  bars at the edge stay usable. With several screens, the settings show them
  as they are arranged: click an edge of any of them, left, top or right, for
  the panel to open from it, and again to turn it off. Where an edge leads on
  to another screen, rest the pointer on it for half a second instead. Or press **Ctrl+Alt+G** (or a shortcut of your own),
  which opens it at the pointer and closes it again.
- **Games**: while a fullscreen game runs, a Game lane comes at the top of
  the panel; for its frame rate over the game itself, turn on "Show while
  playing" on the settings' Overlay page. Over a fullscreen game, borderless
  or exclusive, the settings choose what may open the panel (the shortcut
  only, by default: a push into the edge may be an accident mid-game). A game
  set to exclusive "Fullscreen" steps out while anything shows over it, as
  Windows works, and comes back when the panel closes; windowed games are
  covered as anything else is.
- **Choose what it shows**: in the settings, each module (CPU, each graphics
  card, memory, network, disk…) opens to its own items, each switched on or
  off; drag modules to change their order. WSL and Docker are folded away
  under *More modules* until switched on.
- **Close it**: move the pointer away, or press Esc.
- **Size it**: drag the panel's edges, as soon as it opens.
- **Keep it on the desktop**: drag the panel away from the edge and it tears
  off as a widget (see [Desktop widgets](#desktop-widgets)), onto another
  screen too.
- **Pin it**: the pin in its bottom bar keeps it open while you work elsewhere, and open again when Glance starts.
- **At a glance**: hovering over the tray icon shows the CPU, graphics and
  memory in brief. A heat alert (off unless you turn it on) tells from the
  tray when the CPU or a graphics card stays at the temperature alert for
  30 seconds.
- **Settings**: right-click the tray icon, or start Glance again. The settings
  window works with the keyboard as well (Tab, the arrow keys, Space).
- **Update**: once a day Glance asks its own site (glancepc.com), and GitHub
  and its mirror on Gitee, for a new version, using whichever it can reach.
  Asking its site, Glance says which version it is, whether this is a new
  install's first asking, which version it was updated from, and how many
  weeks it has been on this PC; the site counts those, to know how many
  copies run and are kept up to date. Nothing tells one PC from another:
  no name or number is sent, and the site keeps no address. When one is out, the tray says
  so and the settings offer it. Glance downloads the installer and runs it only if it is signed
  by the same publisher as the Glance you have.
- **Uninstall**: from Windows' Apps list, or from the settings. It removes
  Glance, its scheduled tasks and its settings, and asks whether to remove the
  PawnIO driver too (other monitoring tools may use it).

## Hardware

| | Supported | Tried on real hardware |
|---|---|---|
| CPU | AMD Ryzen (Zen and later), Intel Core | AMD Ryzen 9 9950X |
| Motherboard sensors | ITE and Nuvoton (NCT67xx, and the NCT6683/6686/6687 on most MSI boards) Super I/O chips | ITE IT8689E |
| Memory temperature | DDR4 and DDR5 modules with a sensor | DDR5 |
| Graphics power | NVIDIA (NVML), AMD (ADL) discrete cards | NVIDIA RTX 5070 Ti |
| Everything else | through Windows (the sources Task Manager uses) | |

Reports from Intel and Nuvoton machines are welcome: **Settings → System →
Diagnostics → Copy** puts what Glance found on your machine, and which readings it gets,
on the clipboard, with the latest lines of its log, ready to paste into an
issue (no paths or names in it). The log itself is `glance.log`, beside the
settings in `%APPDATA%\dev.weiyi.glance`. On laptops, fans and
board temperatures belong to the laptop's embedded controller and are not
shown, except an ASUS laptop's fans, which its firmware reports; integrated graphics draw from the CPU package, whose power the CPU
lane shows.

## Build

```
cargo build --release
makensis installer\glance.nsi
```

`scripts\release.ps1` does both, and builds the phone remote (`remote\`, a
crate of its own); with `-Sign` it signs the executables, the uninstaller
and the installer (Microsoft Artifact Signing, through the publisher's own
`scripts\sign.ps1`, which the repository does not hold). The
Android app is in `android\` (`gradlew assembleRelease`, JDK 17). A debug build runs without asking for administrator rights, and so
without the driver's sensors.

The pictures here and the promotional video are drawn by Glance itself:
`cargo build --release --features studio` adds `glance --studio script.json
out-folder`, which renders the panel off screen, frame by frame, from a
script of shots.

## Licence

Glance is released under the MIT licence ([LICENSE](LICENSE)). It ships with
and builds in third-party software under its own licences: the PawnIO driver
setup (GPL-2.0 with an exception; its source is attached to every release),
the PawnIO modules (LGPL-2.1, with their source in `pawnio-modules/source`),
the Archivo and Inter fonts (OFL 1.1) and Rust crates; see
[licenses/THIRD-PARTY.txt](licenses/THIRD-PARTY.txt).
