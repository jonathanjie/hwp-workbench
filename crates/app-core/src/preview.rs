//! Disposable preview work uses a lane independent of file operations.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Clone, Debug)]
pub struct PreviewToken {
    pub cancel: Arc<AtomicBool>,
}

#[derive(Default, Debug)]
pub struct LatestPreview {
    current: Mutex<Option<PreviewToken>>,
}

impl LatestPreview {
    pub fn start(&self) -> PreviewToken {
        let token = PreviewToken {
            cancel: Arc::new(AtomicBool::new(false)),
        };
        if let Ok(mut current) = self.current.lock() {
            if let Some(previous) = current.replace(token.clone()) {
                previous.cancel.store(true, Ordering::SeqCst);
            }
        }
        token
    }

    pub fn cancel(&self) {
        if let Some(token) = self
            .current
            .lock()
            .ok()
            .and_then(|mut current| current.take())
        {
            token.cancel.store(true, Ordering::SeqCst);
        }
    }

    pub fn is_current(&self, token: &PreviewToken) -> bool {
        !token.cancel.load(Ordering::SeqCst)
            && self.current.lock().ok().is_some_and(|current| {
                current
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(&current.cancel, &token.cancel))
            })
    }

    pub fn finish(&self, token: &PreviewToken) {
        if let Ok(mut current) = self.current.lock() {
            if current
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(&current.cancel, &token.cancel))
            {
                *current = None;
            }
        }
    }
}
