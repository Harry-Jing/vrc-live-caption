use super::*;
use crate::caption_session::{CaptionLane, CaptionState};
use crate::error::{AppError, AppResult};

fn context() -> ScriptedRecognitionContext {
    ScriptedRecognitionContext {
        generation: 7,
        stream_id: "recognition-7-1".to_string(),
        language: Some("en".to_string()),
        provider: "mock".to_string(),
        model: "scripted-test".to_string(),
    }
}

#[test]
fn bounded_adapter_emits_one_full_completed_source_caption_for_a_real_unit() -> AppResult<()> {
    let events = FakeBoundedRecognitionAdapter::new(context()).script_completed(
        "unit-bounded",
        100,
        ScriptedText::new("bounded full text", 180),
    );

    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[0],
        RecognitionEvent::UnitStarted {
            generation: 7,
            stream_id,
            unit_id,
            started_at_ms: 100,
        } if stream_id == "recognition-7-1" && unit_id == "unit-bounded"
    ));
    let caption = match &events[1] {
        RecognitionEvent::Caption(caption) => caption,
        RecognitionEvent::UnitStarted { .. } => {
            return Err(AppError::state("Bounded adapter did not emit a caption."));
        }
    };
    assert_eq!(caption.generation, 7);
    assert_eq!(caption.stream_id, "recognition-7-1");
    assert_eq!(caption.unit_id.as_deref(), Some("unit-bounded"));
    assert_eq!(caption.lane, CaptionLane::Source);
    assert_eq!(caption.revision, 1);
    assert_eq!(caption.text, "bounded full text");
    assert_eq!(caption.state, CaptionState::Completed);
    assert_eq!(caption.unit_started_at_ms, Some(100));
    assert_eq!(caption.timestamp_ms, 180);
    Ok(())
}

#[test]
fn unitful_ongoing_completed_adapter_emits_monotonic_full_snapshots_and_one_completion() {
    let events = FakeOngoingCompletedRecognitionAdapter::new(context()).script_unit(
        "unit-live",
        200,
        &[
            ScriptedText::new("I", 220),
            ScriptedText::new("I am speaking", 260),
        ],
        ScriptedText::new("I am speaking now.", 320),
    );

    let captions = events
        .iter()
        .filter_map(|event| match event {
            RecognitionEvent::Caption(caption) => Some(caption),
            RecognitionEvent::UnitStarted { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(captions.len(), 3);
    assert_eq!(
        captions
            .iter()
            .map(|caption| (caption.revision, caption.text.as_str(), caption.state))
            .collect::<Vec<_>>(),
        vec![
            (1, "I", CaptionState::Ongoing),
            (2, "I am speaking", CaptionState::Ongoing),
            (3, "I am speaking now.", CaptionState::Completed),
        ]
    );
    assert!(captions.iter().all(|caption| {
        caption.unit_id.as_deref() == Some("unit-live") && caption.unit_started_at_ms == Some(200)
    }));
}

#[test]
fn unitless_ongoing_only_adapter_never_fabricates_a_unit_or_completion() -> AppResult<()> {
    let events = FakeOngoingOnlyRecognitionAdapter::new(context()).script_stream(&[
        ScriptedText::new("continuous", 410),
        ScriptedText::new("continuous full replacement", 440),
    ]);

    assert_eq!(events.len(), 2);
    for (index, event) in events.iter().enumerate() {
        let caption = match event {
            RecognitionEvent::Caption(caption) => caption,
            RecognitionEvent::UnitStarted { .. } => {
                return Err(AppError::state(
                    "Ongoing-only adapter fabricated a unit lifecycle event.",
                ));
            }
        };
        assert_eq!(caption.revision, index as u64 + 1);
        assert!(caption.unit_id.is_none());
        assert!(caption.unit_started_at_ms.is_none());
        assert_eq!(caption.state, CaptionState::Ongoing);
    }
    assert_eq!(
        events
            .iter()
            .filter_map(|event| match event {
                RecognitionEvent::Caption(caption) => Some(caption.text.as_str()),
                RecognitionEvent::UnitStarted { .. } => None,
            })
            .collect::<Vec<_>>(),
        vec!["continuous", "continuous full replacement"]
    );
    Ok(())
}
