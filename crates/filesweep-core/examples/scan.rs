//! Scans the folders given on the command line and prints what the scanner
//! reports. A developer's tool for trying the library on a real folder: it
//! only reads, like the library itself.
//!
//!     cargo run --release -p filesweep-core --example scan -- <folder>...
//!
//! Groups, skipped entries and the summary go to standard output, progress to
//! standard error.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use filesweep_core::{
    DuplicateGroup, Independence, Outcome, Progress, ScanControl, ScanEvent, ScanPolicy,
    ScanRequest, ScanSummary, SkipReason, scan,
};

fn main() -> ExitCode {
    let roots: Vec<PathBuf> = env::args_os().skip(1).map(PathBuf::from).collect();
    if roots.is_empty() {
        eprintln!("usage: scan <folder>...");
        return ExitCode::from(2);
    }

    let request = ScanRequest {
        roots,
        policy: ScanPolicy::default(),
    };
    let mut progress = ProgressLine::default();

    let result = scan(&request, &ScanControl::new(), &mut |event| match event {
        ScanEvent::Progress(step) => progress.show(&step),
        ScanEvent::Skipped(skip) => {
            println!(
                "skipped   {:<28} {}",
                describe(&skip.reason),
                skip.path.display()
            );
        }
        ScanEvent::Group(group) => print_group(&group),
        // The example prints copies only, not every file it walks past.
        ScanEvent::Indexed(_) => {}
    });

    match result {
        Ok(summary) => {
            print_summary(&summary);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("scan: {error}");
            ExitCode::from(2)
        }
    }
}

fn describe(reason: &SkipReason) -> String {
    match reason {
        SkipReason::Link => "link, not followed".to_string(),
        SkipReason::OnlineOnly => "online-only, not downloaded".to_string(),
        SkipReason::System => "system".to_string(),
        SkipReason::Hidden => "hidden".to_string(),
        SkipReason::Special => "not a regular file".to_string(),
        SkipReason::NotADirectory => "not a folder".to_string(),
        SkipReason::AlreadyCovered => "already scanned".to_string(),
        SkipReason::AccessDenied => "access denied".to_string(),
        SkipReason::InUse => "in use by another program".to_string(),
        SkipReason::Vanished => "gone".to_string(),
        SkipReason::Changed => "changed during the scan".to_string(),
        SkipReason::Io(failure) => format!("error: {}", failure.message),
    }
}

fn print_group(group: &DuplicateGroup) {
    let hash = group.hash.to_string();

    println!();
    println!(
        "group     {} files of {} each, hash {}",
        group.files.len(),
        human(group.size),
        &hash[..12]
    );
    match group.independence {
        Independence::Verified => println!(
            "          removing the extra copies frees at least {}",
            human(group.reclaimable_bytes)
        ),
        Independence::Unverified => {
            println!("          no file identity here: these may be one file under several names")
        }
    }

    for file in &group.files {
        let mut names = file.paths.iter();
        if let Some(first) = names.next() {
            println!("    {}", first.display());
        }
        for other in names {
            println!(
                "      = {}  (another name of the same file)",
                other.display()
            );
        }
    }
}

fn print_summary(summary: &ScanSummary) {
    let outcome = match summary.outcome {
        Outcome::Completed => "completed",
        Outcome::Cancelled => "cancelled",
    };

    println!();
    println!("scan {outcome}");
    println!("  files seen     {}", summary.files_seen);
    println!("  empty files    {}", summary.empty_files);
    println!("  candidates     {}", summary.candidates);
    println!("  files hashed   {}", summary.files_hashed);
    println!("  bytes hashed   {}", human(summary.bytes_hashed));
    println!("  groups         {}", summary.groups);
    println!("  reclaimable    {}", human(summary.reclaimable_bytes));
    println!("  skipped        {}", summary.skipped);
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Shows progress on standard error, at most twice a second: the scanner
/// reports after every block it reads.
#[derive(Default)]
struct ProgressLine {
    last: Option<Instant>,
}

impl ProgressLine {
    fn show(&mut self, step: &Progress) {
        let now = Instant::now();
        if self
            .last
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(500))
        {
            return;
        }
        self.last = Some(now);

        match step {
            Progress::Walking { files_seen } => eprintln!("  walking: {files_seen} files"),
            Progress::Hashing {
                files_done,
                files_total,
                bytes_done,
                bytes_total,
            } => eprintln!(
                "  hashing: {files_done} of {files_total} files, {} of {}",
                human(*bytes_done),
                human(*bytes_total)
            ),
        }
    }
}
