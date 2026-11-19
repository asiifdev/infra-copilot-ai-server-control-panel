//! The set of tools a model may be offered, and the rules for getting into it.

use crate::schema::ArgSchema;
use crate::tool::AiTool;
use core_domain::tier::Tier;
use serde_json::{Value, json};
use std::fmt;

/// A tool as shown to a model (OpenAI-compatible `tools` entry).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolDefinition {
    /// Tool name.
    pub name: &'static str,
    /// Description for the model.
    pub description: &'static str,
    /// JSON Schema of the arguments.
    pub parameters: Value,
}

impl ToolDefinition {
    /// The `{"type":"function","function":{...}}` object of the Chat Completions API.
    #[must_use]
    pub fn to_openai(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            }
        })
    }
}

/// Why a tool was refused at registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// The name is empty, too long or not `[a-z][a-z0-9_]*`.
    BadName(&'static str),
    /// Another tool already has this name.
    Duplicate(&'static str),
    /// The tool's tier is above the registry's ceiling.
    TierAboveCeiling {
        /// Tool name.
        tool: &'static str,
        /// Declared tier.
        tier: Tier,
        /// Ceiling of this registry.
        ceiling: Tier,
    },
    /// A field has a forbidden, malformed or duplicated name, or impossible bounds.
    BadField {
        /// Tool name.
        tool: &'static str,
        /// Offending field.
        field: &'static str,
    },
    /// A read-only tier (0) tool declares an impact, or a tier 1+ tool does not.
    ImpactMismatch(&'static str),
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadName(n) => write!(f, "bad tool name: {n}"),
            Self::Duplicate(n) => write!(f, "duplicate tool: {n}"),
            Self::TierAboveCeiling {
                tool,
                tier,
                ceiling,
            } => write!(
                f,
                "tool {tool} is tier {} but this registry allows at most tier {}",
                tier.level(),
                ceiling.level()
            ),
            Self::BadField { tool, field } => write!(f, "tool {tool} has a bad field: {field}"),
            Self::ImpactMismatch(n) => {
                write!(f, "tool {n}: impact must be set exactly for tier 1+")
            }
        }
    }
}

impl std::error::Error for RegisterError {}

/// Tools by name, with a tier ceiling. M2 registries use [`Tier::Plan`]: a tool that could
/// change a server cannot even be registered, so no prompt and no bug in the loop can reach it.
pub struct Registry {
    ceiling: Tier,
    tools: Vec<Box<dyn AiTool>>,
}

impl Registry {
    /// An empty registry that refuses tools above `ceiling`.
    #[must_use]
    pub fn new(ceiling: Tier) -> Self {
        Self {
            ceiling,
            tools: Vec::new(),
        }
    }

    /// The ceiling.
    #[must_use]
    pub fn ceiling(&self) -> Tier {
        self.ceiling
    }

    /// Adds a tool after checking its declaration.
    ///
    /// # Errors
    /// [`RegisterError`] for a bad name, duplicate, tier above the ceiling, bad field or
    /// impact mismatch.
    pub fn register(&mut self, tool: Box<dyn AiTool>) -> Result<(), RegisterError> {
        let spec = *tool.spec();
        let name = spec.name;
        let name_ok = !name.is_empty()
            && name.len() <= 64
            && name.starts_with(|c: char| c.is_ascii_lowercase())
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !name_ok {
            return Err(RegisterError::BadName(name));
        }
        if self.get(name).is_some() {
            return Err(RegisterError::Duplicate(name));
        }
        if spec.tier > self.ceiling {
            return Err(RegisterError::TierAboveCeiling {
                tool: name,
                tier: spec.tier,
                ceiling: self.ceiling,
            });
        }
        if let Some(field) = spec.schema.first_invalid_field() {
            return Err(RegisterError::BadField { tool: name, field });
        }
        if (spec.tier == Tier::Read) != spec.impact.is_none() {
            return Err(RegisterError::ImpactMismatch(name));
        }
        self.tools.push(tool);
        Ok(())
    }

    /// A tool by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&dyn AiTool> {
        self.tools
            .iter()
            .map(|t| &**t)
            .find(|t| t.spec().name == name)
    }

    /// Number of registered tools.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether no tool is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// What to offer the model, in registration order.
    #[must_use]
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .map(|t| {
                let spec = t.spec();
                ToolDefinition {
                    name: spec.name,
                    description: spec.description,
                    parameters: ArgSchema::to_json(&spec.schema),
                }
            })
            .collect()
    }
}
