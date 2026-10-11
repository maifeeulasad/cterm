//! A uniform handle over a live execution, whoever owns the PTY.
//!
//! The daemon's registry keeps one of these per live execution so the gRPC
//! layer can subscribe, send input, and interrupt without caring whether the
//! execution is a local PTY run the daemon started ([`cterm_shim::RunControl`])
//! or a remote one owned by a shim over IPC (`RemoteControl`).

use async_trait::async_trait;
use cterm_core::{ExecutionEvent, RecordStore, Result};
use cterm_shim::RunControl;
use tokio::sync::broadcast;

/// Control and observation of one live execution.
#[async_trait]
pub trait ExecutionHandle: Send + Sync {
    /// Subscribes to the execution's live events.
    fn subscribe(&self) -> broadcast::Receiver<ExecutionEvent>;
    /// Writes input to the execution.
    async fn send_input(&self, data: Vec<u8>) -> Result<()>;
    /// Interrupts the execution.
    async fn interrupt(&self) -> Result<()>;
}

#[async_trait]
impl<S: RecordStore + Send + Sync + 'static> ExecutionHandle for RunControl<S> {
    fn subscribe(&self) -> broadcast::Receiver<ExecutionEvent> {
        RunControl::subscribe(self)
    }
    async fn send_input(&self, data: Vec<u8>) -> Result<()> {
        RunControl::send_input(self, data).await
    }
    async fn interrupt(&self) -> Result<()> {
        RunControl::interrupt(self).await
    }
}
