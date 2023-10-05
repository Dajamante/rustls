//! no-std time provider

use alloc::sync::Arc;
use core::fmt::Debug;

use pki_types::UnixTime;

/// An object that provides the current time
#[derive(Clone, Debug)]
pub struct TimeProvider {
    inner: Arc<dyn GetCurrentTime>,
}

impl TimeProvider {
    /// Creates a new time provider
    pub fn new(time_getter: impl GetCurrentTime + 'static) -> Self {
        Self {
            inner: Arc::new(time_getter),
        }
    }

    /// A time provider that always fails
    pub fn none() -> Self {
        Self::new(NoTimeProvider)
    }

    pub(crate) fn get_current_time(&self) -> Option<UnixTime> {
        self.inner.get_current_time()
    }
}

/// Get current time
pub trait GetCurrentTime: Debug + Send + Sync {
    /// Returns the current time
    ///
    /// Or `None` if unable to retrieve it
    fn get_current_time(&self) -> Option<UnixTime>;
}

#[derive(Debug)]
struct NoTimeProvider;

impl GetCurrentTime for NoTimeProvider {
    fn get_current_time(&self) -> Option<UnixTime> {
        None
    }
}
