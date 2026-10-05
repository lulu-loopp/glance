# Glance

A system monitor for Windows that stays out of the way: push the pointer
against the edge of the screen and a panel slides out with CPU, GPU, memory,
network, disk, process, storage and motherboard readings; move away and it
slides back.

- Opens on a deliberate push into the edge (raw mouse input), not on merely
  touching it, so scroll bars and window borders at the edge stay usable.
- Three looks: chart paper, frosted glass (the desktop behind is bent at the
  rim of each piece) and Windows 11 (acrylic, as the Start menu draws it),
  light or dark, in Chinese or English.
- Temperatures, power, fans and clocks come from the CPU and the
  motherboard's sensor chip through the signed PawnIO driver, which Glance
  installs itself on its first start; the rest from Windows' own counters
  (PDH, D3DKMT, the process list), the same sources Task Manager uses.
  Supported sensor chips: AMD Ryzen (family 17h and later) and Intel Core
  CPUs, ITE and Nuvoton Super I/O chips, DDR4 and DDR5 memory temperature
  sensors. Only AMD, ITE and DDR5 have been tried on real hardware so far;
  reports from Intel and Nuvoton machines are welcome. On laptops the fans
  and board temperatures belong to the laptop's embedded controller and are
  not shown. A discrete graphics card's power comes from its own driver
  (NVIDIA's NVML, AMD's ADL); integrated graphics draw from the CPU
  package, whose power the CPU lane shows.
- Small: one 0.8 MB executable, drawn natively with Direct2D, DirectWrite
  and DirectComposition; about 55 MB of memory and well under 1% of a core
  while the panel is hidden.

The sensors need administrator rights. The first start asks once; Glance then
registers a scheduled task that starts it elevated without asking again, and
can start with Windows. For that it must be where no ordinary program can
change it, so the installer installs only there: into Program Files, or into
a new folder at a drive's root such as `D:\Glance` (created changeable by
administrators only); a folder inside one your account owns, an existing
folder of other things or a link is refused, and left as it was.
Left-click the tray icon for the panel, right-click it (or start Glance again)
for the settings. Uninstall from Windows' Apps list or from the settings; it
removes Glance, its scheduled tasks and its settings, and asks whether to
remove the PawnIO driver too.

## Build

```
cargo build --release
makensis installer\glance.nsi
```

The executable is `target\release\glance.exe`, the installer
`target\Glance_0.1.1_x64-setup.exe`. A debug build runs without asking for
administrator rights (and so without the driver's sensors).

## Licence

Glance is released under the MIT licence (`LICENSE`). It ships with and
builds in third-party software under its own licences: the PawnIO driver
setup (GPL-2.0 with an exception), the PawnIO modules (LGPL-2.1, with their
source in `pawnio-modules/source`), the Archivo and Inter fonts (OFL 1.1) and
Rust crates; see `licenses/THIRD-PARTY.txt`.
