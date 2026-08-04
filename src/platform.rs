/// Return the current user's home directory.
///
/// Checks `HOME` first (Unix), then `USERPROFILE` (Windows fallback).
pub fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}
