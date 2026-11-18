use std::collections::BTreeMap;

use filesweep_core::{DuplicateGroup, Skip};

use crate::view::SkipCode;

/// Groups and skipped entries in the order the scanner reported them. Both
/// lists only grow, so a place in either stays valid for the whole scan.
#[derive(Default)]
pub(crate) struct Results {
    groups: Vec<DuplicateGroup>,
    skipped: Vec<Skip>,
    skipped_by_reason: BTreeMap<SkipCode, u64>,
    reclaimable_bytes: u64,
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
}
