//! Generation-scoped OpenAI recognition driver.
//!
//! This Module owns application speech boundaries, connection attempts, and
//! reconnect policy behind the provider-neutral active Recognition Module
//! boundary.

mod attempt;
mod audio;
mod realtime;
mod reconnect;
mod segmenter;
mod transport;

use self::attempt::{RecognitionAttempt, RecognitionAttemptAudioChunk};
use self::realtime::{OpenAiRealtimeAttempt, OpenAiRealtimeAttemptContext};
use self::reconnect::{ReconnectDecision, ReconnectTracker, reconnect_jitter_percent};
use self::segmenter::{SegmenterUpdate, SpeechSegmenter};
use self::transport::{OpenAiWebSocketTransport, connect_openai_realtime_attempt};
use super::{
    OwnedRecognitionAudioFrame, RecognitionDriver, RecognitionDriverInput, RecognitionDriverIo,
    RecognitionModule,
};
use crate::audio::SPEECH_RMS_THRESHOLD;
use crate::error::{AppError, AppResult};
use crate::host_resolver::HostResolver;
use crate::wall_clock::unix_timestamp_ms;
use secrecy::SecretString;
use std::time::{Duration, Instant};

const RECOGNITION_EVENT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_QUEUED_AUDIO: Duration = Duration::from_millis(500);
const MAX_QUEUED_AUDIO_FRAMES: usize = 64;
const SILENCE_TIMEOUT: Duration = Duration::from_millis(1_200);
const MIN_VOICED_SECONDS: f32 = 0.3;
const MAX_SEGMENT_SECONDS: f32 = 30.0;
const PREROLL_SECONDS: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenAiTranscriptionModel {
    GptTranscribe,
    GptLiveTranscribe,
}

impl OpenAiTranscriptionModel {
    const fn as_str(self) -> &'static str {
        match self {
            Self::GptTranscribe => "gpt-transcribe",
            Self::GptLiveTranscribe => "gpt-live-transcribe",
        }
    }
}

pub(crate) fn openai_gpt_transcribe_module(
    languages: Vec<String>,
    api_key: SecretString,
    resolver: HostResolver,
) -> AppResult<RecognitionModule> {
    openai_recognition_module(
        OpenAiTranscriptionModel::GptTranscribe,
        languages,
        api_key,
        resolver,
    )
}

pub(crate) fn openai_gpt_live_transcribe_module(
    languages: Vec<String>,
    api_key: SecretString,
    resolver: HostResolver,
) -> AppResult<RecognitionModule> {
    openai_recognition_module(
        OpenAiTranscriptionModel::GptLiveTranscribe,
        languages,
        api_key,
        resolver,
    )
}

fn openai_recognition_module(
    model: OpenAiTranscriptionModel,
    languages: Vec<String>,
    api_key: SecretString,
    resolver: HostResolver,
) -> AppResult<RecognitionModule> {
    RecognitionModule::with_audio_budget(
        MAX_QUEUED_AUDIO,
        MAX_QUEUED_AUDIO_FRAMES,
        OpenAiRecognitionDriver::new(OpenAiRealtimeAttemptFactory {
            model,
            languages,
            api_key,
            resolver,
        }),
    )
}

struct OpenAiRealtimeAttemptFactory {
    model: OpenAiTranscriptionModel,
    languages: Vec<String>,
    api_key: SecretString,
    resolver: HostResolver,
}

impl OpenAiRecognitionAttemptFactory for OpenAiRealtimeAttemptFactory {
    type Attempt = OpenAiRealtimeAttempt<OpenAiWebSocketTransport>;

    fn connect(
        &mut self,
        context: OpenAiRealtimeAttemptContext,
        is_cancelled: &dyn Fn() -> bool,
    ) -> AppResult<Self::Attempt> {
        connect_openai_realtime_attempt(
            context,
            self.model,
            self.languages.clone(),
            &self.api_key,
            &self.resolver,
            is_cancelled,
        )
    }
}

trait OpenAiRecognitionAttemptFactory: Send + 'static {
    type Attempt: RecognitionAttempt;

    fn connect(
        &mut self,
        context: OpenAiRealtimeAttemptContext,
        is_cancelled: &dyn Fn() -> bool,
    ) -> AppResult<Self::Attempt>;
}

struct OpenAiRecognitionDriver<F> {
    attempt_factory: F,
}

impl<F> OpenAiRecognitionDriver<F> {
    fn new(attempt_factory: F) -> Self {
        Self { attempt_factory }
    }
}

impl<F> RecognitionDriver for OpenAiRecognitionDriver<F>
where
    F: OpenAiRecognitionAttemptFactory,
{
    fn run(mut self: Box<Self>, io: RecognitionDriverIo) -> AppResult<()> {
        let mut last_sequence = None;
        // Caption-unit identity belongs to the generation's stable caption
        // stream, so replacing one protocol attempt must not reset it.
        let mut next_stream_unit_sequence = 1_u64;
        let mut reconnect_tracker = ReconnectTracker::default();

        loop {
            if io.is_stopped() {
                return Ok(());
            }

            let connection_epoch = reconnect_tracker.begin_connection_attempt();
            let context = OpenAiRealtimeAttemptContext {
                generation: io.scope().generation,
                connection_epoch,
                stream_id: io.scope().stream_id.clone(),
            };
            let connection_result = self.attempt_factory.connect(context, &|| io.is_stopped());
            let (attempt_result, connected_for) = match connection_result {
                Ok(mut attempt) => {
                    let connected_at = Instant::now();
                    let recovered = reconnect_tracker.has_reached_running();
                    let ready_result = io.ready(recovered);
                    if ready_result.is_ok() {
                        reconnect_tracker.mark_running();
                    }
                    let work_result = ready_result.and_then(|()| {
                        run_connected_attempt(
                            &io,
                            &mut attempt,
                            &mut last_sequence,
                            &mut next_stream_unit_sequence,
                        )
                    });
                    let stop_result = attempt.stop();
                    let result = if io.is_stopped() {
                        if let Err(stop_error) = stop_result {
                            tracing::warn!(
                                code = stop_error.code(),
                                error_message = %stop_error,
                                "OpenAI recognition attempt failed while closing after Stop"
                            );
                        }
                        Ok(())
                    } else {
                        combine_work_and_stop(work_result, stop_result)
                    };
                    (result, Some(connected_at.elapsed()))
                }
                Err(error) => (Err(error), None),
            };

            if io.is_stopped() {
                return Ok(());
            }
            let error = match attempt_result {
                Ok(()) => return Ok(()),
                Err(error) => error,
            };
            let ReconnectDecision::Retry {
                retry_number,
                delay,
            } = reconnect_tracker.on_failure(&error, connected_for, reconnect_jitter_percent())
            else {
                return Err(error);
            };

            if let Err(pause_error) = io.reconnecting(connection_epoch, retry_number, delay) {
                return if io.is_stopped() {
                    Ok(())
                } else {
                    Err(pause_error)
                };
            }
            if io.is_stopped() || io.wait_for_stop(delay)? {
                return Ok(());
            }
        }
    }
}

fn run_connected_attempt(
    io: &RecognitionDriverIo,
    attempt: &mut impl RecognitionAttempt,
    last_sequence: &mut Option<u64>,
    next_stream_unit_sequence: &mut u64,
) -> AppResult<()> {
    let mut segmenter = None;

    let work_result = (|| -> AppResult<()> {
        loop {
            match io.receive(RECOGNITION_EVENT_POLL_INTERVAL)? {
                RecognitionDriverInput::Audio(frame) => {
                    validate_frame_order(&frame, *last_sequence)?;
                    *last_sequence = Some(frame.sequence);
                    let segmenter =
                        segmenter.get_or_insert_with(|| new_openai_segmenter(frame.sample_rate_hz));
                    if segmenter.sample_rate() != frame.sample_rate_hz {
                        return Err(AppError::audio(
                            "Microphone sample rate changed during an active recognition attempt.",
                        ));
                    }
                    let started_at_ms = frame.captured_at_ms;
                    apply_segmenter_updates(
                        io,
                        attempt,
                        frame.sample_rate_hz,
                        started_at_ms,
                        next_stream_unit_sequence,
                        segmenter.push_samples(frame.samples.into_vec(), Instant::now()),
                    )?;
                }
                RecognitionDriverInput::Idle => {
                    if let Some(segmenter) = segmenter.as_mut() {
                        let sample_rate_hz = segmenter.sample_rate();
                        apply_segmenter_updates(
                            io,
                            attempt,
                            sample_rate_hz,
                            unix_timestamp_ms(),
                            next_stream_unit_sequence,
                            [segmenter.tick(Instant::now())],
                        )?;
                    }
                }
                RecognitionDriverInput::Stopped => break,
            }

            for event in attempt.drain_events(unix_timestamp_ms())? {
                io.emit_event(event)?;
            }
        }
        Ok(())
    })();

    if let Some(segmenter) = segmenter.as_mut() {
        segmenter.discard_open_tail();
    }
    work_result
}

fn new_openai_segmenter(sample_rate_hz: u32) -> SpeechSegmenter {
    SpeechSegmenter::new(
        sample_rate_hz,
        SPEECH_RMS_THRESHOLD,
        SILENCE_TIMEOUT,
        MIN_VOICED_SECONDS,
        MAX_SEGMENT_SECONDS,
        PREROLL_SECONDS,
    )
}

fn validate_frame_order(
    frame: &OwnedRecognitionAudioFrame,
    last_sequence: Option<u64>,
) -> AppResult<()> {
    if frame.sample_rate_hz == 0 || frame.samples.is_empty() {
        return Err(AppError::audio(
            "Recognition audio frames require a sample rate and at least one sample.",
        ));
    }
    if last_sequence.is_some_and(|sequence| frame.sequence <= sequence) {
        return Err(AppError::state(
            "Recognition audio frames arrived out of sequence.",
        ));
    }
    Ok(())
}

fn apply_segmenter_updates(
    io: &RecognitionDriverIo,
    attempt: &mut impl RecognitionAttempt,
    sample_rate_hz: u32,
    started_at_ms: u64,
    next_stream_unit_sequence: &mut u64,
    updates: impl IntoIterator<Item = SegmenterUpdate>,
) -> AppResult<()> {
    for update in updates {
        if update.speech_started {
            let event = attempt.start_unit(
                next_stream_unit_id(next_stream_unit_sequence)?,
                started_at_ms,
            )?;
            io.emit_event(event)?;
        }
        if !update.audio.is_empty() {
            attempt.append_audio(RecognitionAttemptAudioChunk {
                sample_rate_hz,
                samples: &update.audio,
            })?;
        }
        if update.speech_ended {
            attempt.end_input()?;
        }
    }
    Ok(())
}

fn next_stream_unit_id(next_sequence: &mut u64) -> AppResult<String> {
    let sequence = *next_sequence;
    *next_sequence = sequence.checked_add(1).ok_or_else(|| {
        AppError::recognition("OpenAI caption unit sequence exceeded the supported range.")
    })?;
    Ok(format!("unit-{sequence}"))
}

fn combine_work_and_stop(work: AppResult<()>, stop: AppResult<()>) -> AppResult<()> {
    match (work, stop) {
        (Err(work_error), Err(stop_error)) => {
            tracing::warn!(
                code = stop_error.code(),
                error_message = %stop_error,
                "OpenAI recognition attempt also failed while closing after a driver error"
            );
            Err(work_error)
        }
        (Err(work_error), Ok(())) => Err(work_error),
        (Ok(()), Err(stop_error)) => Err(stop_error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

#[cfg(test)]
#[path = "openai_tests.rs"]
mod tests;
