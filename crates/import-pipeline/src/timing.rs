use std::time::{Duration, Instant};

pub(crate) fn measure_result_stage<T, E>(
    stage: &mut Duration,
    operation: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    let started = Instant::now();
    let result = operation();
    *stage += started.elapsed();
    result
}

pub(crate) fn measure_stage<T>(stage: &mut Duration, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = operation();
    *stage += started.elapsed();
    result
}
