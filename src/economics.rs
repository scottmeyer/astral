//! Caller-supplied estimates in one consistent cost unit; no baked-in prices.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollEstimate {
    pub remaining_calls: u32,
    pub saved_input_per_call: f64,
    pub checkpoint_cost: f64,
    pub lost_cache_cost: f64,
    pub recovery_cost: f64,
}
impl RollEstimate {
    pub fn net_savings(&self) -> anyhow::Result<f64> {
        anyhow::ensure!(
            [
                self.saved_input_per_call,
                self.checkpoint_cost,
                self.lost_cache_cost,
                self.recovery_cost
            ]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0),
            "cost estimates must be finite and nonnegative"
        );
        let net = f64::from(self.remaining_calls) * self.saved_input_per_call
            - self.checkpoint_cost
            - self.lost_cache_cost
            - self.recovery_cost;
        anyhow::ensure!(net.is_finite(), "cost estimate overflow");
        Ok(net)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn costs_include_rebuild_cache_and_recovery_and_reject_invalid_estimates() {
        let mut e = RollEstimate {
            remaining_calls: 5,
            saved_input_per_call: 10.0,
            checkpoint_cost: 20.0,
            lost_cache_cost: 10.0,
            recovery_cost: 5.0,
        };
        assert_eq!(e.net_savings().unwrap(), 15.0);
        e.remaining_calls = 1;
        assert!(e.net_savings().unwrap() < 0.0);
        e.saved_input_per_call = f64::NAN;
        assert!(e.net_savings().is_err());
    }
}
