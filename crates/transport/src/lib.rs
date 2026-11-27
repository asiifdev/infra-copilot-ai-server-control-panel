//! Transports: the closed set of ways data can move (rclone, rsync, scp, SSH pipe, managed
//! storage), the argv-only command plans that describe them, runners that start those plans, and
//! the rclone setup wizard (ADR 0025).
//!
//! This crate PLANS and VALIDATES. Nothing here starts rclone, rsync, scp or ssh by itself: a
//! [`Runner`] is the single seam where a process would start, and tests use [`FakeRunner`].
//!
//! Security rules enforced here (CLAUDE.md rules 1, 2, 4):
//! - every field is validated before it reaches an argument; paths never start with `-`;
//! - a plan is `program + args + env + stdin`, never a shell string; user-derived values come
//!   after `--`;
//! - secrets are [`Secret`] values that appear only in the environment or stdin of a plan, never
//!   in `args`, and print as `***`;
//! - tool output is untrusted text: it is matched against fixed phrases and a redacted excerpt
//!   is kept, nothing more.

mod advice;
mod plan;
mod policy;
mod runner;
mod spec;
pub mod store;
pub mod testing;
pub mod validate;
mod wizard;

pub use advice::{Advice, ProblemKind, about_unsupported, classify_output, classify_run_error};
pub use core_domain::secret::Secret;
pub use plan::{
    CommandPlan, CopyOptions, Credentials, EnvValue, MAX_SPEED_BYTES, MAX_TRANSFERS, Operation,
    PlanError, SPEED_LEAF, StdinData, plan,
};
pub use policy::{
    EndpointBlocked, EndpointPolicy, HostResolver, PolicyRunError, SystemResolver, check_host,
    check_host_resolved, is_internal_ip, run_with_policy,
};
pub use runner::{
    ENV_ALLOW_LIST, FakeRunner, ProcessRunner, RecordedCall, RunError, RunOutput, Runner,
};
pub use spec::{
    RcloneSpec, RemoteKind, SSH_PORT, SpecError, SshTarget, TransportKind, TransportSpec,
};
pub use validate::{Field, Problem, ValidationError};
pub use wizard::{Completed, Progress, Quota, Report, Step, Wizard, WizardError, WizardParams};
