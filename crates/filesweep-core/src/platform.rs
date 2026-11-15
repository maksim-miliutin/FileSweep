//! Everything here compiles and is tested on every platform, but a platform
//! calls only its own part, so some of it is unused wherever it is built.
#![allow(dead_code)]

use std::io;

use crate::types::{FileIdentity, IoFailure, SkipReason};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EntryFlags {
    pub hidden: bool,
    pub system: bool,
    pub online_only: bool,
}

const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x4_0000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

// Any of the three means the content lives somewhere else. OFFLINE is the old
// remote-storage bit; the RECALL bits are what cloud providers such as
// OneDrive put on files they have not downloaded.
const ONLINE_ONLY: u32 =
    FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;

pub(crate) fn flags_from_windows_attributes(attributes: u32) -> EntryFlags {
    EntryFlags {
        hidden: attributes & FILE_ATTRIBUTE_HIDDEN != 0,
        system: attributes & FILE_ATTRIBUTE_SYSTEM != 0,
        online_only: attributes & ONLINE_ONLY != 0,
    }
}

const ERROR_SHARING_VIOLATION: i32 = 32;
const ERROR_LOCK_VIOLATION: i32 = 33;

/// Windows error codes that std files under a kind too broad to act on.
pub(crate) fn windows_error_reason(code: i32) -> Option<SkipReason> {
    match code {
        ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION => Some(SkipReason::InUse),
        _ => None,
    }
}

pub(crate) fn skip_reason(err: &io::Error) -> SkipReason {
    #[cfg(windows)]
    {
        if let Some(reason) = err.raw_os_error().and_then(windows_error_reason) {
            return reason;
        }
    }

    match err.kind() {
        io::ErrorKind::PermissionDenied => SkipReason::AccessDenied,
        io::ErrorKind::NotFound => SkipReason::Vanished,
        _ => SkipReason::Io(IoFailure::from_error(err)),
    }
}

/// A file index of zero is how a file system says it has none.
pub(crate) fn identity_from_raw(volume: u64, file: u128) -> Option<FileIdentity> {
    if file == 0 {
        return None;
    }

    Some(FileIdentity { volume, file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_attribute_bits_map_to_flags() {
        let none = EntryFlags::default();
        let online_only = EntryFlags {
            online_only: true,
            ..none
        };
        let cases = [
            (0x0, none),
            // ARCHIVE is set on almost every file and means nothing here.
            (0x20, none),
            (
                0x2,
                EntryFlags {
                    hidden: true,
                    ..none
                },
            ),
            (
                0x4,
                EntryFlags {
                    system: true,
                    ..none
                },
            ),
            (
                0x6,
                EntryFlags {
                    hidden: true,
                    system: true,
                    ..none
                },
            ),
            (0x1000, online_only),
            (0x4_0000, online_only),
            (0x40_0000, online_only),
        ];

        for (attributes, expected) in cases {
            assert_eq!(
                flags_from_windows_attributes(attributes),
                expected,
                "attributes {attributes:#x}"
            );
        }
    }

    #[test]
    fn codes_32_and_33_mean_in_use() {
        assert_eq!(windows_error_reason(32), Some(SkipReason::InUse));
        assert_eq!(windows_error_reason(33), Some(SkipReason::InUse));
        assert_eq!(windows_error_reason(5), None);
    }

    #[test]
    fn unknown_file_identity_is_none() {
        assert_eq!(identity_from_raw(7, 0), None);
        assert_eq!(
            identity_from_raw(7, 1),
            Some(FileIdentity { volume: 7, file: 1 })
        );
    }

    #[test]
    fn skip_reason_maps_denied_and_vanished() {
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(skip_reason(&denied), SkipReason::AccessDenied);

        let gone = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(skip_reason(&gone), SkipReason::Vanished);

        let other = io::Error::other("the disk is on fire");
        assert_eq!(
            skip_reason(&other),
            SkipReason::Io(IoFailure::from_error(&other))
        );
    }

    #[cfg(windows)]
    #[test]
    fn skip_reason_reads_a_sharing_violation_as_in_use() {
        let locked = io::Error::from_raw_os_error(32);
        assert_eq!(skip_reason(&locked), SkipReason::InUse);
    }
}
