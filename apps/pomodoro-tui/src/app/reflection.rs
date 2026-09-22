use pomodoro_core::{DomainError, DomainState, ProgressState, ReflectionSummary};

#[derive(Default)]
pub(super) struct HistoryReflection {
    cached: Option<CachedSummary>,
    #[cfg(test)]
    pub(super) rebuilds: usize,
}

struct CachedSummary {
    lengths: (usize, usize),
    summary: Result<ReflectionSummary, DomainError>,
}

impl HistoryReflection {
    pub(super) fn get(&mut self, domain: &DomainState) -> Result<ReflectionSummary, DomainError> {
        let history = domain.history();
        let lengths = (history.sessions.len(), history.events.len());
        // Within one App, controller transitions only append history; a failed
        // save exposes the previous durable prefix. A new load creates a new App.
        // Snapshot-only ticks and save rollback therefore need no history scan.
        if self
            .cached
            .as_ref()
            .is_none_or(|cached| cached.lengths != lengths)
        {
            self.cached = Some(CachedSummary {
                lengths,
                summary: history.reflection(),
            });
            #[cfg(test)]
            {
                self.rebuilds += 1;
            }
        }
        let summary = self
            .cached
            .as_ref()
            .expect("cache was populated")
            .summary
            .clone()?;
        match &domain.snapshot().state {
            ProgressState::Active { session, .. } => summary.including_session(session),
            _ => Ok(summary),
        }
    }
}
