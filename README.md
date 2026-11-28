# FileSweep

FileSweep is a free Windows application for finding duplicate files and
removing the extra copies safely. It works offline: no account, no cloud
service, no telemetry.

## Status

Early development. There is nothing to install yet.

What works: a scanner, a Rust library that reads the folders you choose and
reports files with identical content, and a desktop window over it. Both only
read: nothing is created, changed, moved or deleted.

Planned: moving selected copies to the Recycle Bin after explicit
confirmation (the rules are in [docs/recycle-bin.md](docs/recycle-bin.md)),
finding visually similar photos, and an installer.

## Building

You need:

- [Rust](https://rustup.rs). The toolchain version is pinned in
  `rust-toolchain.toml`. Run `rustup toolchain install` in the repository
  folder to get it.
- On Windows, the Microsoft C++ Build Tools with the "Desktop development with
  C++" workload. Rust uses its linker.

```
cargo test
cargo build --release
```

These build and test the libraries. The desktop shell is checked on its own,
because it needs a WebView to build: Windows has one, and on Linux it takes
the WebKitGTK development packages from the
[Tauri prerequisites](https://tauri.app/start/prerequisites/).

```
cargo test -p filesweep-desktop
```

## Running the desktop app

You also need [Node.js](https://nodejs.org) 22 or newer.

```
cd apps/desktop
npm ci
npm run tauri dev
```

The first run compiles the shell, which takes a few minutes. The window is a
plain working page for now. In it you can:

- choose folders, or take the Downloads, Desktop, Documents and Pictures
  folders it suggests, and run a scan, pause it or cancel it;
- page through the groups of identical files, the most space first, and show
  any file in its folder or copy its path;
- see when the files were last used, year by year, the largest first within a
  year;
- read what was skipped and why, counted by reason.

It removes nothing.

"Last used" is the latest of when a file was changed, opened or created, as
far as Windows records it. Windows often does not record opening: by default
it stops doing so when the system drive is larger than 128 GB. So a file may
have been used later than the window says. From an administrator command
prompt, `fsutil behavior query disablelastaccess` tells how your computer is
set.

`npm run dev` alone serves the same page to a browser, where a demo backend
plays a short scan from memory and no disk is read.

The checks of the interface code, from the same folder:

```
npm run lint
npx tsc --noEmit
npm test
npm run build
```

## Trying the scanner

```
cargo run --release -p filesweep-core --example scan -- <folder>...
```

It prints the groups of identical files it finds, what it skipped and why,
and a summary. It changes nothing on disk.

## Layout

- `crates/filesweep-core`: the read-only scanner library.
- `crates/filesweep-session`: runs a scan in the background and serves its
  progress and results to an interface, as a status snapshot and pages.
- `apps/desktop`: the interface, written in React and TypeScript and built
  with Vite.
- `apps/desktop/src-tauri`: the desktop shell, a Tauri window whose commands
  carry the interface's calls to the session.
- `contract/`: examples of the messages and calls the interface exchanges
  with the session. Both sides are tested against them.

## License

MIT. See [LICENSE](LICENSE).
