use serde::{Deserialize, Serialize};

const MIN_INTERVAL_SECS: u64 = 10;
const MAX_INTERVAL_SECS: u64 = 86_400;
const MAX_THRESHOLD: u32 = 20;
const MAX_ID_LEN: usize = 64;
const MAX_URL_LEN: usize = 2048;
const MAX_HOST_LEN: usize = 253;
const MAX_KEYWORD_LEN: usize = 256;
const MAX_WINDOWS: usize = 64;
const MAX_WARN_DAYS: u32 = 365;

/// A maintenance window, half-open `[start, end)` in unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaintenanceWindow {
    pub start: u64,
    pub end: u64,
}

/// What to check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MonitorKind {
    /// `expect_status` is an inclusive range; `None` means 200..=299.
    Http {
        url: String,
        expect_status: Option<(u16, u16)>,
        keyword: Option<String>,
    },
    Tcp {
        host: String,
        port: u16,
    },
    /// Alert when the certificate expires within `warn_days`.
    TlsExpiry {
        host: String,
        port: u16,
        warn_days: u32,
    },
}

impl MonitorKind {
    /// Whether an HTTP status code satisfies the expectation (always false for non-HTTP kinds).
    #[must_use]
    pub fn status_accepts(&self, code: u16) -> bool {
        match self {
            Self::Http { expect_status, .. } => {
                let (lo, hi) = expect_status.unwrap_or((200, 299));
                (lo..=hi).contains(&code)
            }
            _ => false,
        }
    }
}

/// Closed set of validation failures (no free text; map to i18n keys at the edge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    IdEmpty,
    IdTooLong,
    IntervalOutOfRange,
    TimeoutInvalid,
    FailThresholdOutOfRange,
    RecoverThresholdOutOfRange,
    UrlEmpty,
    UrlTooLong,
    UrlSchemeInvalid,
    UrlHasControlChars,
    HostEmpty,
    HostTooLong,
    HostInvalid,
    PortZero,
    KeywordEmpty,
    KeywordTooLong,
    StatusRangeInvalid,
    WarnDaysOutOfRange,
    MaintenanceInvalid,
    TooManyMaintenanceWindows,
}

/// One monitor. Construct with [`MonitorConfig::new`] (defaults) and adjust fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorConfig {
    pub id: String,
    pub kind: MonitorKind,
    pub interval_secs: u64,
    pub timeout_secs: u64,
    /// Consecutive failures before an incident opens (1..=20, default 3).
    #[serde(default = "default_fail")]
    pub fail_threshold: u32,
    /// Consecutive successes before an incident closes (1..=20, default 1).
    #[serde(default = "default_recover")]
    pub recover_threshold: u32,
    #[serde(default)]
    pub maintenance: Vec<MaintenanceWindow>,
    /// Interval between repeated "still down" transitions; 0 = never repeat (default 3600).
    #[serde(default = "default_repeat")]
    pub repeat_every_secs: u64,
    /// Flapping guard: an `Opened` transition is not emitted again sooner than this after the previous
    /// one (default 300; 0 disables). The incident is still tracked and accounted; its `Opened` is
    /// delivered later if it is still down once the guard expires.
    #[serde(default = "default_reopen")]
    pub min_reopen_secs: u64,
}

fn default_fail() -> u32 {
    3
}
fn default_recover() -> u32 {
    1
}
fn default_repeat() -> u64 {
    3600
}
fn default_reopen() -> u64 {
    300
}

impl MonitorConfig {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        kind: MonitorKind,
        interval_secs: u64,
        timeout_secs: u64,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            interval_secs,
            timeout_secs,
            fail_threshold: default_fail(),
            recover_threshold: default_recover(),
            maintenance: Vec::new(),
            repeat_every_secs: default_repeat(),
            min_reopen_secs: default_reopen(),
        }
    }

    /// Whether `now` falls inside any maintenance window.
    #[must_use]
    pub fn in_maintenance(&self, now: u64) -> bool {
        self.maintenance
            .iter()
            .any(|w| now >= w.start && now < w.end)
    }

    /// Check all bounds.
    ///
    /// # Errors
    /// Returns the first violated bound as a [`ConfigError`].
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.id.is_empty() {
            return Err(ConfigError::IdEmpty);
        }
        if self.id.len() > MAX_ID_LEN {
            return Err(ConfigError::IdTooLong);
        }
        if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&self.interval_secs) {
            return Err(ConfigError::IntervalOutOfRange);
        }
        if self.timeout_secs == 0 || self.timeout_secs > self.interval_secs {
            return Err(ConfigError::TimeoutInvalid);
        }
        if !(1..=MAX_THRESHOLD).contains(&self.fail_threshold) {
            return Err(ConfigError::FailThresholdOutOfRange);
        }
        if !(1..=MAX_THRESHOLD).contains(&self.recover_threshold) {
            return Err(ConfigError::RecoverThresholdOutOfRange);
        }
        if self.maintenance.len() > MAX_WINDOWS {
            return Err(ConfigError::TooManyMaintenanceWindows);
        }
        if self.maintenance.iter().any(|w| w.end <= w.start) {
            return Err(ConfigError::MaintenanceInvalid);
        }
        match &self.kind {
            MonitorKind::Http {
                url,
                expect_status,
                keyword,
            } => {
                if url.is_empty() {
                    return Err(ConfigError::UrlEmpty);
                }
                if url.len() > MAX_URL_LEN {
                    return Err(ConfigError::UrlTooLong);
                }
                if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
                    return Err(ConfigError::UrlHasControlChars);
                }
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Err(ConfigError::UrlSchemeInvalid);
                }
                if let Some((lo, hi)) = expect_status
                    && (*lo < 100 || *hi > 599 || lo > hi)
                {
                    return Err(ConfigError::StatusRangeInvalid);
                }
                if let Some(k) = keyword {
                    if k.is_empty() {
                        return Err(ConfigError::KeywordEmpty);
                    }
                    if k.len() > MAX_KEYWORD_LEN {
                        return Err(ConfigError::KeywordTooLong);
                    }
                }
            }
            MonitorKind::Tcp { host, port } => validate_host(host, *port)?,
            MonitorKind::TlsExpiry {
                host,
                port,
                warn_days,
            } => {
                validate_host(host, *port)?;
                if !(1..=MAX_WARN_DAYS).contains(warn_days) {
                    return Err(ConfigError::WarnDaysOutOfRange);
                }
            }
        }
        Ok(())
    }
}

fn validate_host(host: &str, port: u16) -> Result<(), ConfigError> {
    if host.is_empty() {
        return Err(ConfigError::HostEmpty);
    }
    if host.len() > MAX_HOST_LEN {
        return Err(ConfigError::HostTooLong);
    }
    // Hostname, IPv4 or bare IPv6 characters only.
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
    {
        return Err(ConfigError::HostInvalid);
    }
    if port == 0 {
        return Err(ConfigError::PortZero);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http() -> MonitorConfig {
        MonitorConfig::new(
            "m1",
            MonitorKind::Http {
                url: "https://example.test/".into(),
                expect_status: None,
                keyword: None,
            },
            60,
            10,
        )
    }

    fn tcp(host: &str, port: u16) -> MonitorConfig {
        MonitorConfig::new(
            "m",
            MonitorKind::Tcp {
                host: host.into(),
                port,
            },
            60,
            10,
        )
    }

    #[test]
    fn defaults_are_valid() {
        let c = http();
        assert_eq!(c.validate(), Ok(()));
        assert_eq!(
            (c.fail_threshold, c.recover_threshold, c.repeat_every_secs),
            (3, 1, 3600)
        );
    }

    #[test]
    fn bounds_table() {
        type Edit = fn(&mut MonitorConfig);
        let cases: Vec<(Edit, Result<(), ConfigError>)> = vec![
            (|c| c.id.clear(), Err(ConfigError::IdEmpty)),
            (|c| c.id = "x".repeat(65), Err(ConfigError::IdTooLong)),
            (|c| c.id = "x".repeat(64), Ok(())),
            (
                |c| c.interval_secs = 9,
                Err(ConfigError::IntervalOutOfRange),
            ),
            (
                |c| {
                    c.interval_secs = 10;
                    c.timeout_secs = 10;
                },
                Ok(()),
            ),
            (
                |c| c.interval_secs = 86_401,
                Err(ConfigError::IntervalOutOfRange),
            ),
            (|c| c.timeout_secs = 0, Err(ConfigError::TimeoutInvalid)),
            (|c| c.timeout_secs = 61, Err(ConfigError::TimeoutInvalid)),
            (
                |c| c.fail_threshold = 0,
                Err(ConfigError::FailThresholdOutOfRange),
            ),
            (
                |c| c.fail_threshold = 21,
                Err(ConfigError::FailThresholdOutOfRange),
            ),
            (|c| c.fail_threshold = 20, Ok(())),
            (
                |c| c.recover_threshold = 0,
                Err(ConfigError::RecoverThresholdOutOfRange),
            ),
            (
                |c| c.recover_threshold = 21,
                Err(ConfigError::RecoverThresholdOutOfRange),
            ),
            (
                |c| c.maintenance.push(MaintenanceWindow { start: 5, end: 5 }),
                Err(ConfigError::MaintenanceInvalid),
            ),
            (
                |c| c.maintenance.push(MaintenanceWindow { start: 5, end: 6 }),
                Ok(()),
            ),
            (
                |c| c.maintenance = vec![MaintenanceWindow { start: 1, end: 2 }; 65],
                Err(ConfigError::TooManyMaintenanceWindows),
            ),
        ];
        for (i, (edit, want)) in cases.into_iter().enumerate() {
            let mut c = http();
            edit(&mut c);
            assert_eq!(c.validate(), want, "case {i}");
        }
    }

    #[test]
    fn http_fields() {
        let mut c = http();
        let set = |c: &mut MonitorConfig, url: &str, st: Option<(u16, u16)>, kw: Option<&str>| {
            c.kind = MonitorKind::Http {
                url: url.into(),
                expect_status: st,
                keyword: kw.map(Into::into),
            };
        };
        set(&mut c, "", None, None);
        assert_eq!(c.validate(), Err(ConfigError::UrlEmpty));
        set(&mut c, &format!("https://{}", "a".repeat(2048)), None, None);
        assert_eq!(c.validate(), Err(ConfigError::UrlTooLong));
        set(&mut c, "ftp://x", None, None);
        assert_eq!(c.validate(), Err(ConfigError::UrlSchemeInvalid));
        set(&mut c, "https://x/\r\nHost: y", None, None);
        assert_eq!(c.validate(), Err(ConfigError::UrlHasControlChars));
        set(&mut c, "https://x", Some((99, 200)), None);
        assert_eq!(c.validate(), Err(ConfigError::StatusRangeInvalid));
        set(&mut c, "https://x", Some((300, 200)), None);
        assert_eq!(c.validate(), Err(ConfigError::StatusRangeInvalid));
        set(&mut c, "https://x", Some((200, 600)), None);
        assert_eq!(c.validate(), Err(ConfigError::StatusRangeInvalid));
        set(&mut c, "https://x", Some((200, 399)), Some(""));
        assert_eq!(c.validate(), Err(ConfigError::KeywordEmpty));
        set(&mut c, "https://x", None, Some(&"k".repeat(257)));
        assert_eq!(c.validate(), Err(ConfigError::KeywordTooLong));
        set(&mut c, "http://x", Some((200, 399)), Some("ok"));
        assert_eq!(c.validate(), Ok(()));
    }

    #[test]
    fn host_and_tls_fields() {
        assert_eq!(tcp("", 1).validate(), Err(ConfigError::HostEmpty));
        assert_eq!(
            tcp(&"a".repeat(254), 1).validate(),
            Err(ConfigError::HostTooLong)
        );
        assert_eq!(tcp("a b", 1).validate(), Err(ConfigError::HostInvalid));
        assert_eq!(tcp("a/b", 1).validate(), Err(ConfigError::HostInvalid));
        assert_eq!(tcp("db.internal", 0).validate(), Err(ConfigError::PortZero));
        assert_eq!(tcp("::1", 5432).validate(), Ok(()));
        let tls = |d| {
            MonitorConfig::new(
                "t",
                MonitorKind::TlsExpiry {
                    host: "h.test".into(),
                    port: 443,
                    warn_days: d,
                },
                60,
                10,
            )
        };
        assert_eq!(tls(0).validate(), Err(ConfigError::WarnDaysOutOfRange));
        assert_eq!(tls(366).validate(), Err(ConfigError::WarnDaysOutOfRange));
        assert_eq!(tls(14).validate(), Ok(()));
    }

    #[test]
    fn maintenance_is_half_open() {
        let mut c = http();
        c.maintenance.push(MaintenanceWindow {
            start: 100,
            end: 200,
        });
        assert!(!c.in_maintenance(99));
        assert!(c.in_maintenance(100));
        assert!(c.in_maintenance(199));
        assert!(!c.in_maintenance(200));
    }

    #[test]
    fn status_expectation() {
        let k = MonitorKind::Http {
            url: "http://x".into(),
            expect_status: None,
            keyword: None,
        };
        assert!(k.status_accepts(200) && k.status_accepts(299));
        assert!(!k.status_accepts(301) && !k.status_accepts(500));
        let k = MonitorKind::Http {
            url: "http://x".into(),
            expect_status: Some((200, 399)),
            keyword: None,
        };
        assert!(k.status_accepts(301));
        assert!(
            !MonitorKind::Tcp {
                host: "h".into(),
                port: 1
            }
            .status_accepts(200)
        );
    }
}
