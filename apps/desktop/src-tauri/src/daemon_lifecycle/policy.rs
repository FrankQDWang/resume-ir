use std::collections::VecDeque;
use std::time::Duration;

pub(super) const MAX_AUTOMATIC_RESTARTS: usize = 5;

#[derive(Clone, Copy, Debug)]
pub(super) struct RestartPolicyConfig {
    pub(super) window: Duration,
    pub(super) stable_reset: Duration,
    pub(super) circuit_open: Duration,
    pub(super) backoff: [Duration; MAX_AUTOMATIC_RESTARTS],
}

impl RestartPolicyConfig {
    pub(super) const fn production() -> Self {
        Self {
            window: Duration::from_secs(10 * 60),
            stable_reset: Duration::from_secs(5 * 60),
            circuit_open: Duration::from_secs(5 * 60),
            backoff: [
                Duration::from_millis(250),
                Duration::from_secs(1),
                Duration::from_secs(4),
                Duration::from_secs(15),
                Duration::from_secs(30),
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RecoveryDecision {
    RetryAfter(Duration),
    OpenCircuit(Duration),
}

#[derive(Debug)]
pub(super) struct RestartPolicy {
    config: RestartPolicyConfig,
    automatic_restarts: VecDeque<Duration>,
    stable_since: Option<Duration>,
    half_open_attempt: bool,
    manual_half_open_at: Option<Duration>,
}

impl RestartPolicy {
    pub(super) fn new(config: RestartPolicyConfig) -> Self {
        Self {
            config,
            automatic_restarts: VecDeque::new(),
            stable_since: None,
            half_open_attempt: false,
            manual_half_open_at: None,
        }
    }

    pub(super) fn on_failure(&mut self, now: Duration) -> RecoveryDecision {
        let now = self.policy_time(now);
        self.stable_since = None;
        if self.half_open_attempt {
            self.half_open_attempt = false;
            self.manual_half_open_at = Some(now.saturating_add(self.config.circuit_open));
            return RecoveryDecision::OpenCircuit(self.config.circuit_open);
        }
        self.prune(now);
        if self.automatic_restarts.len() >= MAX_AUTOMATIC_RESTARTS {
            self.manual_half_open_at = Some(now.saturating_add(self.config.circuit_open));
            return RecoveryDecision::OpenCircuit(self.config.circuit_open);
        }
        let delay = self.config.backoff[self.automatic_restarts.len()];
        self.automatic_restarts.push_back(now);
        RecoveryDecision::RetryAfter(delay)
    }

    fn begin_half_open(&mut self) {
        self.half_open_attempt = true;
        self.manual_half_open_at = None;
        self.stable_since = None;
    }

    pub(super) fn begin_manual_half_open(&mut self, now: Duration) -> bool {
        let now = self.policy_time(now);
        if self.manual_half_open_at.is_none_or(|at| now < at) {
            return false;
        }
        self.begin_half_open();
        true
    }

    pub(super) fn manual_half_open_retry_after(&self, now: Duration) -> Option<Duration> {
        let now = self.policy_time(now);
        self.manual_half_open_at.map(|at| at.saturating_sub(now))
    }

    pub(super) fn on_ready(&mut self, now: Duration) {
        self.stable_since = Some(self.policy_time(now));
    }

    pub(super) fn stable_reset_due(&self, now: Duration) -> bool {
        let now = self.policy_time(now);
        self.stable_since
            .is_some_and(|since| now.saturating_sub(since) >= self.config.stable_reset)
    }

    pub(super) fn complete_stable_reset(&mut self, now: Duration) {
        self.automatic_restarts.clear();
        self.half_open_attempt = false;
        self.manual_half_open_at = None;
        self.stable_since = Some(self.policy_time(now));
    }

    pub(super) fn restart_attempts(&mut self, now: Duration) -> u8 {
        let now = self.policy_time(now);
        self.prune(now);
        self.automatic_restarts.len() as u8
    }

    fn policy_time(&self, elapsed: Duration) -> Duration {
        self.config.window.saturating_add(elapsed)
    }

    fn prune(&mut self, now: Duration) {
        while self
            .automatic_restarts
            .front()
            .is_some_and(|at| now.saturating_sub(*at) >= self.config.window)
        {
            self.automatic_restarts.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> RestartPolicyConfig {
        RestartPolicyConfig {
            window: Duration::from_secs(600),
            stable_reset: Duration::from_secs(300),
            circuit_open: Duration::from_secs(300),
            backoff: [
                Duration::from_millis(250),
                Duration::from_secs(1),
                Duration::from_secs(4),
                Duration::from_secs(15),
                Duration::from_secs(30),
            ],
        }
    }

    #[test]
    fn five_automatic_restarts_use_the_fixed_backoff_then_open_the_circuit() {
        let mut policy = RestartPolicy::new(test_config());
        let expected = [250, 1_000, 4_000, 15_000, 30_000];
        for (index, delay_ms) in expected.into_iter().enumerate() {
            assert_eq!(
                policy.on_failure(Duration::from_secs(index as u64)),
                RecoveryDecision::RetryAfter(Duration::from_millis(delay_ms))
            );
        }
        assert_eq!(
            policy.on_failure(Duration::from_secs(6)),
            RecoveryDecision::OpenCircuit(Duration::from_secs(300))
        );
    }

    #[test]
    fn half_open_failure_reopens_the_circuit_without_erasing_history() {
        let mut policy = RestartPolicy::new(test_config());
        assert!(matches!(
            policy.on_failure(Duration::ZERO),
            RecoveryDecision::RetryAfter(_)
        ));
        policy.begin_half_open();
        assert_eq!(
            policy.on_failure(Duration::from_secs(1)),
            RecoveryDecision::OpenCircuit(Duration::from_secs(300))
        );
        assert_eq!(policy.restart_attempts(Duration::from_secs(1)), 1);
    }

    #[test]
    fn manual_half_open_waits_for_expiry_allows_once_and_reopens_after_failure() {
        let mut policy = RestartPolicy::new(test_config());
        for second in 0..MAX_AUTOMATIC_RESTARTS {
            assert!(matches!(
                policy.on_failure(Duration::from_secs(second as u64)),
                RecoveryDecision::RetryAfter(_)
            ));
        }
        assert_eq!(
            policy.on_failure(Duration::from_secs(MAX_AUTOMATIC_RESTARTS as u64)),
            RecoveryDecision::OpenCircuit(Duration::from_secs(300))
        );
        assert_eq!(
            policy.manual_half_open_retry_after(Duration::from_secs(5)),
            Some(Duration::from_secs(300))
        );
        assert!(!policy.begin_manual_half_open(Duration::from_secs(304)));
        assert!(policy.begin_manual_half_open(Duration::from_secs(305)));
        assert!(!policy.begin_manual_half_open(Duration::from_secs(305)));
        assert_eq!(
            policy.on_failure(Duration::from_secs(306)),
            RecoveryDecision::OpenCircuit(Duration::from_secs(300))
        );
        assert!(!policy.begin_manual_half_open(Duration::from_secs(605)));
        assert!(policy.begin_manual_half_open(Duration::from_secs(606)));
    }

    #[test]
    fn five_minutes_of_health_resets_the_restart_budget() {
        let mut policy = RestartPolicy::new(test_config());
        assert!(matches!(
            policy.on_failure(Duration::ZERO),
            RecoveryDecision::RetryAfter(_)
        ));
        policy.on_ready(Duration::from_secs(1));
        assert!(!policy.stable_reset_due(Duration::from_secs(300)));
        assert_eq!(policy.restart_attempts(Duration::from_secs(300)), 1);
        assert!(policy.stable_reset_due(Duration::from_secs(301)));
        policy.complete_stable_reset(Duration::from_secs(301));
        assert_eq!(policy.restart_attempts(Duration::from_secs(301)), 0);
    }

    #[test]
    fn ten_minute_window_prunes_old_restart_attempts() {
        let mut policy = RestartPolicy::new(test_config());
        assert!(matches!(
            policy.on_failure(Duration::ZERO),
            RecoveryDecision::RetryAfter(_)
        ));
        assert_eq!(policy.restart_attempts(Duration::from_secs(599)), 1);
        assert_eq!(policy.restart_attempts(Duration::from_secs(600)), 0);
    }

    #[test]
    fn a_new_policy_session_starts_with_a_fresh_budget() {
        let mut first = RestartPolicy::new(test_config());
        assert!(matches!(
            first.on_failure(Duration::ZERO),
            RecoveryDecision::RetryAfter(_)
        ));
        assert_eq!(first.restart_attempts(Duration::ZERO), 1);

        let mut reopened = RestartPolicy::new(test_config());
        assert_eq!(reopened.restart_attempts(Duration::ZERO), 0);
    }
}
