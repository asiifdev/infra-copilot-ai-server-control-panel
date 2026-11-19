//! A validated migration plan: a spec plus an ordered, acyclic set of steps.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::error::PlanError;
use crate::gates::GateReport;
use crate::spec::{DbMode, MAX_JSON_BYTES, PlanSpec, check_text};
use crate::steps::{Side, Step, StepKind};

/// Most steps a plan may have.
pub const MAX_STEPS: usize = 128;
/// Longest plan name, in bytes.
const MAX_NAME_LEN: usize = 200;
/// Longest database key on a step, in bytes (same bound as in the spec).
const MAX_DB_KEY_LEN: usize = 200;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlan {
    name: String,
    spec: PlanSpec,
    steps: Vec<Step>,
}

/// A plan that passed every rule. It can only be built through [`Plan::new`] or parsed from JSON,
/// both of which validate, so holding a `Plan` means the order and the safety rules hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawPlan")]
pub struct Plan {
    name: String,
    spec: PlanSpec,
    steps: Vec<Step>,
}

impl TryFrom<RawPlan> for Plan {
    type Error = PlanError;

    fn try_from(raw: RawPlan) -> Result<Self, PlanError> {
        Self::new(raw.name, raw.spec, raw.steps)
    }
}

/// Why a step may not be scheduled yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleRefusal {
    /// No step has this number.
    UnknownStep,
    /// The step is already done.
    AlreadyCompleted,
    /// These steps must finish first.
    DependenciesPending(Vec<u32>),
    /// Cutover and decommissioning need a gate report; none was given (no comparison ran).
    GatesUnknown,
    /// A critical finding is open, or the evidence is incomplete.
    GatesNotSatisfied,
    /// The old server may only be shut down after every other step, post-verification included,
    /// has finished.
    SourceNotVerified,
}

/// One stored step, as the database layer hands it back (opaque JSON text and plain numbers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredStep {
    /// Step number.
    pub seq: u32,
    /// Wire name of the kind.
    pub kind: String,
    /// The step document (`{"side":..,"database":..}`, `{}` when it has neither).
    pub spec_json: String,
    /// Dependencies.
    pub depends_on: Vec<u32>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct StepSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    side: Option<Side>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    database: Option<String>,
}

impl Plan {
    /// Builds a plan and checks every rule: spec limits and safety, step numbers, dependencies,
    /// cycles, required kinds, the order of the kinds, sides and databases.
    ///
    /// # Errors
    /// The first rule that is broken, as a [`PlanError`].
    pub fn new(name: String, spec: PlanSpec, steps: Vec<Step>) -> Result<Self, PlanError> {
        check_text(&name, "name", MAX_NAME_LEN)?;
        spec.validate()?;
        validate_steps(&spec, &steps)?;
        Ok(Self { name, spec, steps })
    }

    /// Parses and validates a plan document `{"name", "spec", "steps"}`.
    ///
    /// # Errors
    /// [`PlanError::TooLarge`], [`PlanError::Parse`] or any rule of [`Plan::new`].
    pub fn from_json(json: &str) -> Result<Self, PlanError> {
        if json.len() > MAX_JSON_BYTES {
            return Err(PlanError::TooLarge {
                what: "plan document",
                max: MAX_JSON_BYTES,
            });
        }
        serde_json::from_str(json).map_err(|err| PlanError::Parse(err.to_string()))
    }

    /// Serializes to compact JSON.
    ///
    /// # Errors
    /// Only if serialization fails, which cannot happen for these types.
    pub fn to_json(&self) -> Result<String, PlanError> {
        serde_json::to_string(self).map_err(|err| PlanError::Parse(err.to_string()))
    }

    /// The plan name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The spec.
    #[must_use]
    pub fn spec(&self) -> &PlanSpec {
        &self.spec
    }

    /// The steps, in the order they were given.
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// One step by number.
    #[must_use]
    pub fn step(&self, seq: u32) -> Option<&Step> {
        self.steps.iter().find(|s| s.seq == seq)
    }

    /// Steps in an order that respects the dependencies (ties by number).
    #[must_use]
    pub fn execution_order(&self) -> Vec<&Step> {
        // Validation guarantees acyclicity, so this always visits every step.
        let mut done: BTreeSet<u32> = BTreeSet::new();
        let mut order = Vec::with_capacity(self.steps.len());
        while order.len() < self.steps.len() {
            let next = self
                .steps
                .iter()
                .filter(|s| !done.contains(&s.seq) && s.depends_on.iter().all(|d| done.contains(d)))
                .min_by_key(|s| s.seq);
            let Some(step) = next else { break };
            done.insert(step.seq);
            order.push(step);
        }
        order
    }

    /// The spec as JSON text for `migration_plans.spec`.
    ///
    /// # Errors
    /// Only if serialization fails, which cannot happen for these types.
    pub fn spec_json(&self) -> Result<String, PlanError> {
        self.spec.to_json()
    }

    /// The steps as plain records for `migration_steps` (the database layer never sees this crate).
    ///
    /// # Errors
    /// Only if serialization fails, which cannot happen for these types.
    pub fn stored_steps(&self) -> Result<Vec<StoredStep>, PlanError> {
        self.steps
            .iter()
            .map(|step| {
                let spec = StepSpec {
                    side: step.side,
                    database: step.database.clone(),
                };
                Ok(StoredStep {
                    seq: step.seq,
                    kind: step.kind.as_str().to_owned(),
                    spec_json: serde_json::to_string(&spec)
                        .map_err(|err| PlanError::Parse(err.to_string()))?,
                    depends_on: step.depends_on.clone(),
                })
            })
            .collect()
    }

    /// Rebuilds and re-validates a plan from stored parts. Stored data is not trusted: it goes
    /// through the same rules as a new plan. `source_node_id` and `target_node_id` are the row
    /// columns (the ones the database checked against the org); the spec must agree with them.
    ///
    /// # Errors
    /// [`PlanError::NodeMismatch`] when the spec names other nodes than the row columns.
    /// [`PlanError::Parse`] for a bad step document, [`PlanError::UnknownKind`] and any rule of
    /// [`Plan::new`].
    pub fn from_stored(
        name: &str,
        source_node_id: &str,
        target_node_id: &str,
        spec_json: &str,
        stored: &[StoredStep],
    ) -> Result<Self, PlanError> {
        if stored.len() > MAX_STEPS {
            return Err(PlanError::TooLarge {
                what: "steps",
                max: MAX_STEPS,
            });
        }
        let spec = PlanSpec::from_json(spec_json)?;
        if spec.source.node_id != source_node_id || spec.target.node_id != target_node_id {
            return Err(PlanError::NodeMismatch);
        }
        let mut steps = Vec::with_capacity(stored.len());
        for record in stored {
            if record.spec_json.len() > MAX_JSON_BYTES {
                return Err(PlanError::TooLarge {
                    what: "step document",
                    max: MAX_JSON_BYTES,
                });
            }
            let doc: StepSpec = serde_json::from_str(&record.spec_json)
                .map_err(|err| PlanError::Parse(err.to_string()))?;
            steps.push(Step {
                seq: record.seq,
                kind: StepKind::parse(&record.kind)?,
                depends_on: record.depends_on.clone(),
                side: doc.side,
                database: doc.database,
            });
        }
        Self::new(name.to_owned(), spec, steps)
    }

    /// Whether step `seq` may be scheduled now. `completed` holds the finished steps; `gates` is
    /// the current gate report, or `None` when no comparison has run.
    ///
    /// * every dependency must be completed;
    /// * `cutover` needs a gate report with no open critical finding and complete evidence;
    /// * `decommission_source` (tier 4) needs the same, **and** every other step finished, so the
    ///   old server is never shut down before the target is verified after cutover.
    ///
    /// # Errors
    /// The [`ScheduleRefusal`] explaining why not.
    pub fn check_schedulable(
        &self,
        seq: u32,
        completed: &BTreeSet<u32>,
        gates: Option<&GateReport>,
    ) -> Result<(), ScheduleRefusal> {
        let step = self.step(seq).ok_or(ScheduleRefusal::UnknownStep)?;
        if completed.contains(&seq) {
            return Err(ScheduleRefusal::AlreadyCompleted);
        }
        let pending: Vec<u32> = step
            .depends_on
            .iter()
            .copied()
            .filter(|d| !completed.contains(d))
            .collect();
        if !pending.is_empty() {
            return Err(ScheduleRefusal::DependenciesPending(pending));
        }
        if matches!(step.kind, StepKind::Cutover | StepKind::DecommissionSource) {
            let gates = gates.ok_or(ScheduleRefusal::GatesUnknown)?;
            if !gates.cutover_allowed() {
                return Err(ScheduleRefusal::GatesNotSatisfied);
            }
        }
        if step.kind == StepKind::DecommissionSource {
            let all_others_done = self
                .steps
                .iter()
                .filter(|s| s.seq != seq)
                .all(|s| completed.contains(&s.seq));
            if !all_others_done {
                return Err(ScheduleRefusal::SourceNotVerified);
            }
        }
        Ok(())
    }
}

fn validate_steps(spec: &PlanSpec, steps: &[Step]) -> Result<(), PlanError> {
    if steps.is_empty() {
        return Err(PlanError::NoSteps);
    }
    if steps.len() > MAX_STEPS {
        return Err(PlanError::TooLarge {
            what: "steps",
            max: MAX_STEPS,
        });
    }
    let mut index: BTreeMap<u32, usize> = BTreeMap::new();
    for (i, step) in steps.iter().enumerate() {
        if index.insert(step.seq, i).is_some() {
            return Err(PlanError::DuplicateSeq(step.seq));
        }
    }
    let mut parents: Vec<Vec<usize>> = Vec::with_capacity(steps.len());
    for step in steps {
        if step.depends_on.len() > MAX_STEPS {
            return Err(PlanError::TooLarge {
                what: "depends_on",
                max: MAX_STEPS,
            });
        }
        let mut seen = BTreeSet::new();
        let mut list = Vec::with_capacity(step.depends_on.len());
        for dep in &step.depends_on {
            if *dep == step.seq {
                return Err(PlanError::SelfDependency(step.seq));
            }
            if !seen.insert(*dep) {
                return Err(PlanError::DuplicateDependency(step.seq));
            }
            let Some(&at) = index.get(dep) else {
                return Err(PlanError::UnknownDependency {
                    step: step.seq,
                    missing: *dep,
                });
            };
            list.push(at);
        }
        parents.push(list);
    }
    let ancestors = ancestors_of(&parents)?;

    for kind in StepKind::ALL {
        let count = steps.iter().filter(|s| s.kind == kind).count();
        if kind.required() && count == 0 {
            return Err(PlanError::MissingKind(kind));
        }
        if kind.single() && count > 1 {
            return Err(PlanError::TooMany(kind));
        }
    }

    // Order: a step of a later kind must (transitively) depend on every step of an earlier kind,
    // and never the other way round. Reversed edges are reported first because they are the
    // clearer mistake.
    for (a, step_a) in steps.iter().enumerate() {
        for (b, step_b) in steps.iter().enumerate() {
            if step_a.kind.rank() > step_b.kind.rank() && ancestors[b].contains(&a) {
                return Err(PlanError::OrderReversed {
                    earlier: step_b.kind,
                    later: step_a.kind,
                });
            }
        }
    }
    for (a, step_a) in steps.iter().enumerate() {
        for (b, step_b) in steps.iter().enumerate() {
            if step_a.kind.rank() < step_b.kind.rank() && !ancestors[b].contains(&a) {
                return Err(PlanError::OrderMissing {
                    earlier: step_a.kind,
                    later: step_b.kind,
                });
            }
        }
    }

    for step in steps {
        match (step.kind, step.side) {
            (StepKind::Inventory, None) => return Err(PlanError::SideRequired(step.seq)),
            (StepKind::Inventory, Some(_)) => {}
            (_, Some(_)) => return Err(PlanError::SideNotAllowed(step.seq)),
            (_, None) => {}
        }
        if let Some(key) = &step.database {
            if !matches!(step.kind, StepKind::Restore | StepKind::Verify) {
                return Err(PlanError::DatabaseNotAllowed(step.seq));
            }
            check_text(key, "step.database", MAX_DB_KEY_LEN)?;
            match spec.database_mode(key) {
                None => return Err(PlanError::UnknownDatabase(step.seq)),
                Some(DbMode::Skip) if step.kind == StepKind::Restore => {
                    return Err(PlanError::SkippedDatabaseRestored(step.seq));
                }
                Some(_) => {}
            }
        }
    }
    if !steps
        .iter()
        .any(|s| s.kind == StepKind::Inventory && s.side == Some(Side::Source))
    {
        return Err(PlanError::NoSourceInventory);
    }
    Ok(())
}

/// For each step, the set of all steps it transitively depends on. Fails on a cycle (Kahn).
fn ancestors_of(parents: &[Vec<usize>]) -> Result<Vec<BTreeSet<usize>>, PlanError> {
    let n = parents.len();
    let mut remaining: Vec<usize> = parents.iter().map(Vec::len).collect();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (child, list) in parents.iter().enumerate() {
        for &parent in list {
            if let Some(slot) = children.get_mut(parent) {
                slot.push(child);
            }
        }
    }
    let mut ready: Vec<usize> = (0..n).filter(|&i| remaining.get(i) == Some(&0)).collect();
    let mut ancestors: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    let mut visited = 0;
    while let Some(node) = ready.pop() {
        visited += 1;
        let mine = ancestors.get(node).cloned().unwrap_or_default();
        for &child in children.get(node).map_or(&[][..], Vec::as_slice) {
            if let Some(set) = ancestors.get_mut(child) {
                set.insert(node);
                set.extend(mine.iter().copied());
            }
            if let Some(left) = remaining.get_mut(child) {
                *left -= 1;
                if *left == 0 {
                    ready.push(child);
                }
            }
        }
    }
    if visited == n {
        Ok(ancestors)
    } else {
        Err(PlanError::Cycle)
    }
}
