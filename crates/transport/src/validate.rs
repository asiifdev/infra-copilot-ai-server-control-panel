//! Strict field validation. Every value that ends up in an argument vector passes through here
//! first. Errors name the field and the problem, never the offending value (it may be hostile
//! text or contain a secret).

use std::fmt;
use std::net::{IpAddr, Ipv6Addr};

use crate::spec::RemoteKind;

/// Which field failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// Name of a stored transport.
    Name,
    /// rclone remote name.
    RemoteName,
    /// A remote path (rclone).
    RemotePath,
    /// A remote path on an SSH host (rsync, scp, ssh pipe).
    SshPath,
    /// A local path.
    LocalPath,
    /// Host name or IP literal.
    Host,
    /// Login name.
    User,
    /// TCP port.
    Port,
    /// Name of an rclone option.
    OptionName,
    /// Value of an rclone option.
    OptionValue,
    /// An exclude pattern.
    Exclude,
    /// A numeric tuning value.
    Number,
}

impl Field {
    const fn code(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::RemoteName => "remote_name",
            Self::RemotePath => "remote_path",
            Self::SshPath => "ssh_path",
            Self::LocalPath => "local_path",
            Self::Host => "host",
            Self::User => "user",
            Self::Port => "port",
            Self::OptionName => "option_name",
            Self::OptionValue => "option_value",
            Self::Exclude => "exclude",
            Self::Number => "number",
        }
    }
}

/// What is wrong with the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// Empty.
    Empty,
    /// Longer than the limit.
    TooLong,
    /// Starts with `-` and could be read as an option.
    LeadingDash,
    /// Contains NUL, a newline or another control character.
    ControlChar,
    /// Contains an invisible or direction-changing Unicode character.
    InvisibleChar,
    /// Contains a character outside the allowed set.
    BadChar,
    /// Contains a `..` segment.
    ParentSegment,
    /// Not an absolute path where one is required.
    NotAbsolute,
    /// Wrong overall shape (for example a host label that starts with `-`).
    BadShape,
    /// Outside the allowed numeric range.
    OutOfRange,
    /// Reserved for the product (for example the rclone option `type`).
    Reserved,
}

impl Problem {
    const fn code(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::TooLong => "too_long",
            Self::LeadingDash => "leading_dash",
            Self::ControlChar => "control_char",
            Self::InvisibleChar => "invisible_char",
            Self::BadChar => "bad_char",
            Self::ParentSegment => "parent_segment",
            Self::NotAbsolute => "not_absolute",
            Self::BadShape => "bad_shape",
            Self::OutOfRange => "out_of_range",
            Self::Reserved => "reserved",
        }
    }
}

/// A rejected value. `Display` is a stable machine code and never echoes the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationError {
    /// Which field.
    pub field: Field,
    /// What is wrong.
    pub problem: Problem,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "transport_invalid_{}_{}",
            self.field.code(),
            self.problem.code()
        )
    }
}

impl std::error::Error for ValidationError {}

type Check = Result<(), ValidationError>;

const fn err(field: Field, problem: Problem) -> ValidationError {
    ValidationError { field, problem }
}

/// Maximum length of a path.
pub const MAX_PATH: usize = 1024;
/// Maximum length of an option value (an OAuth token JSON is the largest legitimate one).
pub const MAX_OPTION_VALUE: usize = 8192;

fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

/// Shared text rules: non-empty, bounded, no leading dash (unless allowed), no control or
/// invisible characters.
fn text(field: Field, s: &str, max: usize, dash_ok: bool) -> Check {
    if s.is_empty() {
        return Err(err(field, Problem::Empty));
    }
    if s.len() > max {
        return Err(err(field, Problem::TooLong));
    }
    if !dash_ok && s.starts_with('-') {
        return Err(err(field, Problem::LeadingDash));
    }
    if s.chars().any(char::is_control) {
        return Err(err(field, Problem::ControlChar));
    }
    if s.chars().any(invisible) {
        return Err(err(field, Problem::InvisibleChar));
    }
    Ok(())
}

fn no_parent_segment(field: Field, s: &str) -> Check {
    if s.split('/').any(|seg| seg == "..") {
        Err(err(field, Problem::ParentSegment))
    } else {
        Ok(())
    }
}

/// Name of a stored transport (free text, shown in the UI, never put in an argument).
///
/// # Errors
/// Empty, too long, or containing control or invisible characters.
pub fn transport_name(s: &str) -> Check {
    text(Field::Name, s, 100, true)
}

/// rclone remote name: a lowercase letter, then lowercase letters, digits or `_`, at most 32.
/// Lowercase only, so the environment variable name `RCLONE_CONFIG_<NAME>_...` is unambiguous.
///
/// # Errors
/// Anything outside that shape.
pub fn remote_name(s: &str) -> Check {
    text(Field::RemoteName, s, 32, false)?;
    let mut chars = s.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    if !first_ok {
        return Err(err(Field::RemoteName, Problem::BadShape));
    }
    if chars.any(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')) {
        return Err(err(Field::RemoteName, Problem::BadChar));
    }
    Ok(())
}

/// Path inside an rclone remote. May be empty (the remote root). Spaces and shell
/// metacharacters are allowed because the path is one argument and no shell is involved.
///
/// # Errors
/// Leading `-`, control or invisible characters, `..` segments, or too long.
pub fn rclone_path(s: &str) -> Check {
    if s.is_empty() {
        return Ok(());
    }
    text(Field::RemotePath, s, MAX_PATH, false)?;
    no_parent_segment(Field::RemotePath, s)
}

/// Path on an SSH host. A strict allow-list, because rsync and ssh hand the path to a remote
/// shell: letters, digits and `._/+=,@%-` only (so no spaces, quotes, `;`, `$`, backticks).
///
/// # Errors
/// Empty, leading `-`, `..` segments, or any character outside the allow-list.
pub fn ssh_path(s: &str) -> Check {
    text(Field::SshPath, s, MAX_PATH, false)?;
    if s.chars()
        .any(|c| !(c.is_ascii_alphanumeric() || "._/+=,@%-".contains(c)))
    {
        return Err(err(Field::SshPath, Problem::BadChar));
    }
    no_parent_segment(Field::SshPath, s)
}

/// A local path: absolute, so it can never be read as an option or as `host:path`.
///
/// # Errors
/// Not starting with `/`, control or invisible characters, `..` segments, or too long.
pub fn local_path(s: &str) -> Check {
    text(Field::LocalPath, s, MAX_PATH, false)?;
    if !s.starts_with('/') {
        return Err(err(Field::LocalPath, Problem::NotAbsolute));
    }
    no_parent_segment(Field::LocalPath, s)
}

/// Host name or IP literal. ASCII only; a name needs non-empty labels of at most 63 characters
/// that do not start or end with `-`.
///
/// # Errors
/// Anything else, including a leading `-`.
pub fn host(s: &str) -> Check {
    text(Field::Host, s, 253, false)?;
    if s.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    if s.chars()
        .any(|c| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
    {
        return Err(err(Field::Host, Problem::BadChar));
    }
    let labels_ok = s.trim_end_matches('.').split('.').all(|label| {
        !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-')
    });
    if labels_ok {
        Ok(())
    } else {
        Err(err(Field::Host, Problem::BadShape))
    }
}

/// Whether the host text is an IPv6 literal (needs brackets in `host:path` forms).
#[must_use]
pub fn is_ipv6(s: &str) -> bool {
    s.parse::<Ipv6Addr>().is_ok()
}

/// Login name: POSIX portable, at most 32, no leading `-`.
///
/// # Errors
/// Anything outside `[A-Za-z_][A-Za-z0-9_.-]*`.
pub fn user(s: &str) -> Check {
    text(Field::User, s, 32, false)?;
    let mut chars = s.chars();
    if !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
    {
        return Err(err(Field::User, Problem::BadShape));
    }
    if chars.any(|c| !(c.is_ascii_alphanumeric() || "_.-".contains(c))) {
        return Err(err(Field::User, Problem::BadChar));
    }
    Ok(())
}

/// TCP port, 1 to 65535.
///
/// # Errors
/// Zero.
pub const fn port(p: u16) -> Check {
    if p == 0 {
        Err(err(Field::Port, Problem::OutOfRange))
    } else {
        Ok(())
    }
}

/// rclone option name shape: lowercase letters, digits and `_`, at most 64, not `type`. Which
/// names are acceptable is decided per backend by [`option_allowed`].
///
/// # Errors
/// Anything else.
pub fn option_name(s: &str) -> Check {
    text(Field::OptionName, s, 64, false)?;
    if s.chars()
        .any(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
    {
        return Err(err(Field::OptionName, Problem::BadChar));
    }
    if s == "type" {
        return Err(err(Field::OptionName, Problem::Reserved));
    }
    Ok(())
}

/// The option must be on the backend's allow-list ([`RemoteKind::allowed_options`]).
///
/// # Errors
/// [`Problem::Reserved`] for any name that is not listed (it may run a program or read a file).
pub fn option_allowed(kind: RemoteKind, name: &str) -> Check {
    if kind.allowed_options().contains(&name) {
        Ok(())
    } else {
        Err(err(Field::OptionName, Problem::Reserved))
    }
}

/// rclone option value (an environment variable value: no control characters).
///
/// # Errors
/// Empty, too long, control or invisible characters.
pub fn option_value(s: &str) -> Check {
    // A value may start with `-`: it travels in the environment, not in argv.
    text(Field::OptionValue, s, MAX_OPTION_VALUE, true)
}

/// Exclude pattern, passed as `--exclude=<pattern>`.
///
/// # Errors
/// Empty, over 256 bytes, leading `-`, control or invisible characters.
pub fn exclude(s: &str) -> Check {
    text(Field::Exclude, s, 256, false)
}

/// A bounded number such as the transfer count.
///
/// # Errors
/// Outside `1..=max`.
pub const fn count(n: u32, max: u32) -> Check {
    if n == 0 || n > max {
        Err(err(Field::Number, Problem::OutOfRange))
    } else {
        Ok(())
    }
}
