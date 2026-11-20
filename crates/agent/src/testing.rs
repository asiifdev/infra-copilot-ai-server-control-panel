//! Shared test support: the contract suite every tool must pass, plus a configurable fake.
//!
//! Plain `pub` on purpose (no feature flag): other crates run [`contract_suite`] on their own
//! tools in their tests (docs/design.md 12.4: one suite, every implementation).

use crate::{
    AiTool, Args, CallStatus, Executor, FindingsView, NodeInfo, ReadStore, Registry, StoreError,
    ToolContext, ToolError, ToolOutput, ToolSpec,
};
use core_domain::tier::Tier;
use db::OrgId;
use i18n::{Catalog, Locale};
use inventory::{Inventory, Severity};

/// A store with nothing in it, for tests that do not read data.
pub struct NullStore;

impl ReadStore for NullStore {
    fn nodes(&self, _org: &OrgId) -> Result<Vec<NodeInfo>, StoreError> {
        Ok(Vec::new())
    }

    fn latest_inventory(&self, _org: &OrgId, _node: &str) -> Result<Inventory, StoreError> {
        Err(StoreError::NotFound)
    }

    fn latest_findings(
        &self,
        _org: &OrgId,
        _min: Option<Severity>,
        _limit: u32,
    ) -> Result<FindingsView, StoreError> {
        Err(StoreError::NotFound)
    }
}

/// Runs `f` with a context over [`NullStore`], the built-in catalog and English.
///
/// # Panics
/// If the built-in catalog is broken (its own tests catch that first).
pub fn with_null_context<R>(f: impl FnOnce(&ToolContext<'_>) -> R) -> R {
    let org = test_org();
    let catalog = match Catalog::builtin() {
        Ok(c) => c,
        Err(_) => unreachable!("built-in catalog is valid"),
    };
    let ctx = ToolContext::new(&org, &NullStore, &catalog, Locale::En);
    f(&ctx)
}

/// A fixed organization id for tests.
///
/// # Panics
/// Never: the constant is a valid id (checked by a unit test below).
#[must_use]
pub fn test_org() -> OrgId {
    match OrgId::parse("00000000-0000-4000-8000-000000000001") {
        Ok(org) => org,
        Err(_) => unreachable!("constant org id is valid"),
    }
}

/// A tool whose declaration and result are chosen by the test.
pub struct FakeTool {
    spec: ToolSpec,
    result: Result<String, ToolError>,
}

impl FakeTool {
    /// A tool that returns `result` for any valid call.
    #[must_use]
    pub fn new(spec: ToolSpec, result: Result<String, ToolError>) -> Self {
        Self { spec, result }
    }
}

impl AiTool for FakeTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn run(&self, _ctx: &ToolContext<'_>, _args: &Args) -> Result<ToolOutput, ToolError> {
        self.result.clone().map(ToolOutput::new)
    }
}

/// What every tool must satisfy, whatever it does. `valid_args` is a call the tool accepts.
///
/// # Panics
/// On the first violated rule, naming it (this is a test helper).
pub fn contract_suite(tool: Box<dyn AiTool>, valid_args: &str) {
    let spec = *tool.spec();
    let mut registry = Registry::new(Tier::Plan);
    assert!(
        registry.register(tool).is_ok(),
        "{}: refused by the registry",
        spec.name
    );
    let definitions = registry.definitions();
    assert_eq!(definitions.len(), 1);
    assert_eq!(
        definitions[0].parameters["additionalProperties"], false,
        "{}",
        spec.name
    );

    let executor = Executor::new(registry);
    with_null_context(|ctx| run_suite(&executor, ctx, spec, valid_args));
}

/// [`contract_suite`] for a tool that is already registered in `executor` (a built-in one, whose
/// type is private), run over a context the caller built, for tools whose valid call needs data.
/// The suite makes exactly one valid call, so a tool that writes writes once.
///
/// # Panics
/// On the first violated rule, naming it, or when `name` is not registered (a test helper).
pub fn contract_suite_for(
    executor: &Executor,
    name: &str,
    valid_args: &str,
    ctx: &ToolContext<'_>,
) {
    let Some(tool) = executor.registry().get(name) else {
        panic!("{name}: not registered");
    };
    let spec = *tool.spec();
    let definition = executor
        .registry()
        .definitions()
        .into_iter()
        .find(|d| d.name == name);
    let Some(definition) = definition else {
        panic!("{name}: no definition offered to the model");
    };
    assert_eq!(
        definition.parameters["additionalProperties"], false,
        "{name}"
    );
    run_suite(executor, ctx, spec, valid_args);
}

fn run_suite(executor: &Executor, ctx: &ToolContext<'_>, spec: ToolSpec, valid_args: &str) {
    let ctx = &ctx;
    let ok = executor.call(ctx, spec.name, valid_args);
    assert_eq!(
        ok.record.status,
        CallStatus::Ok,
        "{}: valid call refused",
        spec.name
    );
    assert_eq!(ok.record.tier, Some(spec.tier));
    assert!(
        ok.content.starts_with("<untrusted_output"),
        "{}: result not fenced",
        spec.name
    );

    // A field the schema does not declare must be refused, including an identity field.
    for extra in [r#"{"zz_unknown":1}"#, r#"{"org_id":"someone-else"}"#] {
        let out = executor.call(ctx, spec.name, extra);
        assert!(
            matches!(out.record.status, CallStatus::BadArguments(_)),
            "{}: accepted {extra}",
            spec.name
        );
    }
    // Garbage must not reach the tool or crash the executor.
    for garbage in ["{", "null", "[]", "\u{0}"] {
        let out = executor.call(ctx, spec.name, garbage);
        assert!(
            matches!(out.record.status, CallStatus::BadArguments(_)),
            "{}: accepted {garbage:?}",
            spec.name
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArgSchema, Field, Kind};

    const READ_SPEC: ToolSpec = ToolSpec {
        name: "echo",
        description: "Echoes.",
        tier: Tier::Read,
        schema: ArgSchema {
            fields: &[Field {
                name: "text",
                kind: Kind::Text {
                    max_chars: 20,
                    choices: None,
                },
                required: true,
                description: "Text.",
            }],
        },
        impact: None,
    };

    #[test]
    fn the_test_org_constant_is_valid() {
        assert_eq!(test_org().as_str().len(), 36);
    }

    #[test]
    fn a_plain_read_tool_passes_the_suite() {
        contract_suite(
            Box::new(FakeTool::new(READ_SPEC, Ok("hello".into()))),
            r#"{"text":"hi"}"#,
        );
    }
}
