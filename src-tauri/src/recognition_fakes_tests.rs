use super::*;

fn context() -> ScriptedRecognitionContext {
    ScriptedRecognitionContext {
        generation: 7,
        stream_id: "recognition-7-1".to_string(),
        language: Some("en".to_string()),
        model: "gpt-live-transcribe".to_string(),
    }
}

#[test]
fn scripted_unit_emits_monotonic_full_snapshots_and_one_completion() {
    let events = ScriptedRecognitionAdapter::new(context()).script_unit(
        "unit-live",
        200,
        &[
            ScriptedText::new("I", 220),
            ScriptedText::new("I am speaking", 260),
        ],
        ScriptedText::new("I am speaking now.", 320),
    );

    assert!(matches!(
        &events[0],
        RecognitionEvent::UnitStarted {
            generation: 7,
            stream_id,
            unit_id,
            started_at_ms: 200,
        } if stream_id == "recognition-7-1" && unit_id == "unit-live"
    ));
    let captions = events
        .iter()
        .filter_map(|event| match event {
            RecognitionEvent::Caption(caption) => Some(caption),
            RecognitionEvent::UnitStarted { .. } | RecognitionEvent::UnitEnded { .. } => None,
        })
        .collect::<Vec<_>>();
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
        caption.provider == "openai"
            && caption.model == "gpt-live-transcribe"
            && caption.unit_id.as_deref() == Some("unit-live")
            && caption.unit_started_at_ms == Some(200)
    }));
}

#[test]
fn scripted_end_emits_an_explicit_terminal_event_without_a_caption() {
    let events = ScriptedRecognitionAdapter::new(context()).script_ended(
        "unit-empty",
        400,
        RecognitionEndReason::NoSpeech,
    );

    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[1],
        RecognitionEvent::UnitEnded {
            generation: 7,
            stream_id,
            unit_id,
            reason: RecognitionEndReason::NoSpeech,
        } if stream_id == "recognition-7-1" && unit_id == "unit-empty"
    ));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, RecognitionEvent::Caption(_)))
    );
}
