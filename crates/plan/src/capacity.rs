//! Capacity check before a backup is made (skill `migration-planning`): does the data fit on the
//! target, on the intermediary quota and on the disk that stages the backup? The answer is advice
//! with numbers, not a raw error, and a staging disk that would not fit is refused with the
//! suggestion to stream instead.

use crate::spec::BackupLocation;

/// Free space kept beyond the data itself, as a fraction (one tenth): file systems and databases
/// need room to work, and sizes are measured before the final delta.
const HEADROOM_DIVISOR: u64 = 10;

/// Measurements, all in bytes. `None` means "not measured": it is never read as "plenty".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityInput {
    /// Where the backup is kept.
    pub location: BackupLocation,
    /// Size of the data to move, after exclusions.
    pub data_bytes: Option<u64>,
    /// Free space on the source disk (matters when the backup is kept on the source).
    pub source_free_bytes: Option<u64>,
    /// Free space on the person's machine (matters when the backup is kept locally).
    pub local_free_bytes: Option<u64>,
    /// Quota left on the intermediary storage (matters when the backup is kept there).
    pub intermediary_quota_bytes: Option<u64>,
    /// Free space on the target.
    pub target_free_bytes: Option<u64>,
}

/// What is wrong with a measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityIssueKind {
    /// The size of the data was not measured.
    DataSizeUnknown,
    /// The target has not enough free space.
    TargetTooSmall,
    /// The free space of the target was not measured.
    TargetFreeUnknown,
    /// The disk that would hold the backup (source or local) has not enough free space.
    StagingTooSmall,
    /// The free space of the disk that would hold the backup was not measured.
    StagingFreeUnknown,
    /// The intermediary quota is too small.
    QuotaTooSmall,
    /// The intermediary quota was not measured.
    QuotaUnknown,
}

/// One finding of the check, with the numbers behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityIssue {
    /// What is wrong.
    pub kind: CapacityIssueKind,
    /// Bytes needed, headroom included (absent when the size itself is unknown).
    pub needed: Option<u64>,
    /// Bytes available (absent when not measured).
    pub available: Option<u64>,
}

/// The overall answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityVerdict {
    /// Everything that matters was measured and fits.
    Fits,
    /// Something that matters was not measured; measure it before running.
    Unknown,
    /// Something does not fit.
    Blocked,
}

/// Advice from [`check_capacity`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityAdvice {
    /// Fits, unknown or blocked.
    pub verdict: CapacityVerdict,
    /// Every problem found.
    pub issues: Vec<CapacityIssue>,
    /// The backup would not fit on the disk that stages it: stream it to the transport instead of
    /// writing it to that disk first.
    pub suggest_streaming: bool,
}

fn with_headroom(bytes: u64) -> u64 {
    bytes.saturating_add(bytes / HEADROOM_DIVISOR)
}

/// Checks the measurements. Pure: it measures nothing itself.
#[must_use]
pub fn check_capacity(input: &CapacityInput) -> CapacityAdvice {
    let mut issues = Vec::new();
    let Some(data) = input.data_bytes else {
        issues.push(CapacityIssue {
            kind: CapacityIssueKind::DataSizeUnknown,
            needed: None,
            available: None,
        });
        return CapacityAdvice {
            verdict: CapacityVerdict::Unknown,
            issues,
            suggest_streaming: false,
        };
    };
    let needed = with_headroom(data);

    let mut push = |too_small, unknown, available: Option<u64>| match available {
        None => issues.push(CapacityIssue {
            kind: unknown,
            needed: Some(needed),
            available: None,
        }),
        Some(free) if free < needed => issues.push(CapacityIssue {
            kind: too_small,
            needed: Some(needed),
            available: Some(free),
        }),
        Some(_) => {}
    };

    push(
        CapacityIssueKind::TargetTooSmall,
        CapacityIssueKind::TargetFreeUnknown,
        input.target_free_bytes,
    );
    match input.location {
        BackupLocation::Source => push(
            CapacityIssueKind::StagingTooSmall,
            CapacityIssueKind::StagingFreeUnknown,
            input.source_free_bytes,
        ),
        BackupLocation::Local => push(
            CapacityIssueKind::StagingTooSmall,
            CapacityIssueKind::StagingFreeUnknown,
            input.local_free_bytes,
        ),
        BackupLocation::Intermediary => push(
            CapacityIssueKind::QuotaTooSmall,
            CapacityIssueKind::QuotaUnknown,
            input.intermediary_quota_bytes,
        ),
    }

    let suggest_streaming = issues
        .iter()
        .any(|issue| issue.kind == CapacityIssueKind::StagingTooSmall);
    let blocked = issues.iter().any(|issue| {
        matches!(
            issue.kind,
            CapacityIssueKind::TargetTooSmall
                | CapacityIssueKind::StagingTooSmall
                | CapacityIssueKind::QuotaTooSmall
        )
    });
    let verdict = if blocked {
        CapacityVerdict::Blocked
    } else if issues.is_empty() {
        CapacityVerdict::Fits
    } else {
        CapacityVerdict::Unknown
    };
    CapacityAdvice {
        verdict,
        issues,
        suggest_streaming,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    fn input(location: BackupLocation) -> CapacityInput {
        CapacityInput {
            location,
            data_bytes: Some(10 * GIB),
            source_free_bytes: Some(100 * GIB),
            local_free_bytes: Some(100 * GIB),
            intermediary_quota_bytes: Some(100 * GIB),
            target_free_bytes: Some(100 * GIB),
        }
    }

    fn kinds(advice: &CapacityAdvice) -> Vec<CapacityIssueKind> {
        advice.issues.iter().map(|i| i.kind).collect()
    }

    #[test]
    fn plenty_of_room_fits_for_every_location() {
        for location in [
            BackupLocation::Source,
            BackupLocation::Local,
            BackupLocation::Intermediary,
        ] {
            let advice = check_capacity(&input(location));
            assert_eq!(advice.verdict, CapacityVerdict::Fits);
            assert!(advice.issues.is_empty() && !advice.suggest_streaming);
        }
    }

    #[test]
    fn exactly_the_data_size_does_not_fit_because_headroom_is_required() {
        let mut i = input(BackupLocation::Local);
        i.target_free_bytes = Some(10 * GIB);
        let advice = check_capacity(&i);
        assert_eq!(advice.verdict, CapacityVerdict::Blocked);
        assert_eq!(kinds(&advice), [CapacityIssueKind::TargetTooSmall]);
        // The boundary: exactly data + 10% fits, one byte less does not.
        let need = with_headroom(10 * GIB);
        i.target_free_bytes = Some(need);
        assert_eq!(check_capacity(&i).verdict, CapacityVerdict::Fits);
        i.target_free_bytes = Some(need - 1);
        assert_eq!(check_capacity(&i).verdict, CapacityVerdict::Blocked);
    }

    #[test]
    fn local_staging_that_does_not_fit_is_refused_with_a_streaming_suggestion() {
        let mut i = input(BackupLocation::Local);
        i.local_free_bytes = Some(5 * GIB);
        let advice = check_capacity(&i);
        assert_eq!(advice.verdict, CapacityVerdict::Blocked);
        assert!(advice.suggest_streaming);
        assert_eq!(kinds(&advice), [CapacityIssueKind::StagingTooSmall]);
    }

    #[test]
    fn unmeasured_free_space_is_never_read_as_plenty() {
        let mut i = input(BackupLocation::Local);
        i.local_free_bytes = None;
        let advice = check_capacity(&i);
        assert_eq!(advice.verdict, CapacityVerdict::Unknown);
        assert_eq!(kinds(&advice), [CapacityIssueKind::StagingFreeUnknown]);
        assert!(!advice.suggest_streaming);

        let mut j = input(BackupLocation::Source);
        j.target_free_bytes = None;
        assert_eq!(check_capacity(&j).verdict, CapacityVerdict::Unknown);
    }

    #[test]
    fn unknown_data_size_stops_the_check() {
        let mut i = input(BackupLocation::Source);
        i.data_bytes = None;
        let advice = check_capacity(&i);
        assert_eq!(advice.verdict, CapacityVerdict::Unknown);
        assert_eq!(kinds(&advice), [CapacityIssueKind::DataSizeUnknown]);
    }

    #[test]
    fn quota_only_matters_for_an_intermediary() {
        let mut i = input(BackupLocation::Intermediary);
        i.intermediary_quota_bytes = Some(GIB);
        assert_eq!(
            kinds(&check_capacity(&i)),
            [CapacityIssueKind::QuotaTooSmall]
        );
        let mut local = input(BackupLocation::Local);
        local.intermediary_quota_bytes = Some(0);
        assert_eq!(check_capacity(&local).verdict, CapacityVerdict::Fits);
    }

    #[test]
    fn blocked_beats_unknown_and_all_issues_are_listed() {
        let mut i = input(BackupLocation::Source);
        i.target_free_bytes = Some(GIB);
        i.source_free_bytes = None;
        let advice = check_capacity(&i);
        assert_eq!(advice.verdict, CapacityVerdict::Blocked);
        assert_eq!(advice.issues.len(), 2);
    }

    #[test]
    fn zero_data_fits_and_huge_values_do_not_overflow() {
        let mut i = input(BackupLocation::Local);
        i.data_bytes = Some(0);
        i.target_free_bytes = Some(0);
        i.local_free_bytes = Some(0);
        assert_eq!(check_capacity(&i).verdict, CapacityVerdict::Fits);
        i.data_bytes = Some(u64::MAX);
        i.target_free_bytes = Some(u64::MAX - 1);
        assert_eq!(check_capacity(&i).verdict, CapacityVerdict::Blocked);
    }
}
