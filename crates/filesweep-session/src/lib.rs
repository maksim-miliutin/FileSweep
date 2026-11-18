//! The layer between the FileSweep scanner and an interface.
//!
//! This crate defines what an interface is shown and what it may ask: a status
//! snapshot, pages of results, and errors with codes. A path crosses that
//! border as display text only; an action names a file by ids.
//!
//! Nothing here depends on a GUI toolkit, and nothing here touches the disk:
//! only the scanner reads it.

#![forbid(unsafe_code)]

mod error;
mod view;

pub use error::{QueryError, StartError};
pub use view::{
    FileView, GroupPage, GroupQuery, GroupSort, GroupView, IndependenceView, MAX_PAGE, PageQuery,
    Phase, ScanView, SkipCode, SkipPage, SkipView, State, Status, SummaryView,
};
