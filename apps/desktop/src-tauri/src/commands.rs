//! Every command is `async`, which keeps it off the thread that runs the
//! window. Tauri reads the arguments under camelCase names: `scan_id` here is
//! `scanId` in the interface. `contract/commands.json` holds both sides to
//! the same names.

use std::path::{Path, PathBuf};

use filesweep_core::ScanPolicy;
use filesweep_session::{
    GroupPage, GroupQuery, PageQuery, QueryError, Session, SkipPage, StartError, Status,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SuggestedRoot {
    pub(crate) kind: RootKind,
    pub(crate) path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RootKind {
    Downloads,
    Desktop,
    Documents,
    Pictures,
}

/// A known folder that is missing, or that the system has no name for, is
/// simply not offered.
pub(crate) fn suggestions(
    known: impl IntoIterator<Item = (RootKind, Option<PathBuf>)>,
) -> Vec<SuggestedRoot> {
    known
        .into_iter()
        .filter_map(|(kind, path)| {
            let path = path.filter(|path| path.is_dir())?;

            Some(SuggestedRoot {
                kind,
                path: path.to_string_lossy().into_owned(),
            })
        })
        .collect()
}

/// The user's own folders, as Windows knows them: a Documents folder moved
/// into OneDrive is found where it really is. A whole drive is never offered.
#[tauri::command(async)]
pub(crate) fn suggested_roots<R: Runtime>(app: AppHandle<R>) -> Vec<SuggestedRoot> {
    let folders = app.path();

    suggestions([
        (RootKind::Downloads, folders.download_dir().ok()),
        (RootKind::Desktop, folders.desktop_dir().ok()),
        (RootKind::Documents, folders.document_dir().ok()),
        (RootKind::Pictures, folders.picture_dir().ok()),
    ])
}

type Reveal = dyn Fn(&Path) -> Result<(), String> + Send + Sync;

/// Behind a function so that the tests can stand in for the file manager.
pub(crate) struct Revealer(Box<Reveal>);

impl Revealer {
    pub(crate) fn new(
        reveal: impl Fn(&Path) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        Self(Box::new(reveal))
    }

    /// Explorer, with the file selected. The opener plugin is used as a
    /// library and never registered: registered, its commands would let the
    /// window open any path or address it names.
    pub(crate) fn system() -> Self {
        Self::new(|path| {
            tauri_plugin_opener::reveal_item_in_dir(path).map_err(|error| error.to_string())
        })
    }
}

/// The session's refusals, and the one the system adds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub(crate) enum ShellError {
    StaleScan,
    NoSuchFile,
    RevealFailed { message: String },
}

impl From<QueryError> for ShellError {
    fn from(error: QueryError) -> Self {
        match error {
            QueryError::StaleScan => Self::StaleScan,
            QueryError::NoSuchFile => Self::NoSuchFile,
        }
    }
}

/// The window names the file by the ids of a page it was given and never
/// sends a path, so nothing but a file of the scan can be shown.
#[tauri::command(async)]
pub(crate) fn reveal_file(
    session: State<'_, Session>,
    revealer: State<'_, Revealer>,
    scan_id: u64,
    group_id: u64,
    file_id: u64,
) -> Result<(), ShellError> {
    let path = session.file_path(scan_id, group_id, file_id)?;

    (revealer.0)(&path).map_err(|message| ShellError::RevealFailed { message })
}
