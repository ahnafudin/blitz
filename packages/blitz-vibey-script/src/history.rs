//! Native observations of history-dependent operations during a host sample hook.

/// The first observable operation in the hook turn, including its promise jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleEffect {
    Random,
    Timer,
    AnimationFrame,
    DomToggle,
}

#[derive(Default)]
pub(crate) enum HookObservation {
    #[default]
    Inactive,
    Observing(Option<SampleEffect>),
}

impl HookObservation {
    pub(crate) fn begin(&mut self) {
        if matches!(self, Self::Inactive) {
            *self = Self::Observing(None);
        }
    }

    pub(crate) fn record(&mut self, effect: SampleEffect) {
        if let Self::Observing(first) = self {
            first.get_or_insert(effect);
        }
    }

    pub(crate) fn finish(&mut self) -> Option<SampleEffect> {
        match std::mem::take(self) {
            Self::Observing(first) => first,
            Self::Inactive => None,
        }
    }
}
