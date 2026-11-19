//! Command plans: a program, an argument vector, an environment and optional stdin. Never a
//! shell string. Secrets are [`Secret`] values and appear only in the environment or stdin,
//! never in `args` (arguments are visible in process listings).

use crate::spec::{RcloneSpec, RemoteKind, SshTarget, TransportKind, TransportSpec};
use crate::validate::{self, ValidationError};
use core_domain::redact::redact;
use core_domain::secret::Secret;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;

/// The value of an environment variable.
pub enum EnvValue {
    /// Not secret (a backend name, a region).
    Plain(String),
    /// Secret: printed as `***` everywhere.
    Secret(Secret),
}

impl EnvValue {
    /// The raw value. Call only where the process is started.
    #[must_use]
    pub fn expose(&self) -> &str {
        match self {
            Self::Plain(v) => v,
            Self::Secret(s) => s.expose_secret(),
        }
    }

    /// Whether the value is secret.
    #[must_use]
    pub const fn is_secret(&self) -> bool {
        matches!(self, Self::Secret(_))
    }
}

impl fmt::Debug for EnvValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plain(v) => write!(f, "Plain({v:?})"),
            Self::Secret(s) => write!(f, "Secret({s:?})"),
        }
    }
}

impl PartialEq for EnvValue {
    fn eq(&self, other: &Self) -> bool {
        self.is_secret() == other.is_secret() && self.expose() == other.expose()
    }
}

/// Bytes written to the child's stdin.
pub enum StdinData {
    /// Not secret (a speed-test payload).
    Bytes(Vec<u8>),
    /// Secret.
    Secret(Secret),
}

impl StdinData {
    /// The bytes to write.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Bytes(b) => b,
            Self::Secret(s) => s.expose_secret().as_bytes(),
        }
    }
}

impl fmt::Debug for StdinData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes(b) => write!(f, "Bytes({} bytes)", b.len()),
            Self::Secret(_) => f.write_str("Secret(***)"),
        }
    }
}

impl PartialEq for StdinData {
    fn eq(&self, other: &Self) -> bool {
        self.bytes() == other.bytes()
    }
}

/// One process invocation, described but not started.
#[derive(Debug, PartialEq)]
pub struct CommandPlan {
    /// Program name, looked up on `PATH` by the runner. Always a fixed constant here.
    pub program: &'static str,
    /// Arguments, one per element. Never contains a secret.
    pub args: Vec<String>,
    /// Environment added to the (allow-listed) child environment.
    pub env: Vec<(String, EnvValue)>,
    /// Data for stdin.
    pub stdin: Option<StdinData>,
}

impl fmt::Display for CommandPlan {
    /// A log-safe rendering: secret values are `***`, and the whole line also passes through the
    /// redactor. Not meant to be pasted into a shell.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut line = String::new();
        for (name, value) in &self.env {
            match value {
                EnvValue::Plain(v) => line.push_str(&format!("{name}={v:?} ")),
                EnvValue::Secret(_) => line.push_str(&format!("{name}=*** ")),
            }
        }
        line.push_str(self.program);
        for arg in &self.args {
            line.push_str(&format!(" {arg:?}"));
        }
        f.write_str(&redact(&line).text)
    }
}

/// Credentials of a transport: option name to secret value. Stored sealed in the vault.
pub struct Credentials(Vec<(String, Secret)>);

impl Credentials {
    /// Builds credentials. Names must be valid rclone option names; values non-empty, bounded
    /// and free of control characters (they travel in environment variables).
    ///
    /// # Errors
    /// [`ValidationError`] for a bad name or value; [`ValidationError`] with
    /// `Problem::BadShape` for a repeated name.
    pub fn new(pairs: Vec<(String, Secret)>) -> Result<Self, ValidationError> {
        let mut seen = BTreeMap::new();
        for (name, value) in pairs {
            validate::option_name(&name)?;
            if !RemoteKind::ALL
                .iter()
                .any(|k| k.allowed_options().contains(&name.as_str()))
            {
                return Err(ValidationError {
                    field: validate::Field::OptionName,
                    problem: validate::Problem::Reserved,
                });
            }
            validate::option_value(value.expose_secret())?;
            if seen.insert(name, value).is_some() {
                return Err(ValidationError {
                    field: validate::Field::OptionName,
                    problem: validate::Problem::BadShape,
                });
            }
        }
        Ok(Self(seen.into_iter().collect()))
    }

    /// The names (not secret), sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(n, _)| n.as_str())
    }

    /// Whether an option is present.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(n, _)| n == name)
    }

    /// The pairs, for injection into an environment.
    #[must_use]
    pub fn pairs(&self) -> &[(String, Secret)] {
        &self.0
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Adds or replaces one option (for example the OAuth `token`).
    ///
    /// # Errors
    /// As [`Credentials::new`].
    pub fn with(mut self, name: &str, value: Secret) -> Result<Self, ValidationError> {
        self.0.retain(|(n, _)| n != name);
        self.0.push((name.to_owned(), value));
        Self::new(self.0)
    }

    /// The text sealed in the vault: a JSON object of strings.
    #[must_use]
    pub fn to_vault_text(&self) -> Secret {
        let mut map = Map::new();
        for (name, value) in &self.0 {
            map.insert(
                name.clone(),
                Value::String(value.expose_secret().to_owned()),
            );
        }
        Secret::new(Value::Object(map).to_string())
    }

    /// Reads the vault text back, re-validating. `None` when the text is not a JSON object of
    /// valid strings.
    #[must_use]
    pub fn from_vault_text(text: &Secret) -> Option<Self> {
        let value: Value = serde_json::from_str(text.expose_secret()).ok()?;
        let obj = value.as_object()?;
        let mut pairs = Vec::with_capacity(obj.len());
        for (k, v) in obj {
            pairs.push((k.clone(), Secret::new(v.as_str()?)));
        }
        Self::new(pairs).ok()
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Credentials({:?})", self.names().collect::<Vec<_>>())
    }
}

/// Tuning shared by copy operations.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CopyOptions {
    /// Patterns to leave out, each passed as its own `--exclude=<pattern>` argument.
    pub excludes: Vec<String>,
    /// Report what would happen without changing anything.
    pub dry_run: bool,
    /// Parallel transfers (rclone only), 1 to 64.
    pub transfers: Option<u32>,
}

/// Highest accepted `--transfers`.
pub const MAX_TRANSFERS: u32 = 64;
/// Largest speed-test payload.
pub const MAX_SPEED_BYTES: usize = 64 * 1024 * 1024;
/// Name of the speed-test object inside the remote path.
pub const SPEED_LEAF: &str = ".nodivara-speedtest";

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// Is the tool installed (`rclone version`, `rsync --version`, `ssh -V`)?
    Probe,
    /// Can the destination be reached (`rclone lsd`, or `ssh ... true`)?
    Verify,
    /// Quota of the remote (`rclone about --json`). rclone only.
    Quota,
    /// Start the OAuth flow (`rclone authorize`). rclone only; the person opens the link.
    Authorize,
    /// Upload this many zero bytes to measure speed. rclone only.
    SpeedWrite {
        /// Payload size.
        bytes: usize,
    },
    /// Delete the speed-test object. rclone only.
    SpeedCleanup,
    /// Local to remote.
    Push {
        /// Absolute local path.
        local: String,
        /// Tuning.
        opts: CopyOptions,
    },
    /// Remote to local.
    Pull {
        /// Absolute local path.
        local: String,
        /// Tuning.
        opts: CopyOptions,
    },
}

/// Why no plan could be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    /// A field failed validation.
    Invalid(ValidationError),
    /// Managed storage is declared but not implemented.
    UnsupportedManaged,
    /// This kind has no such operation.
    UnsupportedOperation(TransportKind),
    /// Credentials were given to a kind that authenticates elsewhere.
    CredentialsNotAccepted(TransportKind),
    /// An option appears both as plain option and as credential.
    DuplicateOption,
}

impl From<ValidationError> for PlanError {
    fn from(e: ValidationError) -> Self {
        Self::Invalid(e)
    }
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => e.fmt(f),
            Self::UnsupportedManaged => f.write_str("transport_unsupported_managed"),
            Self::UnsupportedOperation(k) => {
                write!(f, "transport_unsupported_operation_{}", k.as_str())
            }
            Self::CredentialsNotAccepted(k) => {
                write!(f, "transport_credentials_not_accepted_{}", k.as_str())
            }
            Self::DuplicateOption => f.write_str("transport_duplicate_option"),
        }
    }
}

impl std::error::Error for PlanError {}

/// Options for every ssh invocation: never prompt, never accept an unknown host key silently,
/// give up quickly. The same list feeds `ssh`, `scp` and the `-e` of rsync.
const SSH_OPTIONS: [&str; 6] = [
    "BatchMode=yes",
    "StrictHostKeyChecking=yes",
    "ConnectTimeout=15",
    "ServerAliveInterval=15",
    "ServerAliveCountMax=3",
    "ForwardAgent=no",
];

/// Builds the plan for `op` on `spec`. Pure: nothing is started.
///
/// # Errors
/// [`PlanError`].
pub fn plan(
    spec: &TransportSpec,
    credentials: Option<&Credentials>,
    op: &Operation,
) -> Result<CommandPlan, PlanError> {
    if let (Some(_), false) = (credentials, spec.kind().accepts_credentials()) {
        if matches!(spec, TransportSpec::Managed) {
            return Err(PlanError::UnsupportedManaged);
        }
        return Err(PlanError::CredentialsNotAccepted(spec.kind()));
    }
    match spec {
        TransportSpec::Managed => Err(PlanError::UnsupportedManaged),
        TransportSpec::Rclone(s) => rclone_plan(s, credentials, op),
        TransportSpec::Rsync(t) | TransportSpec::Scp(t) | TransportSpec::SshPipe(t) => {
            ssh_plan(spec.kind(), t, op)
        }
    }
}

fn check_copy(local: &str, opts: &CopyOptions) -> Result<(), PlanError> {
    validate::local_path(local)?;
    for pattern in &opts.excludes {
        validate::exclude(pattern)?;
    }
    if let Some(n) = opts.transfers {
        validate::count(n, MAX_TRANSFERS)?;
    }
    Ok(())
}

/// The environment that defines the remote: `RCLONE_CONFIG_<REMOTE>_TYPE`, plain options, then
/// secrets, in a fixed order.
fn rclone_env(
    spec: &RcloneSpec,
    credentials: Option<&Credentials>,
) -> Result<Vec<(String, EnvValue)>, PlanError> {
    let prefix = format!("RCLONE_CONFIG_{}_", spec.remote().to_ascii_uppercase());
    let mut env = vec![(
        format!("{prefix}TYPE"),
        EnvValue::Plain(spec.kind().as_str().to_owned()),
    )];
    for (name, value) in spec.options() {
        if credentials.is_some_and(|c| c.has(name)) {
            return Err(PlanError::DuplicateOption);
        }
        env.push((
            format!("{prefix}{}", name.to_ascii_uppercase()),
            EnvValue::Plain(value.clone()),
        ));
    }
    if let Some(credentials) = credentials {
        for (name, value) in credentials.pairs() {
            validate::option_allowed(spec.kind(), name)?;
            env.push((
                format!("{prefix}{}", name.to_ascii_uppercase()),
                EnvValue::Secret(Secret::new(value.expose_secret())),
            ));
        }
    }
    Ok(env)
}

/// Flags every rclone call carries: no config file (the remote lives in the environment, so
/// nothing is written to disk), no password prompt.
fn rclone_base(subcommand: &str) -> Vec<String> {
    vec![
        subcommand.to_owned(),
        "--config=".to_owned(),
        "--ask-password=false".to_owned(),
    ]
}

fn rclone_plan(
    spec: &RcloneSpec,
    credentials: Option<&Credentials>,
    op: &Operation,
) -> Result<CommandPlan, PlanError> {
    let mut stdin = None;
    let mut args;
    let mut with_env = true;
    match op {
        Operation::Probe => {
            args = vec!["version".to_owned(), "--config=".to_owned()];
            with_env = false;
        }
        Operation::Verify => {
            args = rclone_base("lsd");
            args.push("--".to_owned());
            args.push(spec.remote_arg(None));
        }
        Operation::Quota => {
            args = rclone_base("about");
            args.push("--json".to_owned());
            args.push("--".to_owned());
            args.push(spec.remote_arg(None));
        }
        Operation::Authorize => {
            args = rclone_base("authorize");
            args.push("--".to_owned());
            args.push(spec.kind().as_str().to_owned());
            with_env = false;
        }
        Operation::SpeedWrite { bytes } => {
            if *bytes == 0 || *bytes > MAX_SPEED_BYTES {
                return Err(ValidationError {
                    field: validate::Field::Number,
                    problem: validate::Problem::OutOfRange,
                }
                .into());
            }
            args = rclone_base("rcat");
            args.push("--".to_owned());
            args.push(spec.remote_arg(Some(SPEED_LEAF)));
            stdin = Some(StdinData::Bytes(vec![0_u8; *bytes]));
        }
        Operation::SpeedCleanup => {
            args = rclone_base("deletefile");
            args.push("--".to_owned());
            args.push(spec.remote_arg(Some(SPEED_LEAF)));
        }
        Operation::Push { local, opts } | Operation::Pull { local, opts } => {
            check_copy(local, opts)?;
            args = rclone_base("copy");
            if let Some(n) = opts.transfers {
                args.push(format!("--transfers={n}"));
            }
            if opts.dry_run {
                args.push("--dry-run".to_owned());
            }
            for pattern in &opts.excludes {
                args.push(format!("--exclude={pattern}"));
            }
            args.push("--".to_owned());
            if matches!(op, Operation::Push { .. }) {
                args.push(local.clone());
                args.push(spec.remote_arg(None));
            } else {
                args.push(spec.remote_arg(None));
                args.push(local.clone());
            }
        }
    }
    let env = if with_env {
        rclone_env(spec, credentials)?
    } else {
        Vec::new()
    };
    Ok(CommandPlan {
        program: "rclone",
        args,
        env,
        stdin,
    })
}

fn ssh_args(port: u16, flag: &str) -> Vec<String> {
    let mut args = Vec::new();
    for option in SSH_OPTIONS {
        args.push("-o".to_owned());
        args.push(option.to_owned());
    }
    args.push(flag.to_owned());
    args.push(port.to_string());
    args
}

fn ssh_plan(
    kind: TransportKind,
    target: &SshTarget,
    op: &Operation,
) -> Result<CommandPlan, PlanError> {
    let unsupported = PlanError::UnsupportedOperation(kind);
    let (program, args): (&'static str, Vec<String>) = match op {
        Operation::Probe => ("ssh", vec!["-V".to_owned()]),
        Operation::Verify => {
            let mut args = ssh_args(target.port(), "-p");
            args.extend(["--".to_owned(), target.login(), "true".to_owned()]);
            ("ssh", args)
        }
        Operation::Quota
        | Operation::Authorize
        | Operation::SpeedWrite { .. }
        | Operation::SpeedCleanup => return Err(unsupported),
        Operation::Push { local, opts } | Operation::Pull { local, opts } => {
            check_copy(local, opts)?;
            let push = matches!(op, Operation::Push { .. });
            match kind {
                TransportKind::Rsync => {
                    let mut args = vec![
                        "--archive".to_owned(),
                        "--partial".to_owned(),
                        "--timeout=60".to_owned(),
                        format!("--rsh=ssh {}", rsh_options(target.port())),
                    ];
                    if opts.dry_run {
                        args.push("--dry-run".to_owned());
                    }
                    for pattern in &opts.excludes {
                        args.push(format!("--exclude={pattern}"));
                    }
                    args.push("--".to_owned());
                    if push {
                        args.extend([local.clone(), target.remote_arg()]);
                    } else {
                        args.extend([target.remote_arg(), local.clone()]);
                    }
                    ("rsync", args)
                }
                TransportKind::Scp => {
                    if opts.dry_run || !opts.excludes.is_empty() || opts.transfers.is_some() {
                        // scp has no dry run, excludes or parallelism: refuse rather than
                        // silently doing something different from what was asked.
                        return Err(unsupported);
                    }
                    let mut args = ssh_args(target.port(), "-P");
                    args.insert(0, "-q".to_owned());
                    args.insert(1, "-r".to_owned());
                    args.push("--".to_owned());
                    if push {
                        args.extend([local.clone(), target.remote_arg()]);
                    } else {
                        args.extend([target.remote_arg(), local.clone()]);
                    }
                    ("scp", args)
                }
                _ => {
                    // ssh pipe: the plan is the remote half; the caller connects the stream.
                    if opts.dry_run || !opts.excludes.is_empty() || opts.transfers.is_some() {
                        return Err(unsupported);
                    }
                    let mut args = ssh_args(target.port(), "-p");
                    args.push("--".to_owned());
                    args.push(target.login());
                    // The path passed the strict allow-list, so it is inert for the remote shell.
                    args.push(if push {
                        format!("cat > {}", target.path())
                    } else {
                        format!("cat {}", target.path())
                    });
                    ("ssh", args)
                }
            }
        }
    };
    Ok(CommandPlan {
        program,
        args,
        env: Vec::new(),
        stdin: None,
    })
}

/// The `-o` options as one string, for rsync's `--rsh` (rsync splits it on spaces, no shell).
fn rsh_options(port: u16) -> String {
    let mut parts = Vec::new();
    for option in SSH_OPTIONS {
        parts.push("-o".to_owned());
        parts.push(option.to_owned());
    }
    parts.push("-p".to_owned());
    parts.push(port.to_string());
    parts.join(" ")
}
