use super::*;

pub(crate) enum TestTranslationResult {
    Completed(String),
    Failed(TranslationFailureClass),
    Blocked,
}

#[derive(Clone)]
pub(crate) struct TestTranslationControl {
    shared: Arc<TestTranslationAdapterShared>,
}

impl TestTranslationControl {
    pub(crate) fn wait_until_called(&self, expected: usize, timeout: Duration) -> AppResult<()> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| AppError::state("Translation test Adapter lock was poisoned."))?;
        while state.calls < expected {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppError::state(
                    "Translation test Adapter was not called in time.",
                ));
            }
            let (next, wait) = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| AppError::state("Translation test Adapter wait was poisoned."))?;
            state = next;
            if wait.timed_out() && state.calls < expected {
                return Err(AppError::state(
                    "Translation test Adapter was not called in time.",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn complete_blocked(&self, result: Result<String, TranslationFailureClass>) {
        let completion = self
            .shared
            .state
            .lock()
            .ok()
            .and_then(|mut state| state.blocked_completion.take());
        if let Some(completion) = completion {
            completion.finish(result.map_err(test_adapter_failure));
        }
    }
}

pub(crate) fn translation_module_for_test(
    selection: TranslationConfig,
    credential: ResolvedCredential,
    credential_revision: u64,
    results: impl IntoIterator<Item = TestTranslationResult>,
) -> AppResult<(BoundTranslationModule, TestTranslationControl)> {
    if credential.id != openai_responses::required_credential_id(&selection.endpoint) {
        return Err(AppError::state(
            "Test Translation credential does not match its endpoint.",
        ));
    }
    let credential_id = credential.id;
    let credential_storage = credential.storage;
    let credential_display_suffix = credential.display_suffix;
    let shared = Arc::new(TestTranslationAdapterShared {
        state: Mutex::new(TestTranslationAdapterState {
            results: results.into_iter().collect(),
            calls: 0,
            blocked_completion: None,
        }),
        changed: Condvar::new(),
    });
    let control = TestTranslationControl {
        shared: Arc::clone(&shared),
    };
    let (module, outcomes) = TranslationModule::start(
        selection.target,
        Arc::new(TestTranslationAdapter { shared }),
        PolicyDependencies::real(),
    )?;
    Ok((
        BoundTranslationModule {
            selection,
            credential_id,
            credential_storage,
            credential_display_suffix,
            credential_revision,
            module,
            outcomes,
        },
        control,
    ))
}

struct TestTranslationAdapter {
    shared: Arc<TestTranslationAdapterShared>,
}

struct TestTranslationAdapterShared {
    state: Mutex<TestTranslationAdapterState>,
    changed: Condvar,
}

struct TestTranslationAdapterState {
    results: VecDeque<TestTranslationResult>,
    calls: usize,
    blocked_completion: Option<AdapterCompletion>,
}

impl CompletedTextAdapter for TestTranslationAdapter {
    fn begin(
        &self,
        _request: CompletedTextRequest,
        _control: AttemptControl,
        completion: AdapterCompletion,
    ) -> Result<Box<dyn ActiveTranslationCall>, AdapterFailure> {
        let result = {
            let mut state = self
                .shared
                .state
                .lock()
                .map_err(|_| test_adapter_failure(TranslationFailureClass::Unknown))?;
            state.calls = state.calls.saturating_add(1);
            match state
                .results
                .pop_front()
                .unwrap_or(TestTranslationResult::Failed(
                    TranslationFailureClass::Unknown,
                )) {
                TestTranslationResult::Blocked => {
                    state.blocked_completion = Some(completion);
                    drop(state);
                    self.shared.changed.notify_all();
                    return Ok(Box::new(TestTranslationActiveCall {
                        shared: Arc::clone(&self.shared),
                    }));
                }
                result => result,
            }
        };
        match result {
            TestTranslationResult::Completed(text) => completion.finish(Ok(text)),
            TestTranslationResult::Failed(class) => {
                completion.finish(Err(test_adapter_failure(class)));
            }
            TestTranslationResult::Blocked => unreachable!("blocked result returned above"),
        }
        self.shared.changed.notify_all();
        Ok(Box::new(TestTranslationActiveCall {
            shared: Arc::clone(&self.shared),
        }))
    }
}

struct TestTranslationActiveCall {
    shared: Arc<TestTranslationAdapterShared>,
}

impl ActiveTranslationCall for TestTranslationActiveCall {
    fn cancel(&mut self) -> CancellationStatus {
        if let Ok(mut state) = self.shared.state.lock() {
            state.blocked_completion.take();
            CancellationStatus::Confirmed
        } else {
            CancellationStatus::Unconfirmed
        }
    }
}

fn test_adapter_failure(class: TranslationFailureClass) -> AdapterFailure {
    AdapterFailure {
        class,
        retryable: false,
        retry_after: None,
        request_outcome_ambiguous: false,
    }
}
