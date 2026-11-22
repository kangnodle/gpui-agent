pub trait DiagnosticSink: Send + Sync {
    fn warn(&self, message: &str);
}
