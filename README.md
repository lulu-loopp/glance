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
out with everything your PC is doing; move away and it slides back.

[Website](https://glancepc.com/en/) · [中文说明](README.zh-CN.md) · [Download](https://github.com/lulu-loopp/glance/releases/latest) · [Hardware](#hardware) · [Build](#build)

![Glance's panel in the frosted-glass look, over the desktop](docs/images/desktop-en.webp)

## What it shows

- **CPU**: usage, every thread, clock, temperature, package power, each chiplet's temperature
- **Graphics**: usage, video memory, clock, temperature, fan, and the card's power on NVIDIA and AMD cards
- **Memory**: use, and each module's temperature
- **Network and disk**: traffic, and the drive's temperature
- **Processes**: the busiest programs by CPU, memory, I/O or GPU
- **Storage**: space on each drive
- **Motherboard**: temperatures and fan speeds
- **Battery**, on laptops
- **Games**: while a fullscreen game runs, at the top: its frame rate, 1% low, a chart of its frame times and the screen's refresh rate; the game's own CPU, GPU, memory and video memory; whether the graphics card is held back by its power or temperature limit (NVIDIA); how long you have played; and whether your microphone is muted

## The overlay

A few readings floating over the screen, all the time or only while a game
runs, on the game's screen. Drag it into place, drag its edges to size it,
right-click it to lock it or close it. Choose its readings one by one (frame
rate, 1% low, frame time, the CPU's and GPU's use, heat and power, memory,
video memory, network, microphone), and whether they sit on a plate (grey
or blue) or bare, each in a thin dark halo.

Frames are counted from Windows' own event tracing: nothing is injected into
a game and its process is never opened, so anti-cheat has nothing to object
to. Games using DirectX are covered; Vulkan and OpenGL games show no frame
rate yet.

## Three looks

Chart paper, frosted glass (the desktop behind bends at the rim of each
piece) and Windows 11 (acrylic, as the Start menu draws it). Light or dark,
or following the brightness of your wallpaper. Chinese or English.

![The three looks, dark: chart paper, frosted glass, Windows 11](docs/images/looks-en.webp)

## Small and quiet

One executable of about 1 MB, drawn natively with Direct2D and
DirectComposition. About 55 MB of memory and well under 1% of a CPU core
while the panel is hidden. No account, no ads, and no network access but
one daily question to GitHub and Gitee about a newer version, which the
settings can turn off.

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
  edge, the push needed, where the panel appears, how many columns it takes
  and how large it is are in the settings, and pushing can be turned off).
  A deliberate push is needed, so scroll bars at
  the edge stay usable. Or press **Ctrl+Alt+G** (or a shortcut of your own),
  which opens it at the pointer and closes it again.
- **Games**: while a fullscreen game runs, a Game lane comes at the top of
  the panel; for its frame rate over the game itself, turn on "Show while
  playing" on the settings' Overlay page. The panel shows over borderless
  and windowed games as over anything else. A game set to exclusive "Fullscreen" steps out while
  anything shows over it, as Windows works; the settings choose what may open
  the panel over one (the shortcut only, by default), and the game comes back
  when the panel closes.
- **Choose what it shows**: in the settings, each module (CPU, each graphics
  card, memory, network, disk…) opens to its own items, each switched on or
  off; drag modules to change their order.
- **Close it**: move the pointer away, or press Esc.
- **Keep it open**: the pin in its bottom bar holds it on screen while you
  work elsewhere; unpin it, or press the shortcut, to let it go. Pinned, it
  can be dragged anywhere, onto another screen too, and sized by its edges; moved off the edge, it stays there, Glance restarting
  too, until it is unpinned.
- **At a glance**: hovering over the tray icon shows the CPU, graphics and
  memory in brief. A heat alert (off unless you turn it on) tells from the
  tray when the CPU or a graphics card stays at the temperature alert for
  30 seconds.
- **Settings**: right-click the tray icon, or start Glance again. The settings
  window works with the keyboard as well (Tab, the arrow keys, Space).
- **Update**: once a day Glance asks GitHub, and its mirror on Gitee, for a
  new version, using whichever it can reach. When one is out, the tray says
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
shown; integrated graphics draw from the CPU package, whose power the CPU
lane shows.

## Build

```
cargo build --release
makensis installer\glance.nsi
```

`scripts\release.ps1` does both; with `-Sign` it signs the executable, the
uninstaller and the installer (`scripts\sign.ps1`, Microsoft Artifact
Signing). A debug build runs without asking for administrator rights, and so
without the driver's sensors.

The pictures here and the promotional video are drawn by Glance itself:
`cargo build --release --features studio` adds `glance --studio script.json
out-folder`, which renders the panel off screen, frame by frame, from a
script of shots; `studio\readme.py` and `studio\compose.py` put the frames
together (`studio\`).

## Licence

Glance is released under the MIT licence ([LICENSE](LICENSE)). It ships with
and builds in third-party software under its own licences: the PawnIO driver
setup (GPL-2.0 with an exception; its source is attached to every release),
the PawnIO modules (LGPL-2.1, with their source in `pawnio-modules/source`),
the Archivo and Inter fonts (OFL 1.1) and Rust crates; see
[licenses/THIRD-PARTY.txt](licenses/THIRD-PARTY.txt).
