//! The scanner behind FileSweep: it finds files with identical content.
//!
//! It only reads. Nothing on disk is created, changed, renamed or deleted, and
//! `clippy.toml` in this crate rejects the std calls that would do so.

#![forbid(unsafe_code)]
