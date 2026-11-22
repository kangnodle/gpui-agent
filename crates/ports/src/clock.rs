pub trait Clock: Send + Sync {
    fn now_unix_secs(&self) -> u64;
    fn now_unix_nanos(&self) -> u64;
}
