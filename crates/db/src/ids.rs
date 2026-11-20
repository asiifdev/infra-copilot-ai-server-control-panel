//! Typed identifiers. Every id is a lowercase UUID string; the types stop an org id from being
//! passed where a node id is expected.

use crate::error::{Error, Result};

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            /// Parses an externally supplied id (lowercase hyphenated UUID).
            ///
            /// # Errors
            /// [`Error::Invalid`] when the text is not a lowercase UUID.
            pub fn parse(text: &str) -> Result<Self> {
                if is_uuid(text) {
                    Ok(Self(text.to_owned()))
                } else {
                    Err(Error::Invalid(stringify!($name)))
                }
            }

            /// The id as text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            // Ids read back from the database were validated by its CHECK and by us on write.
            pub(crate) fn from_db(text: String) -> Self {
                Self(text)
            }
        }
    };
}

id_type!(
    /// Tenant identifier. Required by every query on org-owned data.
    OrgId
);
id_type!(
    /// Global user identifier.
    UserId
);
id_type!(
    /// Membership identifier.
    MembershipId
);
id_type!(
    /// API token identifier.
    ApiTokenId
);
id_type!(
    /// Node identifier.
    NodeId
);
id_type!(
    /// Agent identifier.
    AgentId
);
id_type!(
    /// Node secret identifier.
    SecretId
);
id_type!(
    /// Inventory (snapshot) identifier.
    InventoryId
);
id_type!(
    /// Comparison identifier.
    ComparisonId
);
id_type!(
    /// AI provider identifier.
    AiProviderId
);
id_type!(
    /// Assistant conversation identifier.
    ConversationId
);
id_type!(
    /// Proposal identifier.
    ProposalId
);
id_type!(
    /// Transport identifier.
    TransportId
);
id_type!(
    /// Audit entry identifier.
    AuditId
);
id_type!(
    /// Migration plan identifier.
    PlanId
);

id_type!(
    /// Memory identifier.
    MemoryId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_lowercase_uuids() {
        assert!(OrgId::parse("123e4567-e89b-42d3-a456-426614174000").is_ok());
        assert!(OrgId::parse("123E4567-e89b-42d3-a456-426614174000").is_err());
        assert!(OrgId::parse("not-a-uuid").is_err());
        assert!(OrgId::parse("").is_err());
        assert!(OrgId::parse("123e4567e89b42d3a456426614174000----").is_err());
    }
}
