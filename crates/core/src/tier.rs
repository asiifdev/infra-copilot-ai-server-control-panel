//! Risk tiers for actions and AI tools (docs/plan.md section 5, docs/design.md section 9).
//!
//! The tier of an action is decided by the code of the tool that performs it. Neither the AI nor
//! the user can lower it.

/// Risk level of an action, from harmless to destructive. The order is meaningful.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// 0 - read only (list services, read logs, collect inventory).
    Read = 0,
    /// 1 - produces a plan or a proposal; changes nothing.
    Plan = 1,
    /// 2 - light change (restart a non-critical app).
    LightChange = 2,
    /// 3 - risky change (edit `.env`, change DNS, restore a database).
    RiskyChange = 3,
    /// 4 - destructive (delete data, `DROP`, shut a server down, delete backups).
    Destructive = 4,
}

impl Tier {
    /// All tiers in ascending order.
    pub const ALL: [Tier; 5] = [
        Tier::Read,
        Tier::Plan,
        Tier::LightChange,
        Tier::RiskyChange,
        Tier::Destructive,
    ];

    /// Numeric level, 0 to 4.
    #[must_use]
    pub const fn level(self) -> u8 {
        self as u8
    }

    /// Tier for a numeric level, or `None` when out of range.
    #[must_use]
    pub const fn from_level(level: u8) -> Option<Tier> {
        match level {
            0 => Some(Tier::Read),
            1 => Some(Tier::Plan),
            2 => Some(Tier::LightChange),
            3 => Some(Tier::RiskyChange),
            4 => Some(Tier::Destructive),
            _ => None,
        }
    }

    /// May run without a human decision. Only tiers 0 and 1.
    #[must_use]
    pub const fn runs_automatically(self) -> bool {
        self.level() <= Tier::Plan.level()
    }

    /// Needs a stored proposal that a human approves before it runs. Tiers 2 and above.
    #[must_use]
    pub const fn requires_proposal(self) -> bool {
        self.level() >= Tier::LightChange.level()
    }

    /// Needs a dry run and an automatic backup first. Tiers 3 and 4.
    #[must_use]
    pub const fn requires_dry_run_and_backup(self) -> bool {
        self.level() >= Tier::RiskyChange.level()
    }

    /// The approver must type the name of the target. Tier 4 only.
    #[must_use]
    pub const fn requires_typed_confirmation(self) -> bool {
        matches!(self, Tier::Destructive)
    }

    /// In the cloud edition a second, different person must also approve. Tier 4 only.
    #[must_use]
    pub const fn requires_second_approver_in_cloud(self) -> bool {
        matches!(self, Tier::Destructive)
    }

    /// May be approved from a chat bot (tiers 0 to 2). Tiers 3 and 4 only in the panel; the bot
    /// merely sends a link to the proposal.
    #[must_use]
    pub const fn approvable_from_chat_bot(self) -> bool {
        self.level() <= Tier::LightChange.level()
    }
}

#[cfg(test)]
mod tests {
    use super::Tier;

    #[test]
    fn levels_round_trip() {
        for tier in Tier::ALL {
            assert_eq!(Tier::from_level(tier.level()), Some(tier));
        }
        assert_eq!(Tier::from_level(5), None);
        assert_eq!(Tier::from_level(255), None);
    }

    #[test]
    fn order_follows_risk() {
        assert!(Tier::Read < Tier::Plan);
        assert!(Tier::Plan < Tier::LightChange);
        assert!(Tier::LightChange < Tier::RiskyChange);
        assert!(Tier::RiskyChange < Tier::Destructive);
    }

    #[test]
    fn only_read_and_plan_run_automatically() {
        let auto: Vec<Tier> = Tier::ALL
            .into_iter()
            .filter(|t| t.runs_automatically())
            .collect();
        assert_eq!(auto, vec![Tier::Read, Tier::Plan]);
    }

    #[test]
    fn proposals_start_at_light_change() {
        assert!(!Tier::Plan.requires_proposal());
        assert!(Tier::LightChange.requires_proposal());
        assert!(Tier::RiskyChange.requires_proposal());
        assert!(Tier::Destructive.requires_proposal());
    }

    #[test]
    fn dry_run_and_backup_from_risky_change() {
        assert!(!Tier::LightChange.requires_dry_run_and_backup());
        assert!(Tier::RiskyChange.requires_dry_run_and_backup());
        assert!(Tier::Destructive.requires_dry_run_and_backup());
    }

    #[test]
    fn destructive_alone_needs_typed_confirmation_and_second_approver() {
        for tier in Tier::ALL {
            let destructive = tier == Tier::Destructive;
            assert_eq!(tier.requires_typed_confirmation(), destructive);
            assert_eq!(tier.requires_second_approver_in_cloud(), destructive);
        }
    }

    #[test]
    fn chat_bot_cannot_approve_tier_three_or_four() {
        assert!(Tier::Read.approvable_from_chat_bot());
        assert!(Tier::LightChange.approvable_from_chat_bot());
        assert!(!Tier::RiskyChange.approvable_from_chat_bot());
        assert!(!Tier::Destructive.approvable_from_chat_bot());
    }

    #[test]
    fn nothing_that_needs_a_proposal_runs_automatically() {
        for tier in Tier::ALL {
            assert!(!(tier.requires_proposal() && tier.runs_automatically()));
        }
    }
}
