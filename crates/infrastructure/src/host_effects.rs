use gpui_agent_application::ports::{Clock,
                                    DiagnosticSink};

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_secs(&self) -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() }

    fn now_unix_nanos(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock must follow the Unix epoch")
            .as_nanos() as u64
    }
}

pub struct StderrDiagnostics;

impl DiagnosticSink for StderrDiagnostics {
    fn warn(&self, message: &str) {
        eprintln!("{message}");
    }
}
