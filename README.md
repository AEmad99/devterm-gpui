# DevTerm on GPUI

A Rust rewrite of [DevTerm](https://github.com/AEmad99/devterm) on [GPUI](https://github.com/zed-industries/zed), Zed’s GPU UI framework. This is the first slice: the desktop window keeps the Electron app’s shape, and a local shell is already a real PTY.

The Electron app stays the product. This repository is the migration.

## Window

The first screen is the terminal.

- Left rail, 40px: Files, Connections, Workspaces, Snippets, then Git, dictation, shortcuts, and settings
- Library column, 280px, for the rail section that is open
- Group bar (`Group 1`, new group, Save)
- Getting-started row: local terminal, SSH connection, DevTerm Agent
- Pane tab strip over the terminal, with split, new terminal, and the agent mark
- Docked agent column
- Git column
- Status bar: `Local · Linux`, activity, transfers, DevTerm

Colors are Tokyo Night, the same boot palette as the Electron app (`#16161e`, `#1a1b26`, `#7aa2f7`).

## What works

- Local shell via `portable-pty` (in-box ConPTY on Windows; bundled `OpenConsole.exe` only when `DEVTERM_USE_BUNDLED_CONPTY=1` and `conpty.dll` is beside the executable)
- `alacritty_terminal` grid: truecolor, scrollback, selection, bracketed paste, mouse tracking, focus reports, per-pane find
- OSC 7 cwd and OSC 133 A/B, including the shell-integration strings the Electron app injects
- Files follow that cwd, filter, and open in the editor (5 MiB limit, original newline on save, sanitized Markdown preview)
- Settings JSON in the OS config directory, all ten themes, export/import without secrets
- Focus mode and zen mode hide chrome and do not scale terminal text
- SSH connect with `russh` (agent by default, TOFU known hosts, mismatch rejected, TCP no-delay)
- Workspace save writes `workspaces.json`
- DevTerm Agent launch uses the same command builder as the Electron app and spawns the bundled Node runtime when it is installed
- `cargo test` ports the original logic tests (layout, SSH, transfers, agents, browser guard, search, settings, and the rest of `src/logic`)

## Package

```bash
scripts/package-linux.sh
```

The Windows installer script is `packaging/devterm.nsi`. It does not require a bundled ConPTY. Unsigned builds should set `CSC_IDENTITY_AUTO_DISCOVERY=false` if Windows code-signing symlinks fail. App id stays `com.devterm.app`.

## Run

```bash
cargo run
```

GPUI 0.2.2 needs Rust 1.85 or newer (this repo pins 1.99) and a Vulkan driver. On Linux without a GPU, lavapipe works:

```bash
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json cargo run
```

`xattr` 0.2.3, pulled in by GPUI, still names `libc::ENOATTR`. Newer `libc` removed that constant. `vendor/xattr` is the same 0.2.3 crate with Linux `ENOATTR` mapped to `ENODATA`.

## Tests

```bash
cargo test
```

Covers the VT grid and SSH config host parsing.
