//! Runs the crash-recovery simulation over many seeds.
//!
//! - `SHIIOO_SIM_SEEDS=2000 cargo test -p shiioo-sim` runs seeds 0..2000
//!   (default 200).
//! - `SHIIOO_SIM_SEED=1234 cargo test -p shiioo-sim -- --nocapture` runs
//!   one seed, for reproducing a failure.

use shiioo_sim::{Report, run_scenario};

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

#[test]
fn crash_recovery_holds_for_every_seed() {
    let seeds: Vec<u64> = match env_u64("SHIIOO_SIM_SEED") {
        Some(seed) => vec![seed],
        None => (0..env_u64("SHIIOO_SIM_SEEDS").unwrap_or(200)).collect(),
    };

    let mut reports: Vec<Report> = Vec::new();
    let mut failures = Vec::new();
    for &seed in &seeds {
        match run_scenario(seed) {
            Ok(report) => reports.push(report),
            Err(violation) => failures.push(violation),
        }
    }

    let total = |f: fn(&Report) -> usize| reports.iter().map(f).sum::<usize>();
    println!(
        "{} seeds: {} lives, {} crashes, {} races, {} machine restarts, {} stale replays fenced, \
         {} runs stopped for a human",
        seeds.len(),
        total(|r| r.lives),
        total(|r| r.crashes),
        total(|r| r.races),
        total(|r| r.restarts),
        total(|r| r.stale_replays),
        reports.iter().filter(|r| r.needs_attention).count(),
    );

    if let Some(first) = failures.first() {
        let seeds: Vec<u64> = failures.iter().map(|v| v.seed).collect();
        panic!(
            "{} of {} seeds failed: {seeds:?}\n\n{first}",
            failures.len(),
            reports.len() + failures.len()
        );
    }

    // The harness only proves something if it actually exercised the faults.
    if seeds.len() >= 100 {
        assert!(total(|r| r.crashes) > 0, "no crashes were simulated");
        assert!(total(|r| r.races) > 0, "no races were simulated");
        assert!(
            total(|r| r.restarts) > 0,
            "no machine restarts were simulated"
        );
        assert!(
            total(|r| r.stale_replays) > 0,
            "no stale invocations were replayed"
        );
        assert!(
            reports.iter().any(|r| r.needs_attention),
            "no run hit a side-effecting call mid-execution"
        );
        assert!(
            reports.iter().any(|r| !r.needs_attention),
            "no run completed"
        );
    }
}

#[test]
fn same_seed_same_report() {
    for seed in [0, 1, 7, 42] {
        let a = run_scenario(seed).unwrap_or_else(|v| panic!("{v}"));
        let b = run_scenario(seed).unwrap_or_else(|v| panic!("{v}"));
        assert_eq!(
            (
                a.lives,
                a.crashes,
                a.races,
                a.restarts,
                a.stale_replays,
                a.needs_attention
            ),
            (
                b.lives,
                b.crashes,
                b.races,
                b.restarts,
                b.stale_replays,
                b.needs_attention
            ),
            "seed {seed} is not reproducible"
        );
    }
}
