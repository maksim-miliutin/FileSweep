use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::Path;

use filesweep_core::{DuplicateGroup, Skip};

use crate::view::{
    GroupPage, GroupQuery, GroupSort, GroupView, MAX_PAGE, PageQuery, SkipCode, SkipPage, SkipView,
};

/// Groups and skipped entries in the order the scanner reported them. Both
/// lists only grow, so a place in either stays valid for the whole scan.
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
}

impl Results {
    pub(crate) fn take_group(&mut self, group: DuplicateGroup) {
        self.reclaimable_bytes = self
            .reclaimable_bytes
            .saturating_add(group.reclaimable_bytes);
        self.groups.push(group);
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
}

/// The rows of a list of `len` that a query asks for: never past the end of
/// the list and never more than `MAX_PAGE` of them.
fn window(offset: u64, limit: u32, len: usize) -> Range<usize> {
    let start = usize::try_from(offset).map_or(len, |offset| offset.min(len));
    let rows = limit.min(MAX_PAGE) as usize;

    start..start.saturating_add(rows).min(len)
}
