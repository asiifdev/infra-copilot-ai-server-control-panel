//! Hand-rolled argument parsing (no argument-parser crate, decision 0013): `--name value`
//! options, `--flag` switches and positional words.

/// Parsed arguments of one command.
#[derive(Default)]
pub(crate) struct Parsed {
    options: Vec<(String, String)>,
    flags: Vec<String>,
    pub(crate) positional: Vec<String>,
}

impl Parsed {
    pub(crate) fn option(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub(crate) fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }
}

/// Splits `args` into value options (`values`), switches (`flags`) and positional words. An
/// unknown or repeated option, or a value option without a value, is an error carrying the
/// option name (never its value: it might be a secret). `--` ends option parsing.
pub(crate) fn parse(args: &[String], values: &[&str], flags: &[&str]) -> Result<Parsed, String> {
    let mut parsed = Parsed::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            parsed.positional.extend(iter.by_ref().cloned());
            break;
        }
        if !arg.starts_with("--") {
            parsed.positional.push(arg.clone());
            continue;
        }
        if flags.contains(&arg.as_str()) {
            if parsed.flag(arg) {
                return Err(arg.clone());
            }
            parsed.flags.push(arg.clone());
        } else if values.contains(&arg.as_str()) {
            let Some(value) = iter.next() else {
                return Err(arg.clone());
            };
            // A repeated option is ambiguous; refusing beats silently picking the first.
            if parsed.option(arg).is_some() {
                return Err(arg.clone());
            }
            parsed.options.push((arg.clone(), value.clone()));
        } else {
            return Err(arg.clone());
        }
    }
    Ok(parsed)
}

/// Options every command accepts, taken out before the command is parsed.
#[derive(Default)]
pub(crate) struct Globals {
    pub(crate) db: Option<String>,
    pub(crate) org: Option<String>,
}

/// Removes `--db PATH` and `--org SLUG` from `args` (anywhere before a `--`).
pub(crate) fn split_globals(args: &[String]) -> Result<(Globals, Vec<String>), String> {
    let mut globals = Globals::default();
    let mut rest = Vec::with_capacity(args.len());
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let slot = match arg.as_str() {
            "--" => {
                rest.push(arg.clone());
                rest.extend(iter.by_ref().cloned());
                break;
            }
            "--db" => &mut globals.db,
            "--org" => &mut globals.org,
            _ => {
                rest.push(arg.clone());
                continue;
            }
        };
        match iter.next() {
            Some(value) if slot.is_none() => *slot = Some(value.clone()),
            _ => return Err(arg.clone()),
        }
    }
    Ok((globals, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn options_flags_and_words_are_separated() {
        let p = parse(
            &v(&["--name", "a", "--save", "word"]),
            &["--name"],
            &["--save"],
        );
        let Ok(p) = p else { unreachable!("valid") };
        assert_eq!(p.option("--name"), Some("a"));
        assert!(p.flag("--save"));
        assert_eq!(p.positional, ["word"]);
    }

    #[test]
    fn unknown_repeated_and_dangling_options_are_errors_without_their_values() {
        assert_eq!(
            parse(&v(&["--key", "sekrit"]), &[], &[]).err().as_deref(),
            Some("--key")
        );
        assert!(parse(&v(&["--n", "a", "--n", "b"]), &["--n"], &[]).is_err());
        assert!(parse(&v(&["--n"]), &["--n"], &[]).is_err());
        assert!(parse(&v(&["--f", "--f"]), &[], &["--f"]).is_err());
    }

    #[test]
    fn double_dash_ends_options() {
        let p = parse(&v(&["--", "--not-an-option"]), &[], &[]);
        let Ok(p) = p else { unreachable!("valid") };
        assert_eq!(p.positional, ["--not-an-option"]);
    }

    #[test]
    fn globals_are_taken_out_anywhere() {
        let r = split_globals(&v(&["ask", "--db", "x.db", "q", "--org", "acme"]));
        let Ok((g, rest)) = r else {
            unreachable!("valid")
        };
        assert_eq!(g.db.as_deref(), Some("x.db"));
        assert_eq!(g.org.as_deref(), Some("acme"));
        assert_eq!(rest, ["ask", "q"]);
        assert!(split_globals(&v(&["--db"])).is_err());
        assert!(split_globals(&v(&["--db", "a", "--db", "b"])).is_err());
        // After `--` a global-looking word is part of the question.
        let r = split_globals(&v(&["ask", "--", "--db"]));
        let Ok((g, rest)) = r else {
            unreachable!("valid")
        };
        assert!(g.db.is_none());
        assert_eq!(rest, ["ask", "--", "--db"]);
    }
}
