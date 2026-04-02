use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SboxError {
    #[error("config file not found: {}", .0.display())]
    ConfigNotFound(PathBuf),

    #[error("failed to read config {}: {source}", path.display())]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to parse config {}: {source}", path.display())]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("invalid config:\n{message}")]
    ConfigValidation { message: String },

    #[error("unknown profile `{name}`")]
    UnknownProfile { name: String },

    #[error("unknown preset `{name}`")]
    UnknownPreset { name: String },

    #[error("no shell could be resolved")]
    NoShellResolved,

    #[error("refusing unsafe sandbox execution for `{command}`: {reason}")]
    UnsafeExecutionPolicy { command: String, reason: String },

    #[error("configured audit hook `{hook}` failed before `{command}` with exit status `{status}`")]
    AuditHookFailed {
        hook: String,
        command: String,
        status: u8,
    },

    #[error("image signature verification is unavailable for `{image}`: {reason}")]
    SignatureVerificationUnavailable { image: String, reason: String },

    #[error(
        "image signature verification failed for `{image}` using policy {}: {reason}",
        policy.display()
    )]
    SignatureVerificationFailed {
        image: String,
        policy: PathBuf,
        reason: String,
    },

    #[error("refusing to overwrite existing config {}", path.display())]
    InitConfigExists { path: PathBuf },

    #[error("failed to write generated config {}: {source}", path.display())]
    InitWrite {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "failed to resolve a profile for command `{command}`; define a `default` profile or pass `--profile`"
    )]
    ProfileResolutionFailed { command: String },

    #[error("failed to start command `{program}`: {source}")]
    CommandSpawn {
        program: String,
        #[source]
        source: io::Error,
    },

    #[error("backend `{backend}` is unavailable: {source}")]
    BackendUnavailable {
        backend: String,
        #[source]
        source: io::Error,
    },

    #[error("backend `{backend}` command failed: `{command}` exited with status `{status}`")]
    BackendCommandFailed {
        backend: String,
        command: String,
        status: i32,
    },

    #[error(
        "sandbox execution is not implemented yet for profile `{profile}` with backend `{backend}`"
    )]
    SandboxExecutionNotImplemented { profile: String, backend: String },

    #[error("sandbox image source `{image_source}` is not implemented for backend `{backend}`")]
    UnsupportedImageSource {
        backend: String,
        image_source: String,
    },

    #[error("unsupported mount type `{mount_type}`")]
    UnsupportedMountType { mount_type: String },

    #[error("host path for {kind} `{name}` does not exist or is not accessible: {}", path.display())]
    HostPathNotFound {
        kind: &'static str,
        name: String,
        path: PathBuf,
    },

    #[error(
        "secret `{name}` uses unsupported source `{secret_source}`; only host path sources are implemented"
    )]
    UnsupportedSecretSource { name: String, secret_source: String },

    #[error("reusable sandbox sessions are not implemented yet for profile `{profile}`")]
    ReusableSandboxSessionsNotImplemented { profile: String },

    #[error("failed to determine current directory: {source}")]
    CurrentDirectory {
        #[source]
        source: io::Error,
    },

    #[error("failed to initialize logging: {source}")]
    LoggingInit {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}
