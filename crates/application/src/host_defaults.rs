use crate::proposals::ProposalAudit;
use gpui_agent_ports::{Clock,
                       DiagnosticSink};
use std::sync::Arc;

pub(crate) struct DefaultSystemClock;

impl Clock for DefaultSystemClock {
    fn now_unix_secs(&self) -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() }

    fn now_unix_nanos(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock must follow the Unix epoch")
            .as_nanos() as u64
    }
}

pub(crate) struct DefaultStderrDiagnostics;

impl DiagnosticSink for DefaultStderrDiagnostics {
    fn warn(&self, message: &str) {
        eprintln!("{message}");
    }
}

pub(crate) struct DefaultStderrAudit;

impl ProposalAudit for DefaultStderrAudit {
    fn record(&self, event: &serde_json::Value) {
        eprintln!("{event}");
    }
}

pub(crate) fn default_clock() -> Arc<dyn Clock> { Arc::new(DefaultSystemClock) }

pub(crate) fn default_diagnostics() -> Arc<dyn DiagnosticSink> { Arc::new(DefaultStderrDiagnostics) }

pub(crate) fn default_audit() -> Arc<dyn ProposalAudit> { Arc::new(DefaultStderrAudit) }
