# FileSweep

FileSweep is a free Windows application for finding duplicate files and
removing the extra copies safely. It works offline: no account, no cloud
service, no telemetry.

## Status

Early development. There is nothing to install yet.

The first piece is the scanner, a Rust library that reads the folders you
choose and reports files with identical content. It only reads: it never
creates, changes, moves or deletes anything.

Planned after the scanner: a desktop interface, moving selected copies to the
Recycle Bin after explicit confirmation, and finding visually similar photos.

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

## Trying the scanner

```
cargo run --release -p filesweep-core --example scan -- <folder>...
```

It prints the groups of identical files it finds, what it skipped and why,
and a summary. It changes nothing on disk.

## Layout

- `crates/filesweep-core`: the read-only scanner library.

## License

Not chosen yet. A `LICENSE` file will be added before the first release.
