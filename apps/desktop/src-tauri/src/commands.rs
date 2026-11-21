//! Every command is `async`, which keeps it off the thread that runs the
//! window. Tauri reads the arguments under camelCase names: `scan_id` here is
//! `scanId` in the interface. `contract/commands.json` holds both sides to
//! the same names.

use std::path::PathBuf;

use filesweep_core::ScanPolicy;
use filesweep_session::{
    GroupPage, GroupQuery, PageQuery, QueryError, Session, SkipPage, StartError, Status,
};
use tauri::State;

#[tauri::command(async)]
pub(crate) fn scan_start(
    session: State<'_, Session>,
    roots: Vec<String>,
    include_hidden: bool,
) -> Result<Status, StartError> {
    let policy = ScanPolicy {
        include_hidden,
        ..ScanPolicy::default()
    };

    session.start(roots.into_iter().map(PathBuf::from).collect(), policy)
}

#[tauri::command(async)]
pub(crate) fn scan_pause(session: State<'_, Session>) -> Status {
    session.pause()
}

#[tauri::command(async)]
pub(crate) fn scan_resume(session: State<'_, Session>) -> Status {
    session.resume()
}

#[tauri::command(async)]
pub(crate) fn scan_cancel(session: State<'_, Session>) -> Status {
    session.cancel()
}

#[tauri::command(async)]
pub(crate) fn scan_status(session: State<'_, Session>) -> Status {
    session.status()
}

#[tauri::command(async)]
pub(crate) fn scan_groups(
    session: State<'_, Session>,
    scan_id: u64,
    query: GroupQuery,
) -> Result<GroupPage, QueryError> {
    session.groups(scan_id, &query)
}

#[tauri::command(async)]
pub(crate) fn scan_skipped(
    session: State<'_, Session>,
    scan_id: u64,
    query: PageQuery,
) -> Result<SkipPage, QueryError> {
    session.skipped(scan_id, &query)
}
