# Glance

A system monitor for Windows that stays out of the way: push the pointer
against the edge of the screen and a panel slides out with CPU, GPU, memory,
network, disk, process and storage readings; move away and it slides back.

- Opens on a deliberate push into the edge (raw mouse input), not on merely
  touching it, so scroll bars and window borders at the edge stay usable.
- The window never moves under the pointer while open, so it cannot flicker
  between shown and hidden.
- Three looks: chart paper, liquid glass (the desktop behind is refracted at
  the rim of each piece) and Windows 11 (acrylic, as the Start menu draws it).
- Settings, module order and details are chosen inside the panel.

Built with Tauri 2 (Rust and WebView2). Readings come from Windows' own
counters (PDH, D3DKMT, the process list), the same sources Task Manager uses;
no driver and no administrator rights are needed.

## Build

```
npm install
npx tauri build
```

The installer is written to `src-tauri/target/release/bundle/nsis/`.
