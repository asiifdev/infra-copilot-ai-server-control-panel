//! The rclone setup wizard as a pure state machine (migration-planning skill):
//! create remote, authorize, test, quota check, speed measurement, save.
//!
//! The wizard owns no I/O. Each [`Wizard::advance`] runs at most the commands of one step through
//! a [`Runner`] and returns a typed [`Progress`]. The remote is defined in the environment of each
//! command (ADR 0025), so nothing is written to disk; "create" validates the settings and checks
//! that rclone is installed. "Authorize" cannot run inside the wizard (the person must open a
//! link in a browser), so the wizard asks for the token and takes it with
//! [`Wizard::provide_token`]; the token is a [`Secret`] and is never printed.

use crate::advice::{Advice, ProblemKind, about_unsupported, classify_output, classify_run_error};
use crate::plan::{Credentials, Operation, PlanError, plan};
use crate::runner::{RunError, RunOutput, Runner};
use crate::spec::{RcloneSpec, TransportSpec};
use core_domain::secret::Secret;
use serde_json::Value;
use std::fmt;

/// The steps, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    /// Validate the settings and check that rclone is installed.
    CreateRemote,
    /// Obtain the OAuth token (skipped for remotes that do not use one).
    Authorize,
    /// List the remote (`rclone lsd`).
    Test,
    /// Read the quota (`rclone about`) and compare it with what is needed.
    Quota,
    /// Upload a small object and time it.
    Speed,
    /// Everything passed; the caller stores the result.
    Save,
}

impl Step {
    /// Catalog key of the step's label.
    #[must_use]
    pub const fn label_key(self) -> i18n::MessageKey {
        i18n::MessageKey::new(match self {
            Self::CreateRemote => "transport.step.create",
            Self::Authorize => "transport.step.authorize",
            Self::Test => "transport.step.test",
            Self::Quota => "transport.step.quota",
            Self::Speed => "transport.step.speed",
            Self::Save => "transport.step.save",
        })
    }
}

/// Inputs that shape the checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WizardParams {
    /// Bytes the migration will need on the remote; the quota step fails if free space is known
    /// and smaller.
    pub required_bytes: Option<u64>,
    /// Size of the speed-test object.
    pub speed_bytes: usize,
}

impl Default for WizardParams {
    fn default() -> Self {
        Self {
            required_bytes: None,
            speed_bytes: 4 * 1024 * 1024,
        }
    }
}

/// What `rclone about` reported. Fields a backend omits are `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    /// Total size.
    pub total: Option<u64>,
    /// Used.
    pub used: Option<u64>,
    /// Free.
    pub free: Option<u64>,
}

/// What the wizard learned.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    /// The remote could be listed.
    pub listing_ok: bool,
    /// Quota, when the backend reports it.
    pub quota: Option<Quota>,
    /// The backend does not support `about`; quota is unknown, not zero.
    pub quota_unsupported: bool,
    /// Measured upload speed in bytes per second.
    pub speed_bytes_per_sec: Option<u64>,
    /// The speed-test object could not be deleted and may be left on the remote.
    pub speed_leftover: bool,
}

impl Report {
    /// JSON for the `transports.last_check` column. Numbers and booleans only.
    #[must_use]
    pub fn to_json(&self) -> String {
        let num = |v: Option<u64>| v.map_or(Value::Null, Value::from);
        let quota = self.quota.map_or(Value::Null, |q| {
            serde_json::json!({"total": num(q.total), "used": num(q.used), "free": num(q.free)})
        });
        serde_json::json!({
            "ok": self.listing_ok,
            "quota": quota,
            "quota_unsupported": self.quota_unsupported,
            "speed_bytes_per_sec": num(self.speed_bytes_per_sec),
            "speed_leftover": self.speed_leftover,
        })
        .to_string()
    }
}

/// Result of one [`Wizard::advance`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// This step finished; the next one is ready.
    Advanced(Step),
    /// The person must authorize. Show the localized hint with `command`, open the link
    /// rclone prints, and feed the resulting token to [`Wizard::provide_token`].
    NeedsToken {
        /// The command to run on a machine with a browser (contains no secret).
        command: String,
    },
    /// The step failed. The step stays current, so the caller can fix things and advance again.
    Failed(Advice),
    /// All checks passed; call [`Wizard::finish`].
    ReadyToSave,
}

/// Misuse of the wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardError {
    /// A token was supplied while the wizard is not waiting for one.
    NotWaitingForToken,
    /// The text is not an rclone token (a JSON object with an `access_token`).
    BadToken,
    /// `finish` was called before every step passed.
    NotReady,
}

impl fmt::Display for WizardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotWaitingForToken => "transport_wizard_not_waiting_for_token",
            Self::BadToken => "transport_wizard_bad_token",
            Self::NotReady => "transport_wizard_not_ready",
        })
    }
}

impl std::error::Error for WizardError {}

/// A finished wizard: what to store.
#[derive(Debug)]
pub struct Completed {
    /// The validated spec.
    pub spec: TransportSpec,
    /// The credentials, including the OAuth token when one was needed.
    pub credentials: Credentials,
    /// What the checks found.
    pub report: Report,
}

/// The state machine.
#[derive(Debug)]
pub struct Wizard {
    spec: RcloneSpec,
    credentials: Credentials,
    params: WizardParams,
    step: Step,
    report: Report,
}

impl Wizard {
    /// Starts at [`Step::CreateRemote`].
    #[must_use]
    pub const fn new(spec: RcloneSpec, credentials: Credentials, params: WizardParams) -> Self {
        Self {
            spec,
            credentials,
            params,
            step: Step::CreateRemote,
            report: Report {
                listing_ok: false,
                quota: None,
                quota_unsupported: false,
                speed_bytes_per_sec: None,
                speed_leftover: false,
            },
        }
    }

    /// The current step.
    #[must_use]
    pub const fn step(&self) -> Step {
        self.step
    }

    fn secrets(&self) -> Vec<String> {
        self.credentials
            .pairs()
            .iter()
            .map(|(_, v)| v.expose_secret().to_owned())
            .collect()
    }

    fn fail_output(&self, output: &RunOutput) -> Progress {
        let secrets = self.secrets();
        let refs: Vec<&str> = secrets.iter().map(String::as_str).collect();
        Progress::Failed(classify_output(output, &refs))
    }

    fn run(
        &self,
        runner: &dyn Runner,
        op: &Operation,
    ) -> Result<Result<RunOutput, RunError>, PlanError> {
        let spec = TransportSpec::Rclone(self.spec.clone());
        let p = plan(&spec, Some(&self.credentials), op)?;
        Ok(runner.run(&p))
    }

    /// Runs the current step.
    pub fn advance(&mut self, runner: &dyn Runner) -> Progress {
        match self.step {
            Step::CreateRemote => self.create(runner),
            Step::Authorize => self.authorize(),
            Step::Test => self.test(runner),
            Step::Quota => self.quota(runner),
            Step::Speed => self.speed(runner),
            Step::Save => Progress::ReadyToSave,
        }
    }

    fn done(&mut self, completed: Step, next: Step) -> Progress {
        self.step = next;
        Progress::Advanced(completed)
    }

    fn create(&mut self, runner: &dyn Runner) -> Progress {
        // Building the plans validates every field and the credentials before anything runs.
        for op in [Operation::Verify, Operation::Quota] {
            if self.run_plan_only(&op).is_err() {
                return Progress::Failed(Advice::of(ProblemKind::InvalidConfig));
            }
        }
        let spec = TransportSpec::Rclone(self.spec.clone());
        let Ok(probe) = plan(&spec, None, &Operation::Probe) else {
            return Progress::Failed(Advice::of(ProblemKind::InvalidConfig));
        };
        match runner.run(&probe) {
            Err(e) => Progress::Failed(classify_run_error(e)),
            Ok(out) if !out.success() => self.fail_output(&out),
            Ok(_) => {
                let next = if self.needs_token() {
                    Step::Authorize
                } else {
                    Step::Test
                };
                self.done(Step::CreateRemote, next)
            }
        }
    }

    fn run_plan_only(&self, op: &Operation) -> Result<(), PlanError> {
        let spec = TransportSpec::Rclone(self.spec.clone());
        plan(&spec, Some(&self.credentials), op).map(|_| ())
    }

    fn needs_token(&self) -> bool {
        self.spec.kind().needs_oauth() && !self.credentials.has("token")
    }

    fn authorize(&mut self) -> Progress {
        if self.needs_token() {
            return Progress::NeedsToken {
                command: format!("rclone authorize \"{}\"", self.spec.kind().as_str()),
            };
        }
        self.done(Step::Authorize, Step::Test)
    }

    /// Supplies the OAuth token the person obtained from `rclone authorize`. Accepts the bare
    /// JSON or the whole `Paste the following ---> ... <---End paste` block. The text is
    /// parsed, re-serialized on one line, and kept only as a [`Secret`].
    ///
    /// # Errors
    /// [`WizardError::NotWaitingForToken`] outside the authorize step;
    /// [`WizardError::BadToken`] when the text is not an rclone token.
    pub fn provide_token(&mut self, text: &Secret) -> Result<(), WizardError> {
        if self.step != Step::Authorize || !self.needs_token() {
            return Err(WizardError::NotWaitingForToken);
        }
        let raw = text.expose_secret();
        let body = match raw.split_once("--->") {
            Some((_, after)) => after.split("<---").next().unwrap_or(after),
            None => raw,
        };
        let value: Value = serde_json::from_str(body.trim()).map_err(|_| WizardError::BadToken)?;
        let has_access = value
            .get("access_token")
            .and_then(Value::as_str)
            .is_some_and(|t| !t.is_empty());
        if !has_access {
            return Err(WizardError::BadToken);
        }
        let token = Secret::new(value.to_string());
        let credentials = std::mem::replace(&mut self.credentials, empty_credentials());
        match credentials.with("token", token) {
            Ok(updated) => {
                self.credentials = updated;
                Ok(())
            }
            Err(_) => Err(WizardError::BadToken),
        }
    }

    fn test(&mut self, runner: &dyn Runner) -> Progress {
        match self.run(runner, &Operation::Verify) {
            Err(_) => Progress::Failed(Advice::of(ProblemKind::InvalidConfig)),
            Ok(Err(e)) => Progress::Failed(classify_run_error(e)),
            Ok(Ok(out)) if !out.success() => self.fail_output(&out),
            Ok(Ok(_)) => {
                self.report.listing_ok = true;
                self.done(Step::Test, Step::Quota)
            }
        }
    }

    fn quota(&mut self, runner: &dyn Runner) -> Progress {
        let out = match self.run(runner, &Operation::Quota) {
            Err(_) => return Progress::Failed(Advice::of(ProblemKind::InvalidConfig)),
            Ok(Err(e)) => return Progress::Failed(classify_run_error(e)),
            Ok(Ok(out)) => out,
        };
        if !out.success() {
            if about_unsupported(&out) {
                self.report.quota_unsupported = true;
                return self.done(Step::Quota, Step::Speed);
            }
            return self.fail_output(&out);
        }
        let Some(quota) = parse_quota(&out.stdout) else {
            return Progress::Failed(Advice::of(ProblemKind::Unknown));
        };
        self.report.quota = Some(quota);
        if let (Some(need), Some(free)) = (self.params.required_bytes, quota.free)
            && free < need
        {
            return Progress::Failed(Advice::of(ProblemKind::QuotaExceeded));
        }
        self.done(Step::Quota, Step::Speed)
    }

    fn speed(&mut self, runner: &dyn Runner) -> Progress {
        let bytes = self.params.speed_bytes;
        let out = match self.run(runner, &Operation::SpeedWrite { bytes }) {
            Err(_) => return Progress::Failed(Advice::of(ProblemKind::InvalidConfig)),
            Ok(Err(e)) => return Progress::Failed(classify_run_error(e)),
            Ok(Ok(out)) => out,
        };
        if !out.success() {
            return self.fail_output(&out);
        }
        let millis = u64::try_from(out.elapsed.as_millis())
            .unwrap_or(u64::MAX)
            .max(1);
        let size = u64::try_from(bytes).unwrap_or(u64::MAX);
        self.report.speed_bytes_per_sec = Some(size.saturating_mul(1000) / millis);
        // Clean up; a failure here is reported, not fatal.
        let cleaned = matches!(
            self.run(runner, &Operation::SpeedCleanup),
            Ok(Ok(ref o)) if o.success()
        );
        self.report.speed_leftover = !cleaned;
        self.done(Step::Speed, Step::Save)
    }

    /// Consumes the wizard once every step passed.
    ///
    /// # Errors
    /// [`WizardError::NotReady`] before the last step.
    pub fn finish(self) -> Result<Completed, WizardError> {
        if self.step != Step::Save {
            return Err(WizardError::NotReady);
        }
        Ok(Completed {
            spec: TransportSpec::Rclone(self.spec),
            credentials: self.credentials,
            report: self.report,
        })
    }
}

fn empty_credentials() -> Credentials {
    // An empty list always passes validation.
    Credentials::new(Vec::new()).unwrap_or_else(|_| unreachable!("empty credentials are valid"))
}

fn parse_quota(stdout: &[u8]) -> Option<Quota> {
    let value: Value = serde_json::from_slice(stdout).ok()?;
    let obj = value.as_object()?;
    let field = |k: &str| obj.get(k).and_then(Value::as_u64);
    Some(Quota {
        total: field("total"),
        used: field("used"),
        free: field("free"),
    })
}
