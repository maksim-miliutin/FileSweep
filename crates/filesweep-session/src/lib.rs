//! The layer between the FileSweep scanner and an interface.
//!
//! A [`Session`] runs one scan at a time on a background thread and keeps
//! what the scanner reports. An interface is shown a status snapshot, pages of
//! results, and errors with codes. A path crosses that border as display text
//! only; an action names a file by ids, and [`Session::file_path`] turns the
//! ids back into the exact path.
//!
//! Nothing here depends on a GUI toolkit, and nothing here touches the disk:
//! only the scanner reads it.

#![forbid(unsafe_code)]

mod error;
mod results;
mod session;
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod testkit;
mod view;

pub use error::{QueryError, StartError};
pub use session::{MAX_ROOTS, Session};
pub use view::{
    FileView, GroupPage, GroupQuery, GroupSort, GroupView, IndependenceView, MAX_PAGE, PageQuery,
    Phase, ScanView, SkipCode, SkipPage, SkipView, State, Status, SummaryView,
};
