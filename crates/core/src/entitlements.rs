//! Open-core entitlements and usage limits interface (`docs/plan.md`).
//!
//! This module defines the public contract for edition tiers, feature gates, and usage quotas.
//! The public repository only ships this interface and [`LocalCommunityEntitlements`]; commercial
//! license verification, pricing, and cloud billing live in the private repository.

use std::fmt;

/// Product edition tier for an organization.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EntitlementTier {
    /// Self-hosted open-core community edition.
    #[default]
    Community,
    /// Self-hosted commercial licensed edition.
    Licensed,
    /// Hosted multi-tenant cloud edition.
    Cloud,
}

impl EntitlementTier {
    /// Canonical lowercase identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Community => "community",
            Self::Licensed => "licensed",
            Self::Cloud => "cloud",
        }
    }
}

/// Effective capability flags and quota caps for an organization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntitlementLimits {
    /// Maximum number of managed nodes allowed, or `None` for unlimited.
    pub max_nodes: Option<u32>,
    /// Maximum number of organizations allowed, or `None` for unlimited.
    pub max_orgs: Option<u32>,
    /// Whether tier-4 (destructive) actions require a second distinct approver.
    pub require_two_approvers_tier4: bool,
    /// Whether managed transport / relay is enabled.
    pub allow_managed_transport: bool,
}

impl EntitlementLimits {
    /// Default limits for the self-hosted open-core community edition.
    #[must_use]
    pub const fn community() -> Self {
        Self {
            max_nodes: None,
            max_orgs: Some(1),
            require_two_approvers_tier4: false,
            allow_managed_transport: false,
        }
    }
}

impl Default for EntitlementLimits {
    fn default() -> Self {
        Self::community()
    }
}

/// Metered usage dimensions checked against an organization's entitlement limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UsageMetric {
    /// Managed server nodes.
    Nodes,
    /// AI assistant messages in the current quota window.
    AiMessages,
    /// Server-to-server migrations.
    Migrations,
}

impl UsageMetric {
    /// Canonical lowercase identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nodes => "nodes",
            Self::AiMessages => "ai_messages",
            Self::Migrations => "migrations",
        }
    }
}

/// Why an entitlement or usage limit check failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntitlementError {
    /// Consuming `requested` additional units would exceed the configured `limit`.
    LimitExceeded {
        /// The metric or resource whose cap was reached.
        metric: &'static str,
        /// Configured maximum.
        limit: u32,
        /// Current usage before the requested operation.
        current: u32,
        /// Additional units requested.
        requested: u32,
    },
    /// The requested capability is not enabled on the current entitlement tier.
    FeatureNotAllowed(&'static str),
}

impl fmt::Display for EntitlementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded {
                metric,
                limit,
                current,
                requested,
            } => write!(
                f,
                "entitlement limit exceeded for {metric}: current={current}, requested={requested}, limit={limit}"
            ),
            Self::FeatureNotAllowed(feature) => {
                write!(f, "entitlement feature not allowed: {feature}")
            }
        }
    }
}

impl std::error::Error for EntitlementError {}

/// Provides the active entitlement tier and limits for an organization.
pub trait EntitlementProvider: Send + Sync {
    /// Returns the active entitlement tier for `org_id`.
    fn tier(&self, org_id: &str) -> EntitlementTier;

    /// Returns the effective limits for `org_id`.
    fn limits(&self, org_id: &str) -> EntitlementLimits;
}

/// Enforces usage quotas and feature gates for an organization.
pub trait LimitsService: Send + Sync {
    /// Checks whether `org_id` may consume `additional` units of `metric` given `current` usage.
    ///
    /// # Errors
    /// Returns [`EntitlementError::LimitExceeded`] when `current + additional` exceeds the limit.
    fn check_quota(
        &self,
        org_id: &str,
        metric: UsageMetric,
        current: u32,
        additional: u32,
    ) -> Result<(), EntitlementError>;

    /// Checks whether `org_id` may create another organization given `current_orgs`.
    ///
    /// # Errors
    /// Returns [`EntitlementError::LimitExceeded`] when `current_orgs + 1` exceeds `max_orgs`.
    fn check_can_create_org(&self, org_id: &str, current_orgs: u32)
    -> Result<(), EntitlementError>;

    /// Whether tier-4 (destructive) actions in `org_id` require a second distinct approver.
    fn requires_two_approvers_tier4(&self, org_id: &str) -> bool;

    /// Whether managed transport / relay is allowed for `org_id`.
    fn allow_managed_transport(&self, org_id: &str) -> bool;
}

fn check_cap(
    metric: &'static str,
    cap: Option<u32>,
    current: u32,
    additional: u32,
) -> Result<(), EntitlementError> {
    let Some(limit) = cap else {
        return Ok(());
    };
    let next = current.saturating_add(additional);
    if next > limit {
        return Err(EntitlementError::LimitExceeded {
            metric,
            limit,
            current,
            requested: additional,
        });
    }
    Ok(())
}

/// Evaluates [`LimitsService`] checks against any [`EntitlementProvider`].
#[derive(Clone, Copy, Debug)]
pub struct ProviderLimitsService<P>(pub P);

impl<P: EntitlementProvider> LimitsService for ProviderLimitsService<P> {
    fn check_quota(
        &self,
        org_id: &str,
        metric: UsageMetric,
        current: u32,
        additional: u32,
    ) -> Result<(), EntitlementError> {
        let limits = self.0.limits(org_id);
        let cap = match metric {
            UsageMetric::Nodes => limits.max_nodes,
            UsageMetric::AiMessages | UsageMetric::Migrations => None,
        };
        check_cap(metric.as_str(), cap, current, additional)
    }

    fn check_can_create_org(
        &self,
        org_id: &str,
        current_orgs: u32,
    ) -> Result<(), EntitlementError> {
        let limits = self.0.limits(org_id);
        check_cap("orgs", limits.max_orgs, current_orgs, 1)
    }

    fn requires_two_approvers_tier4(&self, org_id: &str) -> bool {
        self.0.limits(org_id).require_two_approvers_tier4
    }

    fn allow_managed_transport(&self, org_id: &str) -> bool {
        self.0.limits(org_id).allow_managed_transport
    }
}

/// Default open-core implementation for self-hosted community deployments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalCommunityEntitlements;

impl EntitlementProvider for LocalCommunityEntitlements {
    fn tier(&self, _org_id: &str) -> EntitlementTier {
        EntitlementTier::Community
    }

    fn limits(&self, _org_id: &str) -> EntitlementLimits {
        EntitlementLimits::community()
    }
}

impl LimitsService for LocalCommunityEntitlements {
    fn check_quota(
        &self,
        org_id: &str,
        metric: UsageMetric,
        current: u32,
        additional: u32,
    ) -> Result<(), EntitlementError> {
        ProviderLimitsService(*self).check_quota(org_id, metric, current, additional)
    }

    fn check_can_create_org(
        &self,
        org_id: &str,
        current_orgs: u32,
    ) -> Result<(), EntitlementError> {
        ProviderLimitsService(*self).check_can_create_org(org_id, current_orgs)
    }

    fn requires_two_approvers_tier4(&self, org_id: &str) -> bool {
        ProviderLimitsService(*self).requires_two_approvers_tier4(org_id)
    }

    fn allow_managed_transport(&self, org_id: &str) -> bool {
        ProviderLimitsService(*self).allow_managed_transport(org_id)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EntitlementError, EntitlementLimits, EntitlementProvider, EntitlementTier, LimitsService,
        LocalCommunityEntitlements, ProviderLimitsService, UsageMetric,
    };

    struct CustomProvider {
        tier: EntitlementTier,
        limits: EntitlementLimits,
    }

    impl EntitlementProvider for CustomProvider {
        fn tier(&self, _org_id: &str) -> EntitlementTier {
            self.tier
        }

        fn limits(&self, _org_id: &str) -> EntitlementLimits {
            self.limits
        }
    }

    #[test]
    fn community_defaults_allow_nodes_and_cap_orgs_at_one() {
        let entitlements = LocalCommunityEntitlements;
        assert_eq!(entitlements.tier("org_1"), EntitlementTier::Community);

        let limits = entitlements.limits("org_1");
        assert_eq!(limits.max_nodes, None);
        assert_eq!(limits.max_orgs, Some(1));
        assert!(!limits.require_two_approvers_tier4);
        assert!(!limits.allow_managed_transport);

        assert!(
            entitlements
                .check_quota("org_1", UsageMetric::Nodes, 100, 50)
                .is_ok()
        );
        assert!(
            entitlements
                .check_quota("org_1", UsageMetric::AiMessages, 10, 1)
                .is_ok()
        );
        assert!(
            entitlements
                .check_quota("org_1", UsageMetric::Migrations, 5, 1)
                .is_ok()
        );
        assert!(entitlements.check_can_create_org("org_1", 0).is_ok());
        assert_eq!(
            entitlements.check_can_create_org("org_1", 1),
            Err(EntitlementError::LimitExceeded {
                metric: "orgs",
                limit: 1,
                current: 1,
                requested: 1,
            })
        );
        assert!(!entitlements.requires_two_approvers_tier4("org_1"));
        assert!(!entitlements.allow_managed_transport("org_1"));
    }

    #[test]
    fn custom_limits_enforce_node_cap_and_cloud_gates() {
        let provider = CustomProvider {
            tier: EntitlementTier::Cloud,
            limits: EntitlementLimits {
                max_nodes: Some(5),
                max_orgs: Some(3),
                require_two_approvers_tier4: true,
                allow_managed_transport: true,
            },
        };
        let svc = ProviderLimitsService(provider);

        assert_eq!(svc.0.tier("org_cloud"), EntitlementTier::Cloud);
        assert!(svc.requires_two_approvers_tier4("org_cloud"));
        assert!(svc.allow_managed_transport("org_cloud"));

        assert!(
            svc.check_quota("org_cloud", UsageMetric::Nodes, 3, 2)
                .is_ok()
        );
        assert_eq!(
            svc.check_quota("org_cloud", UsageMetric::Nodes, 5, 1),
            Err(EntitlementError::LimitExceeded {
                metric: "nodes",
                limit: 5,
                current: 5,
                requested: 1,
            })
        );
        // Saturating addition handles u32::MAX without overflow.
        assert_eq!(
            svc.check_quota("org_cloud", UsageMetric::Nodes, u32::MAX, 1),
            Err(EntitlementError::LimitExceeded {
                metric: "nodes",
                limit: 5,
                current: u32::MAX,
                requested: 1,
            })
        );
        assert!(svc.check_can_create_org("org_cloud", 2).is_ok());
        assert!(svc.check_can_create_org("org_cloud", 3).is_err());
    }
}
