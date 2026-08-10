use super::attempt::{RecognitionAttemptAudioChunk, RecognitionAttemptSession};
use super::realtime::OpenAiRealtimeSessionContext;
use super::*;
use crate::error::{AppError, AppResult};
use crate::recognition::{
    OwnedRecognitionAudioFrame, RecognitionEvent, RecognitionGenerationScope, RecognitionModule,
    RecognitionSignal,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Default)]
struct SessionProbe {
    appended_samples: usize,
    connect_attempts: usize,
    stopped: bool,
}

struct RecordingSession {
    context: OpenAiRealtimeSessionContext,
    probe: Arc<Mutex<SessionProbe>>,
    fail_next_drain: bool,
}

impl RecognitionAttemptSession for RecordingSession {
    fn start_unit(&mut self, unit_id: String, started_at_ms: u64) -> AppResult<RecognitionEvent> {
        Ok(RecognitionEvent::UnitStarted {
            generation: self.context.generation,
            stream_id: self.context.stream_id.clone(),
            unit_id,
            started_at_ms,
        })
    }

    fn append_audio(&mut self, audio: RecognitionAttemptAudioChunk<'_>) -> AppResult<()> {
        let mut probe = self
            .probe
            .lock()
            .map_err(|_| AppError::state("OpenAI driver test probe lock was poisoned."))?;
        probe.appended_samples = probe.appended_samples.saturating_add(audio.samples.len());
        Ok(())
    }

    fn end_input(&mut self) -> AppResult<()> {
        Ok(())
    }

    fn drain_events(&mut self, _received_at_ms: u64) -> AppResult<Vec<RecognitionEvent>> {
        if self.fail_next_drain {
            self.fail_next_drain = false;
            return Err(AppError::stt_network_retryable(
                "The test connection was interrupted.",
            ));
        }
        Ok(Vec::new())
    }

    fn stop(&mut self) -> AppResult<()> {
        let mut probe = self
            .probe
            .lock()
            .map_err(|_| AppError::state("OpenAI driver test probe lock was poisoned."))?;
        probe.stopped = true;
        Ok(())
    }
}

struct RecordingAttemptFactory {
    probe: Arc<Mutex<SessionProbe>>,
    fail_first_drain: bool,
}

impl OpenAiRecognitionAttemptFactory for RecordingAttemptFactory {
    type Session = RecordingSession;

    fn connect(
        &mut self,
        context: OpenAiRealtimeSessionContext,
        _is_cancelled: &dyn Fn() -> bool,
    ) -> AppResult<Self::Session> {
        let mut probe = self
            .probe
            .lock()
            .map_err(|_| AppError::state("OpenAI driver test probe lock was poisoned."))?;
        probe.connect_attempts = probe.connect_attempts.saturating_add(1);
        let attempt = probe.connect_attempts;
        drop(probe);
        Ok(RecordingSession {
            context,
            probe: Arc::clone(&self.probe),
            fail_next_drain: self.fail_first_drain && attempt == 1,
        })
    }
}

#[test]
fn openai_segmenter_ends_an_announced_unit_after_1_2_seconds_without_input() {
    let started_at = Instant::now();
    let mut segmenter = new_openai_segmenter(1_000);
    let started = segmenter.push_samples(vec![0.02; 1_000], started_at);

    assert!(started.iter().any(|update| update.speech_started));
    assert!(
        !segmenter
            .tick(started_at + Duration::from_millis(1_199))
            .speech_ended
    );
    assert!(
        segmenter
            .tick(started_at + Duration::from_millis(1_200))
            .speech_ended
    );
}

#[test]
fn openai_segmenter_hard_splits_continuous_speech_at_30_seconds() {
    let started_at = Instant::now();
    let mut segmenter = new_openai_segmenter(1_000);

    let before_boundary = segmenter.push_samples(vec![0.02; 29_990], started_at);
    assert!(before_boundary.iter().any(|update| update.speech_started));
    assert!(before_boundary.iter().all(|update| !update.speech_ended));

    let boundary = segmenter.push_samples(vec![0.02; 10], started_at + Duration::from_millis(10));
    assert_eq!(boundary.len(), 1);
    assert_eq!(boundary[0].audio.len(), 10);
    assert!(boundary[0].speech_ended);
}

#[test]
fn continuous_audio_is_unitized_inside_the_openai_recognition_module() -> AppResult<()> {
    let probe = Arc::new(Mutex::new(SessionProbe::default()));
    let driver = OpenAiRecognitionDriver::new(RecordingAttemptFactory {
        probe: Arc::clone(&probe),
        fail_first_drain: false,
    });
    let module = RecognitionModule::with_audio_budget(Duration::from_millis(500), 8, driver)?;
    let mut running = module.start(RecognitionGenerationScope {
        generation: 9,
        stream_id: "recognition-9-1".to_string(),
    })?;

    assert!(matches!(
        running.signals.recv_timeout(TEST_TIMEOUT),
        Ok(RecognitionSignal::Ready {
            recovered: false,
            ..
        })
    ));
    running
        .try_submit(OwnedRecognitionAudioFrame {
            sequence: 1,
            captured_at_ms: 123,
            sample_rate_hz: 16_000,
            samples: vec![0.25; 4_800].into_boxed_slice(),
        })
        .map_err(|error| AppError::state(format!("Test audio was rejected: {error:?}")))?;

    assert!(matches!(
        running.signals.recv_timeout(TEST_TIMEOUT),
        Ok(RecognitionSignal::Event(RecognitionEvent::UnitStarted {
            generation: 9,
            started_at_ms: 123,
            ..
        }))
    ));
    running.stop()?;

    let probe = probe
        .lock()
        .map_err(|_| AppError::state("OpenAI driver test probe lock was poisoned."))?;
    assert!(probe.appended_samples >= 4_800);
    assert!(probe.stopped);
    Ok(())
}

#[test]
fn retryable_failure_pauses_capture_before_opening_a_fresh_attempt() -> AppResult<()> {
    let probe = Arc::new(Mutex::new(SessionProbe::default()));
    let driver = OpenAiRecognitionDriver::new(RecordingAttemptFactory {
        probe: Arc::clone(&probe),
        fail_first_drain: true,
    });
    let module = RecognitionModule::with_audio_budget(Duration::from_millis(500), 8, driver)?;
    let mut running = module.start(RecognitionGenerationScope {
        generation: 11,
        stream_id: "recognition-11-1".to_string(),
    })?;

    assert!(matches!(
        running.signals.recv_timeout(TEST_TIMEOUT),
        Ok(RecognitionSignal::Ready {
            recovered: false,
            ..
        })
    ));
    running
        .try_submit(OwnedRecognitionAudioFrame {
            sequence: 1,
            captured_at_ms: 456,
            sample_rate_hz: 16_000,
            samples: vec![0.25; 160].into_boxed_slice(),
        })
        .map_err(|error| AppError::state(format!("Test audio was rejected: {error:?}")))?;

    let pause_epoch = match running.signals.recv_timeout(TEST_TIMEOUT) {
        Ok(RecognitionSignal::Reconnecting {
            epoch,
            attempt: 1,
            delay_ms,
        }) => {
            assert!((400..=600).contains(&delay_ms));
            epoch
        }
        signal => {
            return Err(AppError::state(format!(
                "Expected reconnect signal, received {signal:?}."
            )));
        }
    };
    assert!(!running.is_accepting_audio());
    running.acknowledge_capture_paused(pause_epoch)?;

    assert!(matches!(
        running.signals.recv_timeout(Duration::from_secs(2)),
        Ok(RecognitionSignal::Ready {
            recovered: true,
            ..
        })
    ));
    running.stop()?;

    let probe = probe
        .lock()
        .map_err(|_| AppError::state("OpenAI driver test probe lock was poisoned."))?;
    assert_eq!(probe.connect_attempts, 2);
    assert!(probe.stopped);
    Ok(())
}
