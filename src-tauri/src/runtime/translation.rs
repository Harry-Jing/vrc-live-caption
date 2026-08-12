//! Immutable Translation preparation and generation-owned provider lifecycle.

use crate::caption::{CaptionAggregateUpdate, ReservedCompletedSource, TranslationFailureReason};
use crate::config::TranslationConfig;
use crate::error::AppResult;
use crate::runtime_control::RuntimeGenerationCredentialSnapshot;
use crate::translation::{
    BoundTranslationModule, BoundTranslationParts, TranslationModule, TranslationOutcomeReceiver,
    TranslationSubmissionRejection, TranslationTerminalOutcome,
};
use std::sync::mpsc::TryRecvError;

/// A provider owner and non-secret metadata prepared at the desktop boundary.
///
/// Keeping these values inseparable prevents Runtime from presenting one
/// endpoint or credential while using another for the active generation.
pub(crate) struct PreparedTranslation {
    selection: TranslationConfig,
    module: TranslationModule,
    outcomes: TranslationOutcomeReceiver,
    credential: RuntimeGenerationCredentialSnapshot,
}

impl PreparedTranslation {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "GitHub issue #25 activates the prepared Translation owner at desktop Start."
        )
    )]
    pub(crate) fn cloud(binding: BoundTranslationModule) -> Self {
        let BoundTranslationParts {
            selection,
            credential_id,
            credential_storage,
            credential_display_suffix,
            credential_revision,
            module,
            outcomes,
        } = binding.into_parts();
        Self {
            selection,
            module,
            outcomes,
            credential: RuntimeGenerationCredentialSnapshot {
                id: credential_id,
                storage: credential_storage,
                display_suffix: credential_display_suffix,
                revision: credential_revision,
            },
        }
    }

    pub(crate) fn selection(&self) -> &TranslationConfig {
        &self.selection
    }

    pub(crate) fn credential(&self) -> &RuntimeGenerationCredentialSnapshot {
        &self.credential
    }

    pub(super) fn into_generation(self) -> GenerationTranslation {
        GenerationTranslation {
            module: self.module,
            outcomes: self.outcomes,
            degradation: None,
        }
    }
}

pub(super) struct GenerationTranslation {
    module: TranslationModule,
    outcomes: TranslationOutcomeReceiver,
    degradation: Option<TranslationFailureReason>,
}

pub(super) enum TranslationAdmission {
    Submitted,
    Rejected {
        reason: TranslationFailureReason,
        update: Option<Box<CaptionAggregateUpdate>>,
    },
}

impl GenerationTranslation {
    pub(super) fn submit(
        &mut self,
        reservation: ReservedCompletedSource,
    ) -> AppResult<TranslationAdmission> {
        match self.module.try_submit(reservation) {
            Ok(()) => Ok(TranslationAdmission::Submitted),
            Err(rejection) => self.reject(rejection),
        }
    }

    fn reject(
        &mut self,
        rejection: TranslationSubmissionRejection,
    ) -> AppResult<TranslationAdmission> {
        let reason = rejection.reason();
        self.record_degradation(reason);
        Ok(TranslationAdmission::Rejected {
            reason,
            update: rejection.fail()?.map(Box::new),
        })
    }

    pub(super) fn try_next(&self) -> Result<TranslationTerminalOutcome, TryRecvError> {
        self.outcomes.try_recv()
    }

    pub(super) fn record_degradation(&mut self, reason: TranslationFailureReason) {
        if self.degradation.is_none() {
            self.degradation = Some(reason);
        }
    }

    pub(super) const fn degradation(&self) -> Option<TranslationFailureReason> {
        self.degradation
    }

    pub(super) fn stop(&mut self) -> AppResult<()> {
        self.module.stop()
    }
}
