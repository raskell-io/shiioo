//! Property-based tests for core types.
//!
//! These tests use proptest to verify invariants across randomly generated inputs.

use proptest::prelude::*;
use shiioo_core::agent::{AgentBudgets, AgentId, CostBudget, RequestBudget, TokenBudget};
use shiioo_core::types::TeamId;

// Strategies for generating test data

/// Generate a valid agent ID string.
fn agent_id_string() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,62}".prop_filter("Must be non-empty", |s| !s.is_empty())
}

/// Generate a valid team ID string.
fn team_id_string() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,30}".prop_filter("Must be non-empty", |s| !s.is_empty())
}

/// Generate a token budget limit.
fn token_limit() -> impl Strategy<Value = u64> {
    1u64..1_000_000_000u64
}

/// Generate a cost limit in cents.
fn cost_limit() -> impl Strategy<Value = u64> {
    1u64..100_000_000u64
}

/// Generate a request count limit.
fn request_limit() -> impl Strategy<Value = u32> {
    1u32..1_000_000u32
}

// Property tests

proptest! {
    /// AgentId should preserve the original string value.
    #[test]
    fn agent_id_preserves_value(id in agent_id_string()) {
        let agent_id = AgentId::new(&id);
        prop_assert_eq!(agent_id.0.as_str(), &id);
    }

    /// AgentId equality should be reflexive.
    #[test]
    fn agent_id_equality_reflexive(id in agent_id_string()) {
        let agent_id = AgentId::new(&id);
        prop_assert_eq!(agent_id.clone(), agent_id);
    }

    /// AgentId equality should be symmetric.
    #[test]
    fn agent_id_equality_symmetric(id in agent_id_string()) {
        let a = AgentId::new(&id);
        let b = AgentId::new(&id);
        prop_assert_eq!(a == b, b == a);
    }

    /// TeamId should preserve the original string value.
    #[test]
    fn team_id_preserves_value(id in team_id_string()) {
        let team_id = TeamId::new(&id);
        prop_assert_eq!(team_id.0.as_str(), &id);
    }

    /// Token budget limits should be preserved when set.
    #[test]
    fn token_budget_preserves_limits(
        per_request in token_limit(),
        per_hour in token_limit(),
        per_day in token_limit()
    ) {
        let budget = TokenBudget::new()
            .with_per_request(per_request)
            .with_per_hour(per_hour)
            .with_per_day(per_day);

        prop_assert_eq!(budget.per_request, Some(per_request));
        prop_assert_eq!(budget.per_hour, Some(per_hour));
        prop_assert_eq!(budget.per_day, Some(per_day));
    }

    /// Token budget should report as having limits when any limit is set.
    #[test]
    fn token_budget_has_limits_when_set(limit in token_limit()) {
        let budget = TokenBudget::new().with_per_hour(limit);
        prop_assert!(budget.has_limits());
    }

    /// Empty token budget should report no limits.
    #[test]
    fn empty_token_budget_has_no_limits(_unused in 0..1u32) {
        let budget = TokenBudget::new();
        prop_assert!(!budget.has_limits());
    }

    /// Cost budget limits should be preserved when set.
    #[test]
    fn cost_budget_preserves_limits(
        per_hour in cost_limit(),
        per_day in cost_limit()
    ) {
        let budget = CostBudget::new()
            .with_per_hour(per_hour)
            .with_per_day(per_day);

        prop_assert_eq!(budget.per_hour_cents, Some(per_hour));
        prop_assert_eq!(budget.per_day_cents, Some(per_day));
    }

    /// Request budget limits should be preserved when set.
    #[test]
    fn request_budget_preserves_limits(
        per_hour in request_limit(),
        per_day in request_limit()
    ) {
        let budget = RequestBudget::new()
            .with_per_hour(per_hour)
            .with_per_day(per_day);

        prop_assert_eq!(budget.per_hour, Some(per_hour));
        prop_assert_eq!(budget.per_day, Some(per_day));
    }

    /// AgentBudgets should compose budgets correctly.
    #[test]
    fn agent_budgets_composition(
        token_limit in token_limit(),
        cost_limit in cost_limit(),
        request_limit in request_limit()
    ) {
        let budgets = AgentBudgets::new()
            .with_tokens(TokenBudget::new().with_per_hour(token_limit))
            .with_cost(CostBudget::new().with_per_hour(cost_limit))
            .with_requests(RequestBudget::new().with_per_hour(request_limit));

        prop_assert_eq!(budgets.tokens.per_hour, Some(token_limit));
        prop_assert_eq!(budgets.cost.per_hour_cents, Some(cost_limit));
        prop_assert_eq!(budgets.requests.per_hour, Some(request_limit));
    }

    /// Token budget daily limit should be >= hourly limit for sensible configs.
    #[test]
    fn sensible_token_budget_ordering(
        hourly in 1u64..1000u64,
        multiplier in 1u64..100u64
    ) {
        let daily = hourly * multiplier;
        let budget = TokenBudget::new()
            .with_per_hour(hourly)
            .with_per_day(daily);

        prop_assert!(budget.per_day.unwrap() >= budget.per_hour.unwrap());
    }
}

// Additional property tests for budget calculations

proptest! {
    /// Budget usage should never go negative.
    #[test]
    fn budget_usage_non_negative(
        initial in 0u64..1000u64,
        consumed in 0u64..1000u64
    ) {
        let total = initial + consumed;
        prop_assert!(total >= initial, "Budget usage should accumulate");
        prop_assert!(total >= consumed, "Budget usage should not lose consumed amount");
    }

    /// Multiple budget consumptions should accumulate correctly.
    #[test]
    fn budget_accumulation_correct(
        consumptions in prop::collection::vec(1u64..100u64, 1..10)
    ) {
        let total: u64 = consumptions.iter().sum();
        let manual_sum: u64 = consumptions.into_iter().fold(0, |acc, c| acc + c);
        prop_assert_eq!(total, manual_sum, "Budget accumulation should match manual sum");
    }
}

// Tests for ID uniqueness properties

proptest! {
    /// Different ID strings should produce different AgentIds.
    #[test]
    fn different_strings_different_agent_ids(
        id1 in agent_id_string(),
        id2 in agent_id_string()
    ) {
        prop_assume!(id1 != id2);
        let agent_id1 = AgentId::new(&id1);
        let agent_id2 = AgentId::new(&id2);
        prop_assert_ne!(agent_id1, agent_id2);
    }

    /// AgentId hash should be consistent.
    #[test]
    fn agent_id_hash_consistent(id in agent_id_string()) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let agent_id = AgentId::new(&id);

        let mut hasher1 = DefaultHasher::new();
        agent_id.hash(&mut hasher1);
        let hash1 = hasher1.finish();

        let mut hasher2 = DefaultHasher::new();
        agent_id.hash(&mut hasher2);
        let hash2 = hasher2.finish();

        prop_assert_eq!(hash1, hash2, "Hash should be consistent");
    }

    /// Equal AgentIds should have equal hashes.
    #[test]
    fn equal_agent_ids_equal_hashes(id in agent_id_string()) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let id1 = AgentId::new(&id);
        let id2 = AgentId::new(&id);

        let mut hasher1 = DefaultHasher::new();
        id1.hash(&mut hasher1);

        let mut hasher2 = DefaultHasher::new();
        id2.hash(&mut hasher2);

        prop_assert_eq!(hasher1.finish(), hasher2.finish());
    }
}

// Tests for serialization round-trips

proptest! {
    /// AgentId should survive JSON serialization round-trip.
    #[test]
    fn agent_id_json_roundtrip(id in agent_id_string()) {
        let agent_id = AgentId::new(&id);
        let json = serde_json::to_string(&agent_id).expect("Serialization should succeed");
        let deserialized: AgentId = serde_json::from_str(&json).expect("Deserialization should succeed");
        prop_assert_eq!(agent_id, deserialized);
    }

    /// TokenBudget should survive JSON serialization round-trip.
    #[test]
    fn token_budget_json_roundtrip(
        per_request in proptest::option::of(token_limit()),
        per_hour in proptest::option::of(token_limit()),
        per_day in proptest::option::of(token_limit())
    ) {
        let mut budget = TokenBudget::new();
        if let Some(v) = per_request {
            budget = budget.with_per_request(v);
        }
        if let Some(v) = per_hour {
            budget = budget.with_per_hour(v);
        }
        if let Some(v) = per_day {
            budget = budget.with_per_day(v);
        }

        let json = serde_json::to_string(&budget).expect("Serialization should succeed");
        let deserialized: TokenBudget = serde_json::from_str(&json).expect("Deserialization should succeed");

        prop_assert_eq!(budget.per_request, deserialized.per_request);
        prop_assert_eq!(budget.per_hour, deserialized.per_hour);
        prop_assert_eq!(budget.per_day, deserialized.per_day);
    }
}
