//! Generation-scoped selection and ordering for Completed publication.
//!
//! The Caption Aggregate owns correlation and terminal Translation state. This
//! coordinator consumes only its accepted changes, retains Source admission
//! order while Translation is pending, and resolves reserved positions in the
//! existing bounded publisher as each exact unit becomes terminal.

use super::common::{PublisherCloseReason, PublisherSubmitOutcome};
use super::completed::{
    CompletedChatboxPublisher, CompletedPublicationContent, CompletedPublisherInput,
};
use crate::caption::{
    CaptionAggregateChange, CaptionAggregateUpdate, CaptionLane, CaptionSnapshot, CaptionState,
    SourceSnapshotRef, TranslationUnitSnapshot,
};
use crate::config::ContentSelection;
use crate::error::{AppError, AppResult};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(super) struct CompletedContentCoordinator {
    selection: ContentSelection,
    generation_id: u64,
    stream_id: String,
    publisher: CompletedChatboxPublisher,
    state: Arc<Mutex<CoordinatorState>>,
}

#[derive(Default)]
struct CoordinatorState {
    closed: bool,
    units: VecDeque<CoordinatedUnit>,
}

struct CoordinatedUnit {
    unit_id: String,
    source: Option<CoordinatedSource>,
}

struct CoordinatedSource {
    reference: SourceSnapshotRef,
    text: String,
}

impl CompletedContentCoordinator {
    pub(super) fn new(
        selection: ContentSelection,
        generation_id: u64,
        stream_id: String,
        publisher: CompletedChatboxPublisher,
    ) -> AppResult<Self> {
        if selection == ContentSelection::SourceOnly {
            return Err(AppError::state(
                "Translation publication coordinator requires selected Translation content.",
            ));
        }
        Ok(Self {
            selection,
            generation_id,
            stream_id,
            publisher,
            state: Arc::new(Mutex::new(CoordinatorState::default())),
        })
    }

    pub(super) fn try_observe(
        &self,
        update: &CaptionAggregateUpdate,
    ) -> AppResult<PublisherSubmitOutcome> {
        let inputs = {
            let mut state = self.lock_state()?;
            if state.closed {
                return Ok(PublisherSubmitOutcome::Closed);
            }

            let mut inputs = Vec::new();
            match &update.change {
                CaptionAggregateChange::SourceUnitOpened(unit) => {
                    if !state
                        .units
                        .iter()
                        .any(|current| current.unit_id == unit.unit_id)
                    {
                        state.units.push_back(CoordinatedUnit {
                            unit_id: unit.unit_id.clone(),
                            source: None,
                        });
                    }
                    inputs.push(CompletedPublisherInput::Started {
                        unit_id: unit.unit_id.clone(),
                    });
                    inputs.push(CompletedPublisherInput::PublicationReserved {
                        unit_id: unit.unit_id.clone(),
                    });
                }
                CaptionAggregateChange::SourceUnitAborted { unit_id } => {
                    if let Some(position) = state
                        .units
                        .iter()
                        .position(|current| current.unit_id == *unit_id)
                    {
                        let _ = state.units.remove(position);
                    }
                    inputs.push(CompletedPublisherInput::Aborted {
                        unit_id: unit_id.clone(),
                    });
                }
                CaptionAggregateChange::CaptionAccepted(caption)
                    if caption.lane == CaptionLane::Source
                        && caption.state == CaptionState::Completed =>
                {
                    if self.accept_source(&mut state, caption)
                        && let Some(unit_id) = caption.unit_id.as_ref()
                    {
                        inputs.push(CompletedPublisherInput::SourceResolved {
                            unit_id: unit_id.clone(),
                        });
                    }
                }
                CaptionAggregateChange::CaptionAccepted(caption)
                    if caption.lane == CaptionLane::Translation
                        && caption.state == CaptionState::Completed =>
                {
                    if let Some(input) = self.accept_translation(&mut state, caption) {
                        inputs.push(input);
                    }
                }
                CaptionAggregateChange::TranslationFailed(translation) => {
                    if let Some(input) = self.accept_failure(&mut state, translation) {
                        inputs.push(input);
                    }
                }
                CaptionAggregateChange::CaptionAccepted(_) => {}
            }
            inputs
        };

        let mut outcome = self.publisher.admission_outcome()?;
        for input in inputs {
            let submitted = self.publisher.try_submit(input)?;
            if submitted == PublisherSubmitOutcome::Closed {
                outcome = PublisherSubmitOutcome::Closed;
                break;
            }
        }
        Ok(outcome)
    }

    pub(super) fn request_close(&self, reason: PublisherCloseReason) -> AppResult<()> {
        {
            let mut state = self.lock_state()?;
            state.closed = true;
            state.units.clear();
        }
        self.publisher.request_close(reason)
    }

    pub(super) fn join(&self) -> AppResult<()> {
        self.publisher.join()
    }

    fn accept_source(&self, state: &mut CoordinatorState, caption: &CaptionSnapshot) -> bool {
        if caption.generation != self.generation_id
            || caption.stream_id != self.stream_id
            || caption.source_ref.is_some()
        {
            return false;
        }
        let Some(unit_id) = caption.unit_id.as_ref() else {
            return false;
        };
        let Some(unit) = state
            .units
            .iter_mut()
            .find(|current| current.unit_id == *unit_id)
        else {
            return false;
        };
        if unit.source.is_none() {
            unit.source = Some(CoordinatedSource {
                reference: SourceSnapshotRef {
                    generation: caption.generation,
                    stream_id: caption.stream_id.clone(),
                    unit_id: unit_id.clone(),
                    revision: caption.revision,
                },
                text: caption.text.clone(),
            });
            return true;
        }
        false
    }

    fn accept_translation(
        &self,
        state: &mut CoordinatorState,
        caption: &CaptionSnapshot,
    ) -> Option<CompletedPublisherInput> {
        if caption.generation != self.generation_id || caption.stream_id != self.stream_id {
            return None;
        }
        let source_ref = caption.source_ref.as_ref()?;
        if caption.generation != source_ref.generation
            || caption.stream_id != source_ref.stream_id
            || caption.unit_id.as_deref() != Some(source_ref.unit_id.as_str())
        {
            return None;
        }
        let position = matching_unit_position(state, source_ref)?;
        let unit = state.units.remove(position)?;
        let source = unit.source?;
        let content = match self.selection {
            ContentSelection::TranslationOnly => {
                CompletedPublicationContent::Monolingual(caption.text.clone())
            }
            ContentSelection::Bilingual => CompletedPublicationContent::Bilingual {
                source: source.text,
                translation: caption.text.clone(),
            },
            ContentSelection::SourceOnly => return None,
        };
        Some(CompletedPublisherInput::ContentReady {
            unit_id: unit.unit_id,
            content,
        })
    }

    fn accept_failure(
        &self,
        state: &mut CoordinatorState,
        translation: &TranslationUnitSnapshot,
    ) -> Option<CompletedPublisherInput> {
        let TranslationUnitSnapshot::Failed { source_ref, .. } = translation else {
            return None;
        };
        if source_ref.generation != self.generation_id || source_ref.stream_id != self.stream_id {
            return None;
        }
        let position = matching_unit_position(state, source_ref)?;
        let unit = state.units.remove(position)?;
        let source = unit.source?;
        match self.selection {
            ContentSelection::TranslationOnly => {
                Some(CompletedPublisherInput::PublicationOmitted {
                    unit_id: unit.unit_id,
                })
            }
            ContentSelection::Bilingual => Some(CompletedPublisherInput::ContentReady {
                unit_id: unit.unit_id,
                content: CompletedPublicationContent::Monolingual(source.text),
            }),
            ContentSelection::SourceOnly => None,
        }
    }

    fn lock_state(&self) -> AppResult<std::sync::MutexGuard<'_, CoordinatorState>> {
        self.state
            .lock()
            .map_err(|_| AppError::state("Completed content coordinator lock was poisoned."))
    }
}

fn matching_unit_position(
    state: &CoordinatorState,
    source_ref: &SourceSnapshotRef,
) -> Option<usize> {
    state.units.iter().position(|unit| {
        unit.source
            .as_ref()
            .is_some_and(|source| source.reference == *source_ref)
    })
}
