use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};

struct FakeClock {
    now: Mutex<Instant>,
    sleeps: Mutex<Vec<Duration>>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            now: Mutex::new(Instant::now()),
            sleeps: Mutex::new(Vec::new()),
        }
    }

    fn sleeps(&self) -> Vec<Duration> {
        self.sleeps
            .lock()
            .map(|sleeps| sleeps.clone())
            .unwrap_or_default()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.now
            .lock()
            .map(|now| *now)
            .unwrap_or_else(|poisoned| *poisoned.into_inner())
    }

    fn sleep(&self, duration: Duration) {
        if let Ok(mut sleeps) = self.sleeps.lock() {
            sleeps.push(duration);
        }
        if let Ok(mut now) = self.now.lock() {
            *now += duration;
        }
    }
}

struct CancelOnSleepClock {
    clock: FakeClock,
    cancel: Arc<AtomicBool>,
}

impl Clock for CancelOnSleepClock {
    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn sleep(&self, duration: Duration) {
        self.clock.sleep(duration);
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[test]
fn first_actual_attempt_is_immediately_available() -> AppResult<()> {
    let clock = Arc::new(FakeClock::new());
    let pacer = ChatboxPacer::with_clock(clock.clone());
    let cancel = AtomicBool::new(false);

    let permit = pacer
        .wait_for_turn(Some(&cancel))?
        .ok_or_else(|| crate::error::AppError::runtime("First attempt was cancelled."))?;
    permit.attempt(|| Ok(()))?;

    assert!(clock.sleeps().is_empty());

    Ok(())
}

#[test]
fn actual_attempts_are_separated_by_the_fixed_one_second_interval() -> AppResult<()> {
    let clock = Arc::new(FakeClock::new());
    let pacer = ChatboxPacer::with_clock(clock.clone());
    let cancel = AtomicBool::new(false);

    pacer
        .wait_for_turn(Some(&cancel))?
        .ok_or_else(|| crate::error::AppError::runtime("First attempt was cancelled."))?
        .attempt(|| Ok(()))?;
    pacer
        .wait_for_turn(Some(&cancel))?
        .ok_or_else(|| crate::error::AppError::runtime("Second attempt was cancelled."))?
        .attempt(|| Ok(()))?;

    assert_eq!(
        clock.sleeps().into_iter().sum::<Duration>(),
        Duration::from_secs(1)
    );

    Ok(())
}

#[test]
fn failed_attempt_reserves_the_next_opportunity() -> AppResult<()> {
    let clock = Arc::new(FakeClock::new());
    let pacer = ChatboxPacer::with_clock(clock.clone());
    let cancel = AtomicBool::new(false);

    let first_result = pacer
        .wait_for_turn(Some(&cancel))?
        .ok_or_else(|| crate::error::AppError::runtime("First attempt was cancelled."))?
        .attempt::<()>(|| {
            Err(crate::error::AppError::osc_send(
                "test",
                "failure".to_string(),
            ))
        });
    assert!(first_result.is_err());

    pacer
        .wait_for_turn(Some(&cancel))?
        .ok_or_else(|| crate::error::AppError::runtime("Second attempt was cancelled."))?
        .attempt(|| Ok(()))?;

    assert_eq!(
        clock.sleeps().into_iter().sum::<Duration>(),
        Duration::from_secs(1)
    );

    Ok(())
}

#[test]
fn unused_permit_does_not_consume_an_attempt() -> AppResult<()> {
    let clock = Arc::new(FakeClock::new());
    let pacer = ChatboxPacer::with_clock(clock.clone());

    let unused = pacer
        .wait_for_turn(None)?
        .ok_or_else(|| crate::error::AppError::runtime("Permit was cancelled."))?;
    drop(unused);
    pacer
        .wait_for_turn(None)?
        .ok_or_else(|| crate::error::AppError::runtime("Attempt was cancelled."))?
        .attempt(|| Ok(()))?;

    assert!(clock.sleeps().is_empty());

    Ok(())
}

#[test]
fn concurrent_callers_never_consume_an_initial_burst() -> AppResult<()> {
    for _ in 0..100 {
        let clock = Arc::new(FakeClock::new());
        let pacer = ChatboxPacer::with_clock(clock.clone());
        let barrier = Arc::new(Barrier::new(3));
        let attempts = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();

        for _ in 0..2 {
            let worker_pacer = pacer.clone();
            let worker_barrier = barrier.clone();
            let worker_attempts = attempts.clone();
            workers.push(std::thread::spawn(move || -> AppResult<()> {
                worker_barrier.wait();
                worker_pacer
                    .wait_for_turn(None)?
                    .ok_or_else(|| crate::error::AppError::runtime("Attempt was cancelled."))?
                    .attempt(|| {
                        worker_attempts.fetch_add(1, Ordering::Relaxed);
                        Ok(())
                    })
            }));
        }

        barrier.wait();
        for worker in workers {
            worker
                .join()
                .map_err(|_| crate::error::AppError::runtime("Pacer test worker panicked."))??;
        }

        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        assert_eq!(
            clock.sleeps().into_iter().sum::<Duration>(),
            Duration::from_secs(1)
        );
    }

    Ok(())
}

#[test]
fn cancellation_during_wait_does_not_reserve_an_attempt() -> AppResult<()> {
    let cancel = Arc::new(AtomicBool::new(false));
    let clock = Arc::new(CancelOnSleepClock {
        clock: FakeClock::new(),
        cancel: cancel.clone(),
    });
    let pacer = ChatboxPacer::with_clock(clock.clone());

    pacer
        .wait_for_turn(None)?
        .ok_or_else(|| crate::error::AppError::runtime("First attempt was cancelled."))?
        .attempt(|| Ok(()))?;
    assert!(pacer.wait_for_turn(Some(cancel.as_ref()))?.is_none());
    assert_eq!(
        clock.clock.sleeps().into_iter().sum::<Duration>(),
        PACING_POLL_INTERVAL
    );

    cancel.store(false, Ordering::Relaxed);
    pacer
        .wait_for_turn(None)?
        .ok_or_else(|| crate::error::AppError::runtime("Follow-up attempt was cancelled."))?
        .attempt(|| Ok(()))?;
    assert_eq!(
        clock.clock.sleeps().into_iter().sum::<Duration>(),
        CHATBOX_ATTEMPT_INTERVAL
    );

    Ok(())
}
