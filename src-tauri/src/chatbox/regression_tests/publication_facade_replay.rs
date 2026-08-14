use super::super::layout::{
    ChatboxLayoutError, PreparedChatboxText, prepare_completed_pages, prepare_live_viewport,
};
use super::super::text_pacing::{ChatboxTextPacer, Clock};
use super::super::transport::{ChatboxSendReceipt, ChatboxTransport};
use super::super::{ChatboxPublication, PublicationObservationOutcome, PublisherCloseReason};
use super::support::{
    PORTABLE_CORPUS_JSON, first_oversized_grapheme_utf16_units, has_test_target, required_string,
};
use crate::caption::{
    ActiveCaptionStream, CAPTION_AGGREGATE_CONTRACT_VERSION, CaptionAggregateChange,
    CaptionAggregateSnapshot, CaptionAggregateUpdate, CaptionLane, CaptionSnapshot, CaptionState,
};
use crate::caption_pipeline::ResolvedPublicationTiming;
use crate::error::{AppError, AppResult};
use crate::events::DiagnosticUpdate;
use crate::generation_fence::GenerationFence;
use serde_json::Value;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SHARED_FACADE_REPLAY_CASE_IDS: [&str; 6] = [
    "LINES-CR-BASIC",
    "LIMIT-ASCII-OVER",
    "LINES-NINE-LF",
    "KINSOKU-CLOSE-PROBE",
    "MIX-THREE-WRITING-SYSTEMS",
    "PRODUCT-EMOJI-BILINGUAL",
];

const LIVE_ONLY_FACADE_REPLAY_CASE_IDS: [&str; 2] =
    ["LIVE-NATURAL-WORD-BOUNDARY", "LIVE-OVERSIZED-OLD-GRAPHEME"];

#[test]
fn completed_facade_replays_layered_corpus_cases_without_rewriting_pages() -> AppResult<()> {
    let corpus = parse_corpus()?;
    let (publication, receiver) = start_corpus_publication(ResolvedPublicationTiming::Completed)?;

    for (index, case_id) in SHARED_FACADE_REPLAY_CASE_IDS.iter().enumerate() {
        let case = corpus_case(&corpus, case_id)?;
        assert!(has_test_target(case, "completed-pagination").map_err(AppError::state)?);
        let payload = required_string(case, "payload").map_err(AppError::state)?;
        let expected = prepare_completed_pages(payload).map_err(|error| {
            AppError::state(format!(
                "Replay case {case_id} unexpectedly failed: {error:?}"
            ))
        })?;
        assert!(
            !expected.is_empty(),
            "Replay case emitted no pages: {case_id}"
        );

        let revision = u64::try_from(index + 1)
            .map_err(|_| AppError::state("Corpus replay revision overflowed."))?;
        assert_eq!(
            publication.try_observe(&corpus_update(revision, case_id, payload))?,
            PublicationObservationOutcome::Handled
        );
        for expected_page in expected {
            assert_eq!(
                receive_corpus_text(&receiver)?,
                expected_page.as_str(),
                "Completed facade rewrote a prepared corpus page: {case_id}"
            );
        }
    }

    close_corpus_publication(&publication)?;
    assert_no_corpus_text(&receiver);
    Ok(())
}

#[test]
fn live_facade_replays_layered_corpus_cases_without_rewriting_viewports() -> AppResult<()> {
    let corpus = parse_corpus()?;
    let (publication, receiver) = start_corpus_publication(ResolvedPublicationTiming::LiveUnit {
        observation_window_ms: 1_000,
    })?;
    let case_ids = SHARED_FACADE_REPLAY_CASE_IDS
        .iter()
        .chain(LIVE_ONLY_FACADE_REPLAY_CASE_IDS.iter());

    for (index, case_id) in case_ids.enumerate() {
        let case = corpus_case(&corpus, case_id)?;
        assert!(has_test_target(case, "live-window").map_err(AppError::state)?);
        let payload = required_string(case, "payload").map_err(AppError::state)?;
        let expected = prepare_live_viewport(payload)
            .map_err(|error| {
                AppError::state(format!(
                    "Replay case {case_id} unexpectedly failed: {error:?}"
                ))
            })?
            .ok_or_else(|| AppError::state(format!("Replay case had no viewport: {case_id}")))?;

        let revision = u64::try_from(index + 1)
            .map_err(|_| AppError::state("Corpus replay revision overflowed."))?;
        assert_eq!(
            publication.try_observe(&corpus_update(revision, case_id, payload))?,
            PublicationObservationOutcome::Handled
        );
        assert_eq!(
            receive_corpus_text(&receiver)?,
            expected.as_str(),
            "Live facade rewrote a prepared corpus viewport: {case_id}"
        );
    }

    close_corpus_publication(&publication)?;
    assert_no_corpus_text(&receiver);
    Ok(())
}

#[test]
fn oversized_newest_egc_is_reported_and_never_reaches_either_facade_transport() -> AppResult<()> {
    let corpus = parse_corpus()?;
    let case_id = "LIVE-OVERSIZED-NEW-GRAPHEME";
    let case = corpus_case(&corpus, case_id)?;
    let payload = required_string(case, "payload").map_err(AppError::state)?;
    let utf16_units = first_oversized_grapheme_utf16_units(payload).ok_or_else(|| {
        AppError::state("Oversized replay case no longer contains an oversized EGC.")
    })?;
    assert_eq!(
        prepare_completed_pages(payload),
        Err(ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units })
    );
    assert_eq!(
        prepare_live_viewport(payload),
        Err(ChatboxLayoutError::GraphemeExceedsInputBudget { utf16_units })
    );

    for timing in [
        ResolvedPublicationTiming::Completed,
        ResolvedPublicationTiming::LiveUnit {
            observation_window_ms: 1_000,
        },
    ] {
        let (publication, receiver, diagnostics) =
            start_corpus_publication_with_diagnostics(timing)?;
        assert_eq!(
            publication.try_observe(&corpus_update(1, case_id, payload))?,
            PublicationObservationOutcome::Handled
        );
        diagnostics
            .recv_timeout(Duration::from_secs(1))
            .map_err(|error| {
                AppError::runtime(format!("Facade did not report the oversized EGC: {error}"))
            })?;
        assert_no_corpus_text(&receiver);
        close_corpus_publication(&publication)?;
    }

    Ok(())
}

struct AdvancingClock {
    now: Mutex<Instant>,
}

impl AdvancingClock {
    fn new() -> Self {
        Self {
            now: Mutex::new(Instant::now()),
        }
    }
}

impl Clock for AdvancingClock {
    fn now(&self) -> Instant {
        self.now
            .lock()
            .map(|now| *now)
            .unwrap_or_else(|poisoned| *poisoned.into_inner())
    }

    fn sleep(&self, duration: Duration) {
        let mut now = match self.now.lock() {
            Ok(now) => now,
            Err(poisoned) => poisoned.into_inner(),
        };
        *now += duration;
    }
}

struct CorpusRecordingTransport {
    texts: mpsc::Sender<String>,
}

impl ChatboxTransport for CorpusRecordingTransport {
    fn send_text(&self, text: &PreparedChatboxText) -> AppResult<ChatboxSendReceipt> {
        self.texts
            .send(text.as_str().to_string())
            .map_err(|_| AppError::state("Corpus transport receiver was dropped."))?;
        Ok(ChatboxSendReceipt {
            target: "corpus-recording".to_string(),
            byte_count: text.as_str().len(),
        })
    }

    fn send_typing(&self, _is_typing: bool) -> AppResult<()> {
        Ok(())
    }
}

fn parse_corpus() -> AppResult<Value> {
    serde_json::from_str(PORTABLE_CORPUS_JSON)
        .map_err(|error| AppError::state(format!("Chatbox corpus was invalid JSON: {error}")))
}

fn corpus_case<'a>(corpus: &'a Value, case_id: &str) -> AppResult<&'a Value> {
    corpus["cases"]
        .as_array()
        .and_then(|cases| {
            cases
                .iter()
                .find(|case| case["case_id"].as_str() == Some(case_id))
        })
        .ok_or_else(|| AppError::state(format!("Chatbox corpus case was missing: {case_id}")))
}

fn start_corpus_publication(
    timing: ResolvedPublicationTiming,
) -> AppResult<(ChatboxPublication, Receiver<String>)> {
    start_corpus_publication_with_reporter(timing, Arc::new(|_| {}))
}

fn start_corpus_publication_with_diagnostics(
    timing: ResolvedPublicationTiming,
) -> AppResult<(ChatboxPublication, Receiver<String>, Receiver<()>)> {
    let (diagnostics, diagnostic_receiver) = mpsc::channel();
    let reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync> = Arc::new(move |_| {
        let _ = diagnostics.send(());
    });
    let (publication, receiver) = start_corpus_publication_with_reporter(timing, reporter)?;
    Ok((publication, receiver, diagnostic_receiver))
}

fn start_corpus_publication_with_reporter(
    timing: ResolvedPublicationTiming,
    reporter: Arc<dyn Fn(DiagnosticUpdate) + Send + Sync>,
) -> AppResult<(ChatboxPublication, Receiver<String>)> {
    let (texts, receiver) = mpsc::channel();
    let transport: Arc<dyn ChatboxTransport> = Arc::new(CorpusRecordingTransport { texts });
    let fence = GenerationFence::new();
    let publication = ChatboxPublication::start_with_transport(
        transport,
        ChatboxTextPacer::with_clock(Arc::new(AdvancingClock::new())),
        1,
        fence.committer(),
        timing,
        reporter,
    )?;
    Ok((publication, receiver))
}

fn corpus_update(revision: u64, case_id: &str, text: &str) -> CaptionAggregateUpdate {
    let stream_id = "corpus-replay-1".to_string();
    let caption = CaptionSnapshot {
        generation: 1,
        stream_id: stream_id.clone(),
        unit_id: Some(case_id.to_string()),
        lane: CaptionLane::Source,
        revision,
        text: text.to_string(),
        state: CaptionState::Completed,
        language: None,
        source_ref: None,
        unit_started_at_ms: Some(revision),
        timestamp_ms: revision,
    };
    CaptionAggregateUpdate {
        snapshot: CaptionAggregateSnapshot {
            contract_version: CAPTION_AGGREGATE_CONTRACT_VERSION,
            snapshot_revision: revision,
            active_stream: Some(ActiveCaptionStream {
                generation: 1,
                stream_id,
            }),
            open_source_units: Vec::new(),
            captions: vec![caption.clone()],
            translation_units: Vec::new(),
        },
        change: CaptionAggregateChange::CaptionAccepted(caption),
    }
}

fn receive_corpus_text(receiver: &Receiver<String>) -> AppResult<String> {
    receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|error| {
            AppError::runtime(format!(
                "Corpus replay transport did not receive text: {error}"
            ))
        })
}

fn assert_no_corpus_text(receiver: &Receiver<String>) {
    assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
}

fn close_corpus_publication(publication: &ChatboxPublication) -> AppResult<()> {
    publication.request_close(PublisherCloseReason::Stop)?;
    publication.join()
}
