//! Prevent concurrent operations from changing the same CS2 and OBS session.

use serde::Serialize;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OperationOwner {
    pub id: String,
    pub operation: String,
    pub started_at: String,
}

#[derive(Debug, Serialize)]
pub struct RuntimeSessionState {
    pub busy: bool,
    pub owner: Option<OperationOwner>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeSessionBusy {
    code: &'static str,
    message: &'static str,
    owner: OperationOwner,
}

#[derive(Debug, Default)]
pub struct RuntimeSession {
    owner: Mutex<Option<OperationOwner>>,
}

impl RuntimeSession {
    pub fn try_begin(&self, next: OperationOwner) -> Result<RuntimeLease<'_>, RuntimeSessionBusy> {
        let mut state = self.owner.lock().expect("runtime session state");
        if let Some(owner) = state.as_ref() {
            return Err(RuntimeSessionBusy {
                code: "RUNTIME_SESSION_BUSY",
                message: "Another CS2/OBS operation is already running.",
                owner: owner.clone(),
            });
        }
        *state = Some(next);
        Ok(RuntimeLease { session: self })
    }

    pub fn state(&self) -> RuntimeSessionState {
        let owner = self.owner.lock().expect("runtime session state").clone();
        RuntimeSessionState {
            busy: owner.is_some(),
            owner,
        }
    }
}

#[derive(Debug)]
#[must_use = "the runtime session is released when this lease is dropped"]
pub struct RuntimeLease<'a> {
    session: &'a RuntimeSession,
}

impl Drop for RuntimeLease<'_> {
    fn drop(&mut self) {
        *self.session.owner.lock().expect("runtime session state") = None;
    }
}
