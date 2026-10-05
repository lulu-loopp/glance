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
- Small: one 0.8 MB executable, drawn natively with Direct2D, DirectWrite
  and DirectComposition; about 55 MB of memory and well under 1% of a core
  while the panel is hidden.

The sensors need administrator rights. The first start asks once; Glance then
registers a scheduled task that starts it elevated without asking again.
Left-click the tray icon for the panel, right-click it (or start Glance again)
for the settings.

## Build

```
cargo build --release
makensis installer\glance.nsi
```

The executable is `target\release\glance.exe`, the installer
`target\Glance_0.1.0_x64-setup.exe`. A debug build runs without asking for
administrator rights (and so without the driver's sensors).
