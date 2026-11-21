//! The commands as the window calls them: through Tauri's own IPC, on its
//! mock runtime, with the names and argument shapes of
//! `contract/commands.json`. No window is shown and no WebView is needed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use filesweep_core::{DuplicateGroup, ScanEvent};
use filesweep_session::{Session, testkit};

use crate::commands::{Revealer, RootKind, ShellError, SuggestedRoot, suggestions};
use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager, WebviewWindow, WebviewWindowBuilder};

/// The application with one window, as the tests see it.
struct Window {
    webview: WebviewWindow<MockRuntime>,
    // Dropping the application would take the session and the window with it.
    app: App<MockRuntime>,
    revealed: Arc<Mutex<Vec<PathBuf>>>,
}

fn window() -> Window {
    window_revealing(Ok(()))
}

/// The revealer of this window writes down every path it is given and
/// answers each with `answer`. No file manager is opened.
fn window_revealing(answer: Result<(), String>) -> Window {
    let revealed = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&revealed);
    let revealer = Revealer::new(move |path| {
        log.lock()
            .expect("no test panicked while holding the log")
            .push(path.to_path_buf());
        answer.clone()
    });

    let app = crate::shell(mock_builder(), revealer)
        .build(mock_context(noop_assets()))
        .expect("the application builds on the mock runtime");
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("a mock window opens");

    Window {
        webview,
        app,
        revealed,
    }
}

impl Window {
    fn revealed(&self) -> Vec<PathBuf> {
        self.revealed
            .lock()
            .expect("no test panicked while holding the log")
            .clone()
    }

    /// A scripted scan that has found one group of two files and stands
    /// still. The director has to be kept: dropping it ends the scan.
    fn scan_with_one_group(&self) -> (u64, DuplicateGroup, testkit::Director) {
        let session = self.app.state::<Session>();
        let (started, director) = testkit::start_scripted(&session, vec![testkit::root()])
            .expect("a scripted scan starts");
        let group = testkit::group(4096, 4096);
        director.emit(ScanEvent::Group(group.clone()));
        let scan_id = started.scan.expect("a started scan is described").id;

        (scan_id, group, director)
    }

    /// Calls a command the way the interface does. `Err` is what the
    /// interface would see rejected: a coded refusal of ours, or Tauri's own
    /// text when it could not find the command or read an argument.
    fn invoke(&self, command: &str, payload: Value) -> Result<Value, Value> {
        let origin = if cfg!(windows) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        };
        let request = InvokeRequest {
            cmd: command.to_owned(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: origin.parse().expect("the origin is a url"),
            body: InvokeBody::Json(payload),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };

        tauri::test::get_ipc_response(&self.webview, request).map(|body| {
            body.deserialize::<Value>()
                .expect("a command answers in json")
        })
    }

    /// The state a command answers with, or the refusal as it came.
    fn state_after(&self, command: &str) -> Result<Value, Value> {
        self.invoke(command, json!({}))
            .map(|status| status["state"].clone())
    }

    /// Polls the status until the scan is over, and fails instead of hanging
    /// if it never is.
    fn wait_for_the_end(&self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);

        loop {
            let status = self
                .invoke("scan_status", json!({}))
                .expect("the status is always given");
            if ["completed", "cancelled", "failed"]
                .contains(&status["state"].as_str().unwrap_or(""))
            {
                return status;
            }

            assert!(Instant::now() < deadline, "the scan never ended: {status}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn contract(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../contract")
        .join(name);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));

    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

#[test]
fn every_command_in_the_contract_is_answered() {
    let window = window();
    let examples = contract("commands.json");
    let examples = examples.as_object().expect("the contract is an object");
    assert_eq!(examples.len(), 9);

    for (name, example) in examples {
        let command = example["command"].as_str().expect("a command has a name");
        let answer = window.invoke(command, example["payload"].clone());

        // A refusal of ours carries a code. Plain text means Tauri itself
        // turned the call down: no such command, or an argument it could not
        // read under the name the interface used.
        if let Err(refusal) = answer {
            assert!(refusal.get("code").is_some(), "{name}: {refusal}");
        }
    }
}

#[test]
fn a_scan_runs_from_start_to_groups_through_the_commands() {
    let tree = tempfile::tempdir().expect("a temporary folder");
    for name in ["one.bin", "two.bin"] {
        std::fs::write(tree.path().join(name), b"the same bytes twice")
            .expect("a file in the temporary folder");
    }
    let window = window();

    let started = window
        .invoke(
            "scan_start",
            json!({ "roots": [tree.path()], "includeHidden": false }),
        )
        .expect("the scan starts");
    let scan_id = started["scan"]["id"].clone();
    assert_eq!(scan_id, json!(1));

    let finished = window.wait_for_the_end();
    assert_eq!(finished["state"], "completed");
    assert_eq!(finished["scan"]["groups"], 1);

    let by_savings = json!({ "offset": 0, "limit": 10, "sort": "savings" });
    let groups = window
        .invoke(
            "scan_groups",
            json!({ "scanId": scan_id, "query": by_savings }),
        )
        .expect("the groups of the current scan are given");
    assert_eq!(groups["total"], 1);
    let names: Vec<&str> = groups["groups"][0]["files"]
        .as_array()
        .expect("a group lists its files")
        .iter()
        .filter_map(|file| file["name"].as_str())
        .collect();
    assert_eq!(names, ["one.bin", "two.bin"]);

    let skipped = window
        .invoke(
            "scan_skipped",
            json!({ "scanId": scan_id, "query": { "offset": 0, "limit": 10 } }),
        )
        .expect("the skipped entries of the current scan are given");
    assert_eq!(skipped["total"], 0);
}

#[test]
fn a_refused_start_reaches_the_window_as_a_code() {
    let window = window();

    let refused = window.invoke("scan_start", json!({ "roots": [], "includeHidden": false }));

    assert_eq!(refused, Err(json!({ "code": "noRoots" })));
}

#[test]
fn a_stale_query_reaches_the_window_as_a_code() {
    let window = window();
    let query = json!({ "offset": 0, "limit": 10, "sort": "found" });

    let refused = window.invoke("scan_groups", json!({ "scanId": 99, "query": query }));

    assert_eq!(refused, Err(json!({ "code": "staleScan" })));
}

#[test]
fn controls_answer_with_the_status() {
    let window = window();
    let idle = json!({ "state": "idle", "scan": null });

    for command in ["scan_pause", "scan_resume", "scan_cancel", "scan_status"] {
        assert_eq!(
            window.invoke(command, json!({})),
            Ok(idle.clone()),
            "{command}"
        );
    }
}

#[test]
fn hidden_files_are_scanned_only_when_asked_for() {
    let tree = tempfile::tempdir().expect("a temporary folder");
    // A name with a leading dot is hidden on Unix; Windows hides by an
    // attribute, which the scanner's own Windows tests set.
    for name in [".one.bin", ".two.bin"] {
        std::fs::write(tree.path().join(name), b"the same bytes twice")
            .expect("a file in the temporary folder");
    }
    let groups_found = |include_hidden: bool| {
        let window = window();
        window
            .invoke(
                "scan_start",
                json!({ "roots": [tree.path()], "includeHidden": include_hidden }),
            )
            .expect("the scan starts");

        window.wait_for_the_end()["scan"]["groups"].clone()
    };

    assert_eq!(groups_found(true), json!(1));
    if cfg!(unix) {
        assert_eq!(groups_found(false), json!(0));
    }
}

// The bug this guards: with only an idle session to call, a pause button
// wired to resume passed every test. A real scan of a temporary folder is
// over before it can be paused, so this one is scripted and stands still.
#[test]
fn pause_resume_and_cancel_reach_the_running_scan() {
    let window = window();
    let session = window.app.state::<Session>();
    let (_, director) =
        testkit::start_scripted(&session, vec![testkit::root()]).expect("a scripted scan starts");

    assert_eq!(window.state_after("scan_status"), Ok(json!("scanning")));
    assert_eq!(window.state_after("scan_pause"), Ok(json!("paused")));
    assert_eq!(window.state_after("scan_resume"), Ok(json!("scanning")));
    assert!(!director.is_cancelled());

    assert_eq!(window.state_after("scan_cancel"), Ok(json!("stopping")));
    assert!(director.is_cancelled());
}

#[test]
fn suggestions_keep_the_folders_that_exist_in_the_order_given() {
    let home = tempfile::tempdir().expect("a temporary folder");
    let downloads = home.path().join("Downloads");
    let pictures = home.path().join("Pictures");
    std::fs::create_dir(&downloads).expect("a folder in the temporary folder");
    std::fs::create_dir(&pictures).expect("a folder in the temporary folder");

    let offered = suggestions([
        (RootKind::Downloads, Some(downloads.clone())),
        // The system names a folder that is not there.
        (RootKind::Desktop, Some(home.path().join("Desktop"))),
        // The system cannot name the folder at all.
        (RootKind::Documents, None),
        (RootKind::Pictures, Some(pictures.clone())),
    ]);

    let expected = [
        (RootKind::Downloads, downloads),
        (RootKind::Pictures, pictures),
    ]
    .map(|(kind, path)| SuggestedRoot {
        kind,
        path: path.to_string_lossy().into_owned(),
    });
    assert_eq!(offered, expected);
}

#[test]
fn a_file_is_not_offered_as_a_folder() {
    let home = tempfile::tempdir().expect("a temporary folder");
    let folder = home.path().join("Pictures");
    let file = home.path().join("Downloads");
    std::fs::create_dir(&folder).expect("a folder in the temporary folder");
    std::fs::write(&file, b"not a folder").expect("a file in the temporary folder");

    let offered = suggestions([
        (RootKind::Downloads, Some(file)),
        (RootKind::Pictures, Some(folder)),
    ]);

    let kinds: Vec<RootKind> = offered.iter().map(|offer| offer.kind).collect();
    assert_eq!(kinds, [RootKind::Pictures]);
}

#[test]
fn suggestions_have_the_shape_and_the_names_the_contract_lists() {
    let example = contract("suggested-roots.json");
    let kinds = [
        RootKind::Downloads,
        RootKind::Desktop,
        RootKind::Documents,
        RootKind::Pictures,
    ];
    let offered: Vec<SuggestedRoot> = kinds
        .iter()
        .zip(["Downloads", "Desktop", "Documents", "Pictures"])
        .map(|(&kind, folder)| SuggestedRoot {
            kind,
            path: format!("/home/me/{folder}"),
        })
        .collect();

    assert_eq!(serde_json::to_value(&offered).expect("json"), example);
    assert_eq!(
        serde_json::to_value(kinds).expect("json"),
        contract("names.json")["rootKinds"]
    );
}

#[test]
fn suggested_roots_are_answered_through_the_ipc() {
    let window = window();
    let known = contract("names.json")["rootKinds"].clone();

    let offered = window
        .invoke("suggested_roots", json!({}))
        .expect("suggestions are always given");

    // Which folders this machine has is not the test's business; that each
    // offer is well formed is.
    for offer in offered.as_array().expect("a list of offers") {
        let kinds = known.as_array().expect("a list of names");
        assert!(kinds.contains(&offer["kind"]), "{offer}");
        assert!(offer["path"].is_string(), "{offer}");
    }
}

#[test]
fn a_file_named_by_its_ids_is_revealed_at_its_exact_path() {
    let window = window();
    let (scan_id, group, _director) = window.scan_with_one_group();

    let answer = window.invoke(
        "reveal_file",
        json!({ "scanId": scan_id, "groupId": 0, "fileId": 1 }),
    );

    assert_eq!(answer, Ok(Value::Null));
    assert_eq!(window.revealed(), group.files[1].paths[..1]);
}

#[test]
fn a_reveal_of_an_earlier_scan_or_a_missing_file_is_refused_and_reveals_nothing() {
    let window = window();
    let (scan_id, _, _director) = window.scan_with_one_group();
    let reveal = |scan_id: u64, group_id: u64, file_id: u64| {
        window.invoke(
            "reveal_file",
            json!({ "scanId": scan_id, "groupId": group_id, "fileId": file_id }),
        )
    };

    assert_eq!(
        reveal(scan_id + 1, 0, 0),
        Err(json!({ "code": "staleScan" }))
    );
    assert_eq!(reveal(scan_id, 1, 0), Err(json!({ "code": "noSuchFile" })));
    assert_eq!(reveal(scan_id, 0, 2), Err(json!({ "code": "noSuchFile" })));
    assert_eq!(window.revealed(), Vec::<PathBuf>::new());
}

#[test]
fn a_reveal_the_system_could_not_do_reaches_the_window_as_a_code() {
    let window = window_revealing(Err("the file manager did not answer".to_owned()));
    let (scan_id, _, _director) = window.scan_with_one_group();

    let answer = window.invoke(
        "reveal_file",
        json!({ "scanId": scan_id, "groupId": 0, "fileId": 0 }),
    );

    assert_eq!(
        answer,
        Err(json!({ "code": "revealFailed", "message": "the file manager did not answer" }))
    );
}

#[test]
fn shell_refusals_have_the_shape_the_contract_lists() {
    let example = contract("shell-errors.json");
    let refusals = [
        ("staleScan", ShellError::StaleScan),
        ("noSuchFile", ShellError::NoSuchFile),
        (
            "revealFailed",
            ShellError::RevealFailed {
                message: "the file manager did not answer".to_owned(),
            },
        ),
    ];

    assert_eq!(
        example.as_object().map(|listed| listed.len()),
        Some(refusals.len())
    );
    for (name, refusal) in refusals {
        assert_eq!(
            serde_json::to_value(&refusal).expect("json"),
            example[name],
            "{name}"
        );
    }
}
