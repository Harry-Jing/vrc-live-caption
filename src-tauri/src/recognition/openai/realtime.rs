//! OpenAI Realtime transcription protocol state machine.
//!
//! This module is a transport-independent deep module: it owns the OpenAI JSON
//! protocol, item correlation, model-specific output semantics, completion
//! ordering, and the hard Stop fence. Runtime only supplies a WebSocket
//! transport and consumes normalized `RecognitionEvent`s.

use super::OpenAiTranscriptionModel;
use super::attempt::{RecognitionAttempt, RecognitionAttemptAudioChunk};
use super::audio::{REALTIME_PCM_SAMPLE_RATE_HZ, RealtimePcm16Encoder};
use crate::caption::{CaptionLane, CaptionSnapshotV2, CaptionState};
use crate::error::{AppError, AppResult, ProviderFailureClass};
use crate::recognition::{RecognitionEvent, RecognitionUnitAbortReason};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

const PROVIDER_NAME: &str = "openai";
const RECENT_FINISHED_ITEM_LIMIT: usize = 128;
const MAX_OUTSTANDING_UNITS: usize = 32;
const MAX_PENDING_PROVIDER_ITEMS: usize = 32;
const MAX_PENDING_EVENTS_PER_ITEM: usize = 64;
const MAX_SERVER_FRAMES_PER_DRAIN: usize = 64;
const MAX_SATURATED_DRAINS_AFTER_DEADLINE: u8 = 2;
const MAX_TRANSCRIPT_BYTES_PER_UNIT: usize = 256 * 1024;
const MAX_PENDING_TRANSCRIPT_BYTES: usize = 1024 * 1024;
const ITEM_COMPLETION_TIMEOUT: Duration = Duration::from_secs(30);

/// Narrow external seam implemented by the eventual WebSocket connector and
/// by deterministic tests. It intentionally deals only in text frames because
/// Realtime audio is base64 inside JSON client events.
pub(crate) trait RealtimeTransport: Send {
    fn send_text(&mut self, message: String) -> AppResult<()>;
    fn try_receive_text(&mut self) -> AppResult<Option<String>>;
    fn close(&mut self) -> AppResult<()>;
}

trait MonotonicClock: Send {
    fn now(&self) -> Duration;
}

struct InstantClock {
    origin: Instant,
}

impl InstantClock {
    fn start() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl MonotonicClock for InstantClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenAiRealtimeAttemptContext {
    pub(crate) generation: u64,
    pub(crate) connection_epoch: u64,
    pub(crate) stream_id: String,
}

struct RecognitionUnitState {
    unit_id: String,
    started_at_ms: u64,
    provider_item_id: Option<String>,
    revision: u64,
    live_text: String,
    terminal: Option<UnitTerminal>,
    completion_wait_started_at: Option<Duration>,
}

struct CaptionEmission {
    unit_id: String,
    started_at_ms: u64,
    revision: u64,
    text: String,
    state: CaptionState,
    language: Option<String>,
    timestamp_ms: u64,
}

enum UnitTerminal {
    Caption(CaptionSnapshotV2),
    Aborted(RecognitionUnitAbortReason),
}

enum PendingTranscriptEvent {
    Delta {
        delta: String,
        received_at_ms: u64,
    },
    Completed {
        transcript: String,
        detected_languages: Vec<String>,
        received_at_ms: u64,
    },
    Failed {
        detail: String,
    },
}

impl PendingTranscriptEvent {
    fn text_bytes(&self) -> usize {
        match self {
            Self::Delta { delta, .. } => delta.len(),
            Self::Completed { transcript, .. } => transcript.len(),
            Self::Failed { .. } => 0,
        }
    }
}

pub(crate) struct OpenAiRealtimeAttempt<T: RealtimeTransport> {
    context: OpenAiRealtimeAttemptContext,
    model: OpenAiTranscriptionModel,
    languages: Vec<String>,
    transport: T,
    audio_encoder: RealtimePcm16Encoder,
    ready: bool,
    stopped: bool,
    next_unit_sequence: u64,
    active_unit_sequence: Option<u64>,
    units_by_sequence: BTreeMap<u64, RecognitionUnitState>,
    committed_unit_order: VecDeque<u64>,
    item_to_unit_sequence: HashMap<String, u64>,
    pending_by_item: HashMap<String, VecDeque<PendingTranscriptEvent>>,
    pending_transcript_bytes: usize,
    ready_events: VecDeque<RecognitionEvent>,
    recent_finished_items: VecDeque<String>,
    saturated_drains_after_deadline: u8,
    clock: Box<dyn MonotonicClock>,
}

impl<T: RealtimeTransport> OpenAiRealtimeAttempt<T> {
    pub(crate) fn connect(
        context: OpenAiRealtimeAttemptContext,
        model: OpenAiTranscriptionModel,
        languages: Vec<String>,
        transport: T,
    ) -> AppResult<Self> {
        Self::connect_with_clock(
            context,
            model,
            languages,
            transport,
            Box::new(InstantClock::start()),
        )
    }

    fn connect_with_clock(
        context: OpenAiRealtimeAttemptContext,
        model: OpenAiTranscriptionModel,
        languages: Vec<String>,
        transport: T,
        clock: Box<dyn MonotonicClock>,
    ) -> AppResult<Self> {
        if context.stream_id.trim().is_empty() {
            return Err(AppError::recognition(
                "Realtime recognition stream ID cannot be empty.",
            ));
        }
        let languages = normalize_languages(languages)?;
        let mut attempt = Self {
            context,
            model,
            languages,
            transport,
            audio_encoder: RealtimePcm16Encoder::new(),
            ready: false,
            stopped: false,
            next_unit_sequence: 0,
            active_unit_sequence: None,
            units_by_sequence: BTreeMap::new(),
            committed_unit_order: VecDeque::new(),
            item_to_unit_sequence: HashMap::new(),
            pending_by_item: HashMap::new(),
            pending_transcript_bytes: 0,
            ready_events: VecDeque::new(),
            recent_finished_items: VecDeque::new(),
            saturated_drains_after_deadline: 0,
            clock,
        };
        let update = session_update(
            model,
            &attempt.languages,
            attempt.context.generation,
            attempt.context.connection_epoch,
        );
        attempt.send_client_event(update)?;
        Ok(attempt)
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.ready
    }

    fn ensure_not_stopped(&self) -> AppResult<()> {
        if self.stopped {
            Err(AppError::recognition(
                "OpenAI Realtime recognition attempt has already stopped.",
            ))
        } else {
            Ok(())
        }
    }

    fn send_client_event(&mut self, event: Value) -> AppResult<()> {
        let message = serde_json::to_string(&event).map_err(|error| {
            AppError::recognition(format!(
                "Failed to serialize an OpenAI Realtime client event: {error}"
            ))
        })?;
        self.transport.send_text(message)
    }

    fn send_audio_bytes(&mut self, pcm16: &[u8]) -> AppResult<()> {
        if pcm16.is_empty() {
            return Ok(());
        }
        self.send_client_event(json!({
            "type": "input_audio_buffer.append",
            "audio": BASE64_STANDARD.encode(pcm16),
        }))
    }

    fn handle_server_message(&mut self, message: &str, received_at_ms: u64) -> AppResult<()> {
        let event: ServerEvent = serde_json::from_str(message).map_err(|error| {
            tracing::warn!(
                json_error_category = ?error.classify(),
                line = error.line(),
                column = error.column(),
                "OpenAI Realtime returned an invalid server event"
            );
            AppError::recognition("OpenAI Realtime returned an invalid server event.")
        })?;
        match event {
            ServerEvent::SessionUpdated => {
                self.ready = true;
                Ok(())
            }
            ServerEvent::BufferCommitted { item_id } => self.bind_committed_item(item_id),
            ServerEvent::TranscriptDelta { item_id, delta } => {
                self.handle_delta(item_id, delta, received_at_ms)
            }
            ServerEvent::TranscriptCompleted {
                item_id,
                transcript,
                languages,
            } => self.handle_completed(
                item_id,
                transcript,
                languages
                    .into_iter()
                    .map(|language| language.code)
                    .collect(),
                received_at_ms,
            ),
            ServerEvent::TranscriptFailed { item_id, error } => {
                let class = error.classification();
                if class == ProviderFailureClass::Unknown {
                    tracing::warn!(
                        provider = PROVIDER_NAME,
                        provider_failure_class = ?class,
                        "OpenAI Realtime transcription item failed"
                    );
                    self.handle_failed(
                        item_id,
                        "OpenAI could not transcribe one audio item.".to_string(),
                    )
                } else {
                    self.fail_provider_session(class)
                }
            }
            ServerEvent::Error { error } => {
                let class = error.classification();
                self.fail_provider_session(class)
            }
            ServerEvent::Other => Ok(()),
        }
    }

    fn fail_provider_session(&mut self, class: ProviderFailureClass) -> AppResult<()> {
        let failure = openai_provider_failure(class);
        tracing::error!(
            provider = PROVIDER_NAME,
            provider_failure_class = ?failure.provider_failure_class(),
            retry_disposition = ?failure.retry_disposition(),
            code = failure.code(),
            "OpenAI Realtime provider failure requires a clean reconnect"
        );

        // Although some provider errors are recoverable, append/commit and
        // item failures can leave the remote audio buffer ambiguous. A clean
        // reconnect is safer than publishing captions from a drifted unit.
        let _ = self.stop();
        Err(failure)
    }

    fn bind_committed_item(&mut self, item_id: String) -> AppResult<()> {
        if self.is_recently_finished(&item_id) {
            return Ok(());
        }
        if self.item_to_unit_sequence.contains_key(&item_id) {
            return self.replay_pending_item_events(&item_id);
        }

        let Some(sequence) = self.committed_unit_order.iter().copied().find(|sequence| {
            self.units_by_sequence
                .get(sequence)
                .is_some_and(|unit| unit.provider_item_id.is_none())
        }) else {
            return Err(AppError::recognition(
                "An OpenAI committed item did not match a local committed unit.",
            ));
        };
        self.bind_item_to_unit(sequence, item_id.clone())?;
        self.replay_pending_item_events(&item_id)
    }

    fn bind_item_to_unit(&mut self, sequence: u64, item_id: String) -> AppResult<()> {
        if let Some(existing_sequence) = self.item_to_unit_sequence.get(&item_id) {
            if *existing_sequence == sequence {
                return Ok(());
            }
            return Err(AppError::recognition(
                "OpenAI assigned one provider item to more than one local unit.",
            ));
        }

        let unit = self.units_by_sequence.get_mut(&sequence).ok_or_else(|| {
            AppError::recognition("OpenAI item referenced an unknown local recognition unit.")
        })?;
        if let Some(existing_item_id) = unit.provider_item_id.as_deref() {
            if existing_item_id == item_id {
                return Ok(());
            }
            return Err(AppError::recognition(
                "OpenAI assigned more than one provider item to one local recognition unit.",
            ));
        }
        unit.provider_item_id = Some(item_id.clone());
        self.item_to_unit_sequence.insert(item_id, sequence);
        Ok(())
    }

    fn handle_delta(
        &mut self,
        item_id: String,
        delta: String,
        received_at_ms: u64,
    ) -> AppResult<()> {
        if self.model == OpenAiTranscriptionModel::GptTranscribe
            || self.is_recently_finished(&item_id)
        {
            return Ok(());
        }

        let sequence = if let Some(sequence) = self.item_to_unit_sequence.get(&item_id).copied() {
            sequence
        } else if let Some(sequence) = self.bindable_active_unit_sequence() {
            self.bind_item_to_unit(sequence, item_id.clone())?;
            self.replay_pending_item_events(&item_id)?;
            sequence
        } else {
            return self.queue_pending_event(
                item_id,
                PendingTranscriptEvent::Delta {
                    delta,
                    received_at_ms,
                },
            );
        };
        self.apply_delta(sequence, delta, received_at_ms)
    }

    fn apply_delta(&mut self, sequence: u64, delta: String, received_at_ms: u64) -> AppResult<()> {
        let (unit_id, started_at_ms, revision, text) = {
            let Some(unit) = self.units_by_sequence.get_mut(&sequence) else {
                return Ok(());
            };
            if unit.terminal.is_some() || delta.is_empty() {
                return Ok(());
            }
            if unit.live_text.len().saturating_add(delta.len()) > MAX_TRANSCRIPT_BYTES_PER_UNIT {
                return Err(AppError::recognition(
                    "OpenAI Realtime transcript exceeded the per-unit text limit.",
                ));
            }
            unit.revision = unit.revision.checked_add(1).ok_or_else(|| {
                AppError::recognition("Realtime caption revision exceeded the supported range.")
            })?;
            unit.live_text.push_str(&delta);
            (
                unit.unit_id.clone(),
                unit.started_at_ms,
                unit.revision,
                unit.live_text.clone(),
            )
        };
        let caption = self.caption(CaptionEmission {
            unit_id,
            started_at_ms,
            revision,
            text,
            state: CaptionState::Ongoing,
            language: None,
            timestamp_ms: received_at_ms,
        });
        self.ready_events
            .push_back(RecognitionEvent::Caption(caption));
        Ok(())
    }

    fn handle_completed(
        &mut self,
        item_id: String,
        transcript: String,
        detected_languages: Vec<String>,
        received_at_ms: u64,
    ) -> AppResult<()> {
        if self.is_recently_finished(&item_id) {
            return Ok(());
        }
        let Some(sequence) = self.item_to_unit_sequence.get(&item_id).copied() else {
            return self.queue_pending_event(
                item_id,
                PendingTranscriptEvent::Completed {
                    transcript,
                    detected_languages,
                    received_at_ms,
                },
            );
        };
        self.apply_completed(sequence, transcript, detected_languages, received_at_ms)
    }

    fn apply_completed(
        &mut self,
        sequence: u64,
        transcript: String,
        detected_languages: Vec<String>,
        received_at_ms: u64,
    ) -> AppResult<()> {
        if transcript.len() > MAX_TRANSCRIPT_BYTES_PER_UNIT {
            return Err(AppError::recognition(
                "OpenAI Realtime transcript exceeded the per-unit text limit.",
            ));
        }
        let (unit_id, started_at_ms, revision) = {
            let Some(unit) = self.units_by_sequence.get_mut(&sequence) else {
                return Ok(());
            };
            if unit.terminal.is_some() {
                return Ok(());
            }
            unit.revision = unit.revision.checked_add(1).ok_or_else(|| {
                AppError::recognition("Realtime caption revision exceeded the supported range.")
            })?;
            (unit.unit_id.clone(), unit.started_at_ms, unit.revision)
        };
        let terminal = if transcript.trim().is_empty() {
            UnitTerminal::Aborted(RecognitionUnitAbortReason::NoSpeech)
        } else {
            UnitTerminal::Caption(self.caption(CaptionEmission {
                unit_id,
                started_at_ms,
                revision,
                text: transcript,
                state: CaptionState::Completed,
                language: single_detected_language(detected_languages),
                timestamp_ms: received_at_ms,
            }))
        };
        if let Some(unit) = self.units_by_sequence.get_mut(&sequence) {
            unit.terminal = Some(terminal);
        }
        self.release_terminal_units_in_order();
        Ok(())
    }

    fn handle_failed(&mut self, item_id: String, detail: String) -> AppResult<()> {
        if self.is_recently_finished(&item_id) {
            return Ok(());
        }
        let Some(sequence) = self.item_to_unit_sequence.get(&item_id).copied() else {
            return self.queue_pending_event(item_id, PendingTranscriptEvent::Failed { detail });
        };
        self.apply_failed(sequence, detail)
    }

    fn apply_failed(&mut self, sequence: u64, detail: String) -> AppResult<()> {
        let unit = self.units_by_sequence.get_mut(&sequence).ok_or_else(|| {
            AppError::recognition(
                "OpenAI item failure referenced an unknown local recognition unit.",
            )
        })?;
        if unit.terminal.is_none() {
            unit.terminal = Some(UnitTerminal::Aborted(RecognitionUnitAbortReason::Failed {
                detail,
            }));
        }
        self.release_terminal_units_in_order();
        Ok(())
    }

    fn queue_pending_event(
        &mut self,
        item_id: String,
        event: PendingTranscriptEvent,
    ) -> AppResult<()> {
        if !self.pending_by_item.contains_key(&item_id)
            && self.pending_by_item.len() >= MAX_PENDING_PROVIDER_ITEMS
        {
            return Err(AppError::recognition(
                "OpenAI Realtime exceeded the bounded number of uncorrelated provider items.",
            ));
        }
        let pending_transcript_bytes = self
            .pending_transcript_bytes
            .checked_add(event.text_bytes())
            .ok_or_else(|| {
                AppError::recognition("OpenAI Realtime pending text size overflowed.")
            })?;
        if pending_transcript_bytes > MAX_PENDING_TRANSCRIPT_BYTES {
            return Err(AppError::recognition(
                "OpenAI Realtime exceeded the bounded amount of uncorrelated transcript text.",
            ));
        }
        let pending = self.pending_by_item.entry(item_id).or_default();
        if pending.len() >= MAX_PENDING_EVENTS_PER_ITEM {
            return Err(AppError::recognition(
                "OpenAI Realtime exceeded the bounded number of pending events for one item.",
            ));
        }
        pending.push_back(event);
        self.pending_transcript_bytes = pending_transcript_bytes;
        Ok(())
    }

    fn replay_pending_item_events(&mut self, item_id: &str) -> AppResult<()> {
        let Some(sequence) = self.item_to_unit_sequence.get(item_id).copied() else {
            return Ok(());
        };
        let Some(mut pending) = self.pending_by_item.remove(item_id) else {
            return Ok(());
        };
        let replayed_bytes = pending
            .iter()
            .map(PendingTranscriptEvent::text_bytes)
            .sum::<usize>();
        self.pending_transcript_bytes =
            self.pending_transcript_bytes.saturating_sub(replayed_bytes);
        while let Some(event) = pending.pop_front() {
            match event {
                PendingTranscriptEvent::Delta {
                    delta,
                    received_at_ms,
                } => {
                    if self.model == OpenAiTranscriptionModel::GptLiveTranscribe {
                        self.apply_delta(sequence, delta, received_at_ms)?;
                    }
                }
                PendingTranscriptEvent::Completed {
                    transcript,
                    detected_languages,
                    received_at_ms,
                } => {
                    self.apply_completed(sequence, transcript, detected_languages, received_at_ms)?
                }
                PendingTranscriptEvent::Failed { detail } => self.apply_failed(sequence, detail)?,
            }
        }
        Ok(())
    }

    fn bindable_active_unit_sequence(&self) -> Option<u64> {
        if self.committed_unit_order.iter().any(|sequence| {
            self.units_by_sequence
                .get(sequence)
                .is_some_and(|unit| unit.provider_item_id.is_none())
        }) {
            return None;
        }
        self.active_unit_sequence.filter(|sequence| {
            self.units_by_sequence
                .get(sequence)
                .is_some_and(|unit| unit.provider_item_id.is_none())
        })
    }

    fn release_terminal_units_in_order(&mut self) {
        while let Some(sequence) = self.committed_unit_order.front().copied() {
            let is_terminal = self
                .units_by_sequence
                .get(&sequence)
                .is_some_and(|unit| unit.terminal.is_some());
            if !is_terminal {
                break;
            }

            self.committed_unit_order.pop_front();
            let Some(mut unit) = self.units_by_sequence.remove(&sequence) else {
                continue;
            };
            if let Some(item_id) = unit.provider_item_id.take() {
                self.item_to_unit_sequence.remove(&item_id);
                self.remember_finished_item(item_id);
            }
            if let Some(terminal) = unit.terminal.take() {
                match terminal {
                    UnitTerminal::Caption(caption) => self
                        .ready_events
                        .push_back(RecognitionEvent::Caption(caption)),
                    UnitTerminal::Aborted(reason) => {
                        self.ready_events.push_back(RecognitionEvent::UnitAborted {
                            generation: self.context.generation,
                            stream_id: self.context.stream_id.clone(),
                            unit_id: unit.unit_id,
                            reason,
                        });
                    }
                }
            }
        }
    }

    fn expire_overdue_committed_units(&mut self, now: Duration) -> AppResult<()> {
        let mut expired = Vec::new();
        for sequence in self.committed_unit_order.iter().copied() {
            let Some(unit) = self.units_by_sequence.get_mut(&sequence) else {
                continue;
            };
            if unit.terminal.is_some() {
                continue;
            }
            let Some(wait_started_at) = unit.completion_wait_started_at else {
                continue;
            };
            if now.saturating_sub(wait_started_at) < ITEM_COMPLETION_TIMEOUT {
                continue;
            }
            if unit.provider_item_id.is_none() {
                return Err(AppError::recognition_network_retryable(format!(
                    "OpenAI Realtime did not acknowledge a committed audio item within {} seconds; reconnect before sending more audio.",
                    ITEM_COMPLETION_TIMEOUT.as_secs()
                )));
            }
            expired.push(sequence);
        }

        for sequence in expired {
            self.apply_failed(
                sequence,
                format!(
                    "OpenAI Realtime did not complete one recognition item within {} seconds.",
                    ITEM_COMPLETION_TIMEOUT.as_secs()
                ),
            )?;
        }
        Ok(())
    }

    fn has_overdue_committed_unit(&self, now: Duration) -> bool {
        self.committed_unit_order.iter().copied().any(|sequence| {
            self.units_by_sequence.get(&sequence).is_some_and(|unit| {
                unit.terminal.is_none()
                    && unit.completion_wait_started_at.is_some_and(|started_at| {
                        now.saturating_sub(started_at) >= ITEM_COMPLETION_TIMEOUT
                    })
            })
        })
    }

    fn caption(&self, emission: CaptionEmission) -> CaptionSnapshotV2 {
        CaptionSnapshotV2 {
            generation: self.context.generation,
            stream_id: self.context.stream_id.clone(),
            unit_id: Some(emission.unit_id),
            lane: CaptionLane::Source,
            revision: emission.revision,
            text: emission.text,
            state: emission.state,
            language: emission.language,
            source_ref: None,
            unit_started_at_ms: Some(emission.started_at_ms),
            timestamp_ms: emission.timestamp_ms,
        }
    }

    fn is_recently_finished(&self, item_id: &str) -> bool {
        self.recent_finished_items
            .iter()
            .any(|finished| finished == item_id)
    }

    fn remember_finished_item(&mut self, item_id: String) {
        self.recent_finished_items.push_back(item_id);
        while self.recent_finished_items.len() > RECENT_FINISHED_ITEM_LIMIT {
            self.recent_finished_items.pop_front();
        }
    }
}

impl<T: RealtimeTransport> RecognitionAttempt for OpenAiRealtimeAttempt<T> {
    fn start_unit(&mut self, unit_id: String, started_at_ms: u64) -> AppResult<RecognitionEvent> {
        self.ensure_not_stopped()?;
        if self.active_unit_sequence.is_some() {
            return Err(AppError::recognition(
                "Cannot start a Realtime recognition unit before committing the active unit.",
            ));
        }
        if self.units_by_sequence.len() >= MAX_OUTSTANDING_UNITS {
            return Err(AppError::recognition(
                "OpenAI Realtime exceeded the bounded number of outstanding recognition units.",
            ));
        }
        if unit_id.trim().is_empty() {
            return Err(AppError::recognition(
                "Realtime recognition unit ID cannot be empty.",
            ));
        }
        if self
            .units_by_sequence
            .values()
            .any(|unit| unit.unit_id == unit_id)
        {
            return Err(AppError::recognition(format!(
                "Realtime recognition unit ID {unit_id} is already active."
            )));
        }

        let sequence = self.next_unit_sequence;
        self.next_unit_sequence = self.next_unit_sequence.checked_add(1).ok_or_else(|| {
            AppError::recognition(
                "Realtime recognition unit sequence exceeded the supported range.",
            )
        })?;
        self.units_by_sequence.insert(
            sequence,
            RecognitionUnitState {
                unit_id: unit_id.clone(),
                started_at_ms,
                provider_item_id: None,
                revision: 0,
                live_text: String::new(),
                terminal: None,
                completion_wait_started_at: None,
            },
        );
        self.active_unit_sequence = Some(sequence);

        Ok(RecognitionEvent::UnitStarted {
            generation: self.context.generation,
            stream_id: self.context.stream_id.clone(),
            unit_id,
            started_at_ms,
        })
    }

    fn append_audio(&mut self, audio: RecognitionAttemptAudioChunk<'_>) -> AppResult<()> {
        self.ensure_not_stopped()?;
        if self.active_unit_sequence.is_none() {
            return Err(AppError::recognition(
                "Cannot append Realtime audio without an active recognition unit.",
            ));
        }
        let pcm16 = self
            .audio_encoder
            .append(audio.sample_rate_hz, audio.samples)?;
        self.send_audio_bytes(&pcm16)
    }

    fn end_input(&mut self) -> AppResult<()> {
        self.ensure_not_stopped()?;
        let sequence = self.active_unit_sequence.ok_or_else(|| {
            AppError::recognition(
                "Cannot commit Realtime audio without an active recognition unit.",
            )
        })?;

        let final_pcm16 = self.audio_encoder.finish_unit();
        self.send_audio_bytes(&final_pcm16)?;
        self.send_client_event(json!({
            "event_id": format!(
                "vrc-commit-{}-{}-{sequence}",
                self.context.generation, self.context.connection_epoch
            ),
            "type": "input_audio_buffer.commit",
        }))?;

        let wait_started_at = self.clock.now();
        let unit = self.units_by_sequence.get_mut(&sequence).ok_or_else(|| {
            AppError::recognition("OpenAI committed an unknown local recognition unit.")
        })?;
        unit.completion_wait_started_at = Some(wait_started_at);
        self.active_unit_sequence = None;
        self.committed_unit_order.push_back(sequence);
        self.release_terminal_units_in_order();
        Ok(())
    }

    fn drain_events(&mut self, received_at_ms: u64) -> AppResult<Vec<RecognitionEvent>> {
        if self.stopped {
            return Ok(Vec::new());
        }
        let mut transport_drained = false;
        for _ in 0..MAX_SERVER_FRAMES_PER_DRAIN {
            let Some(message) = self.transport.try_receive_text()? else {
                transport_drained = true;
                break;
            };
            self.handle_server_message(&message, received_at_ms)?;
        }
        let now = self.clock.now();
        if transport_drained {
            self.saturated_drains_after_deadline = 0;
            self.expire_overdue_committed_units(now)?;
        } else if self.has_overdue_committed_unit(now) {
            self.saturated_drains_after_deadline =
                self.saturated_drains_after_deadline.saturating_add(1);
            if self.saturated_drains_after_deadline >= MAX_SATURATED_DRAINS_AFTER_DEADLINE {
                self.saturated_drains_after_deadline = 0;
                self.expire_overdue_committed_units(now)?;
            }
        } else {
            self.saturated_drains_after_deadline = 0;
        }
        Ok(self.ready_events.drain(..).collect())
    }

    fn stop(&mut self) -> AppResult<()> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        self.active_unit_sequence = None;
        self.units_by_sequence.clear();
        self.committed_unit_order.clear();
        self.item_to_unit_sequence.clear();
        self.pending_by_item.clear();
        self.pending_transcript_bytes = 0;
        self.ready_events.clear();
        self.recent_finished_items.clear();
        self.saturated_drains_after_deadline = 0;
        self.audio_encoder.reset_unit();
        self.transport.close()
    }
}

fn normalize_languages(languages: Vec<String>) -> AppResult<Vec<String>> {
    if languages.is_empty() {
        return Err(AppError::recognition(
            "At least one expected language is required for Realtime transcription.",
        ));
    }
    let mut normalized = Vec::with_capacity(languages.len());
    let mut seen = HashSet::with_capacity(languages.len());
    for language in languages {
        let language = language.trim().to_string();
        if language.is_empty() {
            return Err(AppError::recognition(
                "Realtime transcription languages cannot contain an empty value.",
            ));
        }
        if !seen.insert(language.to_ascii_lowercase()) {
            return Err(AppError::recognition(format!(
                "Realtime transcription language {language} is duplicated."
            )));
        }
        normalized.push(language);
    }
    Ok(normalized)
}

fn session_update(
    model: OpenAiTranscriptionModel,
    languages: &[String],
    generation: u64,
    connection_epoch: u64,
) -> Value {
    json!({
        "event_id": format!("vrc-session-update-{generation}-{connection_epoch}"),
        "type": "session.update",
        "session": {
            "type": "transcription",
            "audio": {
                "input": {
                    "format": {
                        "type": "audio/pcm",
                        "rate": REALTIME_PCM_SAMPLE_RATE_HZ,
                    },
                    "transcription": {
                        "model": model.as_str(),
                        "languages": languages,
                    },
                    "turn_detection": null,
                }
            }
        }
    })
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ServerEvent {
    #[serde(rename = "session.updated")]
    SessionUpdated,
    #[serde(rename = "input_audio_buffer.committed")]
    BufferCommitted { item_id: String },
    #[serde(rename = "conversation.item.input_audio_transcription.delta")]
    TranscriptDelta { item_id: String, delta: String },
    #[serde(rename = "conversation.item.input_audio_transcription.completed")]
    TranscriptCompleted {
        item_id: String,
        transcript: String,
        #[serde(default)]
        languages: Vec<DetectedLanguage>,
    },
    #[serde(rename = "conversation.item.input_audio_transcription.failed")]
    TranscriptFailed {
        item_id: String,
        error: ProviderError,
    },
    #[serde(rename = "error")]
    Error { error: ProviderError },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
pub(crate) struct ProviderError {
    #[serde(rename = "message", default)]
    _message: Option<IgnoredAny>,
    #[serde(rename = "type", default)]
    kind: Option<Value>,
    #[serde(default)]
    code: Option<Value>,
    #[serde(rename = "param", default)]
    _param: Option<IgnoredAny>,
    #[serde(rename = "event_id", default)]
    _event_id: Option<IgnoredAny>,
}

impl ProviderError {
    pub(crate) fn classification(&self) -> ProviderFailureClass {
        // `code` is more specific than `type`, especially for 429 errors:
        // OpenAI documents that quota/billing failures can share the broader
        // `insufficient_quota` type. Never inspect the human-readable message.
        match self.code.as_ref().and_then(Value::as_str) {
            Some("invalid_api_key" | "authentication_error") => {
                return ProviderFailureClass::Authentication;
            }
            Some("permission_denied" | "access_terminated") => {
                return ProviderFailureClass::PermissionDenied;
            }
            Some(
                "credit_balance_exhausted"
                | "insufficient_quota"
                | "organization_spend_limit_exceeded"
                | "project_spend_limit_exceeded"
                | "organization_usage_limit_exceeded",
            ) => return ProviderFailureClass::UsageLimit,
            Some("rate_limit_exceeded") => return ProviderFailureClass::RateLimited,
            Some("server_error" | "websocket_connection_limit_reached") => {
                return ProviderFailureClass::ServiceUnavailable;
            }
            Some("invalid_value" | "invalid_request_error") => {
                return ProviderFailureClass::InvalidRequest;
            }
            Some(_) | None => {}
        }

        match self.kind.as_ref().and_then(Value::as_str) {
            Some("authentication_error") => ProviderFailureClass::Authentication,
            Some("permission_error") => ProviderFailureClass::PermissionDenied,
            Some("invalid_request_error") => ProviderFailureClass::InvalidRequest,
            Some("rate_limit_error") => ProviderFailureClass::RateLimited,
            Some("insufficient_quota") => ProviderFailureClass::UsageLimit,
            Some("api_error" | "server_error" | "overloaded_error") => {
                ProviderFailureClass::ServiceUnavailable
            }
            Some(_) | None => ProviderFailureClass::Unknown,
        }
    }
}

pub(crate) fn openai_provider_failure(class: ProviderFailureClass) -> AppError {
    let message = match class {
        ProviderFailureClass::Authentication => {
            "OpenAI rejected the Realtime transcription credentials."
        }
        ProviderFailureClass::PermissionDenied => "OpenAI denied access to Realtime transcription.",
        ProviderFailureClass::InvalidRequest => {
            "OpenAI rejected the Realtime transcription request."
        }
        ProviderFailureClass::RateLimited => {
            "OpenAI temporarily rate-limited Realtime transcription."
        }
        ProviderFailureClass::UsageLimit => {
            "OpenAI Realtime transcription reached a credit, spend, or usage limit."
        }
        ProviderFailureClass::ServiceUnavailable => {
            "OpenAI Realtime transcription is temporarily unavailable."
        }
        ProviderFailureClass::Unknown => "OpenAI Realtime transcription failed.",
    };
    AppError::recognition_provider(class, message)
}

#[derive(Deserialize)]
struct DetectedLanguage {
    code: String,
}

fn single_detected_language(languages: Vec<String>) -> Option<String> {
    if languages.len() != 1 {
        return None;
    }

    languages
        .into_iter()
        .next()
        .map(|language| language.trim().to_string())
        .filter(|language| !language.is_empty())
}

#[cfg(test)]
#[path = "realtime_tests.rs"]
mod tests;
