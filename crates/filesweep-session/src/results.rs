use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;
use std::time::SystemTime;

use filesweep_core::{DuplicateGroup, FileTimes, IndexedFile, Skip};

use crate::view::{
    GroupPage, GroupQuery, GroupSort, GroupView, HistoryQuery, MAX_EDGES, MAX_PAGE, PageQuery,
    SkipCode, SkipPage, SkipView, UsageFileView, UsageHistory, UsagePage, UsageQuery, UsageSpan,
    unix_millis,
};

/// Groups, skipped entries and indexed files in the order the scanner
/// reported them. The lists only grow, so a place in any of them stays valid
/// for the whole scan.
#[derive(Default)]
pub(crate) struct Results {
    groups: Vec<DuplicateGroup>,
    skipped: Vec<Skip>,
    skipped_by_reason: BTreeMap<SkipCode, u64>,
    reclaimable_bytes: u64,
    /// Places in `groups`, the most reclaimable first. Built when a page in
    /// that order is asked for, and out of date whenever it is shorter than
    /// `groups`: groups are only ever added.
    by_savings: Vec<usize>,
    files: Vec<Kept>,
    /// Places in `files`, the largest first, kept the way `by_savings` is.
    by_size: Vec<usize>,
}

/// What is kept of an indexed file. A scan may index millions of them, so
/// the times take eight bytes each and the path has no room to grow.
struct Kept {
    path: Box<Path>,
    size: u64,
    modified: Millis,
    accessed: Millis,
    created: Millis,
    /// The latest of the three.
    last_used: Millis,
}

/// Milliseconds since 1970, or no time at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Millis(i64);

impl Millis {
    /// The scanner gives no time at or before 1970, so the one value a real
    /// time never takes stands for none.
    const NONE: Self = Self(i64::MIN);

    fn of(time: Option<SystemTime>) -> Self {
        time.map_or(Self::NONE, |time| Self(unix_millis(time)))
    }

    fn get(self) -> Option<i64> {
        (self != Self::NONE).then_some(self.0)
    }
}

impl Results {
    pub(crate) fn take_group(&mut self, group: DuplicateGroup) {
        self.reclaimable_bytes = self
            .reclaimable_bytes
            .saturating_add(group.reclaimable_bytes);
        self.groups.push(group);
    }

    pub(crate) fn take_file(&mut self, file: IndexedFile) {
        let FileTimes {
            modified,
            accessed,
            created,
        } = file.times;
        let latest = [modified, accessed, created].into_iter().flatten().max();

        self.files.push(Kept {
            path: file.path.into_boxed_path(),
            size: file.size,
            modified: Millis::of(modified),
            accessed: Millis::of(accessed),
            created: Millis::of(created),
            last_used: Millis::of(latest),
        });
    }

    pub(crate) fn take_skip(&mut self, skip: Skip) {
        *self
            .skipped_by_reason
            .entry(SkipCode::of(&skip.reason))
            .or_default() += 1;
        self.skipped.push(skip);
    }

    pub(crate) fn group_count(&self) -> u64 {
        self.groups.len() as u64
    }

    pub(crate) fn skip_count(&self) -> u64 {
        self.skipped.len() as u64
    }

    pub(crate) fn reclaimable_bytes(&self) -> u64 {
        self.reclaimable_bytes
    }

    pub(crate) fn skipped_by_reason(&self) -> &BTreeMap<SkipCode, u64> {
        &self.skipped_by_reason
    }

    pub(crate) fn group_page(&mut self, query: &GroupQuery) -> GroupPage {
        let rows = window(query.offset, query.limit, self.groups.len());
        let groups = match query.sort {
            GroupSort::Found => rows.map(|place| self.group_view(place)).collect(),
            GroupSort::Savings => {
                self.order_by_savings();
                self.by_savings[rows]
                    .iter()
                    .map(|&place| self.group_view(place))
                    .collect()
            }
        };

        GroupPage {
            total: self.group_count(),
            offset: query.offset,
            groups,
        }
    }

    pub(crate) fn skip_page(&self, query: &PageQuery) -> SkipPage {
        let rows = window(query.offset, query.limit, self.skipped.len());

        SkipPage {
            total: self.skip_count(),
            offset: query.offset,
            items: self.skipped[rows].iter().map(SkipView::new).collect(),
        }
    }

    pub(crate) fn history(&self, query: &HistoryQuery) -> UsageHistory {
        let mut edges = query.edges_ms.clone();
        edges.sort_unstable();
        edges.dedup();
        edges.truncate(MAX_EDGES);

        let mut spans = vec![UsageSpan::default(); edges.len() + 1];
        let mut undated = UsageSpan::default();
        for kept in &self.files {
            let span = match kept.last_used.get() {
                // As many edges lie at or before the time as spans before
                // the one it falls in.
                Some(time) => &mut spans[edges.partition_point(|&edge| edge <= time)],
                None => &mut undated,
            };
            span.files += 1;
            span.bytes = span.bytes.saturating_add(kept.size);
        }

        UsageHistory {
            edges_ms: edges,
            spans,
            undated,
        }
    }

    pub(crate) fn usage_page(&mut self, query: &UsageQuery) -> UsagePage {
        self.order_by_size();

        let in_span = |time: i64| {
            query.from_ms.is_none_or(|from| time >= from)
                && query.before_ms.is_none_or(|before| time < before)
        };
        let rows = query.limit.min(MAX_PAGE) as usize;
        let mut total = 0;
        let mut bytes: u64 = 0;
        let mut files = Vec::new();

        for &place in &self.by_size {
            let kept = &self.files[place];
            let Some(time) = kept.last_used.get().filter(|&time| in_span(time)) else {
                continue;
            };

            if total >= query.offset && files.len() < rows {
                files.push(UsageFileView {
                    modified_ms: kept.modified.get(),
                    accessed_ms: kept.accessed.get(),
                    created_ms: kept.created.get(),
                    ..UsageFileView::new(place as u64, &kept.path, kept.size, time)
                });
            }
            total += 1;
            bytes = bytes.saturating_add(kept.size);
        }

        UsagePage {
            total,
            bytes,
            offset: query.offset,
            files,
        }
    }

    pub(crate) fn indexed_path(&self, file: u64) -> Option<&Path> {
        let kept = self.files.get(usize::try_from(file).ok()?)?;

        Some(&kept.path)
    }

    pub(crate) fn file_path(&self, group: u64, file: u64) -> Option<&Path> {
        let group = self.groups.get(usize::try_from(group).ok()?)?;
        let file = group.files.get(usize::try_from(file).ok()?)?;

        file.paths.first().map(|path| path.as_path())
    }

    fn group_view(&self, place: usize) -> GroupView {
        GroupView::new(place as u64, &self.groups[place])
    }

    fn order_by_savings(&mut self) {
        if self.by_savings.len() == self.groups.len() {
            return;
        }

        let groups = &self.groups;
        let mut order: Vec<usize> = (0..groups.len()).collect();
        // The sort is stable, so groups that promise the same amount stay in
        // the order they were found.
        order.sort_by_key(|&place| Reverse(groups[place].reclaimable_bytes));
        self.by_savings = order;
    }

    fn order_by_size(&mut self) {
        if self.by_size.len() == self.files.len() {
            return;
        }

        let files = &self.files;
        let mut order: Vec<usize> = (0..files.len()).collect();
        // Stable: files of one size stay in the order the walk met them.
        order.sort_by_key(|&place| Reverse(files[place].size));
        self.by_size = order;
    }
}

/// The rows of a list of `len` that a query asks for: never past the end of
/// the list and never more than `MAX_PAGE` of them.
fn window(offset: u64, limit: u32, len: usize) -> Range<usize> {
    let start = usize::try_from(offset).map_or(len, |offset| offset.min(len));
    let rows = limit.min(MAX_PAGE) as usize;

    start..start.saturating_add(rows).min(len)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{Kept, Millis};

    #[test]
    fn a_kept_file_is_a_path_and_five_numbers() {
        assert!(size_of::<Kept>() <= 56, "{} bytes", size_of::<Kept>());
    }

    #[test]
    fn a_time_keeps_its_milliseconds_and_no_time_stays_none() {
        let time = UNIX_EPOCH + Duration::from_millis(1_500);

        assert_eq!(Millis::of(Some(time)).get(), Some(1_500));
        assert_eq!(Millis::of(None).get(), None);
    }
}
