//! HTTP resource semantics and concrete acquisition/inspection adapters.
//!
//! Inspection uses HEAD only. A received final status is an observation rather than an acquisition
//! failure. Evidence records the requested URL, the post-redirect final URL, method, admission wait,
//! retry status, and planned delay. `declared_content_length` is response metadata: it is not body
//! bytes observed by Pulith, artifact identity, validator continuity, provenance, or trust evidence.
//! Inspection never falls back to GET, copies a response body, stages a file, or publishes a target.

#![allow(
    clippy::result_large_err,
    reason = "network errors intentionally preserve complete retry and resume evidence"
)]

use std::fmt;
use std::io;
#[cfg(feature = "http-ureq")]
use std::io::Write;
#[cfg(feature = "http-ureq")]
use std::io::{Read, Seek};
use std::num::NonZeroU32;
#[cfg(feature = "http-ureq")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(feature = "http-ureq")]
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[cfg(feature = "http-ureq")]
use governor::clock::Clock;

#[derive(Clone, Debug, Eq, PartialEq)]
/// Failure to admit an absolute HTTP or HTTPS URL.
pub enum RemoteUrlError {
    /// The invalid outcome.
    Invalid {
        /// Original input that could not be parsed as a URL.
        input: String,
    },
    /// The unsupported scheme outcome.
    UnsupportedScheme {
        /// Parsed scheme rejected because it is not HTTP or HTTPS.
        scheme: String,
    },
}

impl fmt::Display for RemoteUrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { input } => write!(f, "invalid remote URL: {input}"),
            Self::UnsupportedScheme { scheme } => {
                write!(f, "unsupported remote URL scheme: {scheme}")
            }
        }
    }
}

impl std::error::Error for RemoteUrlError {}

#[derive(Debug)]
/// Failure to acquire remote material, retaining available attempt and resume evidence.
pub enum DownloadError {
    /// The remote url outcome.
    RemoteUrl(RemoteUrlError),
    /// The http status outcome.
    HttpStatus {
        /// The url value.
        url: url::Url,
        /// The status value.
        status: u16,
        /// The retryable value.
        retryable: bool,
        /// The attempts value.
        attempts: Vec<AttemptEvidence>,
        /// The resume value.
        resume: Option<ResumeEvidence>,
    },
    /// The transport outcome.
    Transport {
        /// The url value.
        url: url::Url,
        /// The phase value.
        phase: TransportPhase,
        /// The message value.
        message: String,
        /// The attempts value.
        attempts: Vec<AttemptEvidence>,
        /// The resume value.
        resume: Option<ResumeEvidence>,
    },
    /// The protocol outcome.
    Protocol {
        /// The url value.
        url: url::Url,
        /// The kind value.
        kind: ProtocolError,
        /// The attempts value.
        attempts: Vec<AttemptEvidence>,
        /// The resume value.
        resume: Option<ResumeEvidence>,
    },
    /// The limit exceeded outcome.
    LimitExceeded {
        /// The url value.
        url: url::Url,
        /// The max value.
        max: u64,
        /// The actual value.
        actual: u64,
        /// The attempts value.
        attempts: Vec<AttemptEvidence>,
        /// The resume value.
        resume: Option<ResumeEvidence>,
    },

    /// The local outcome.
    Local {
        /// The url value.
        url: Option<url::Url>,
        /// The action value.
        action: &'static str,
        /// The path value.
        path: PathBuf,
        /// The source value.
        source: io::Error,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// HTTP transport phase in which an adapter error occurred.
pub enum TransportPhase {
    /// The send request outcome.
    SendRequest,
    /// The read body outcome.
    ReadBody,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Violation of the HTTP range, length, or validator contract.
pub enum ProtocolError {
    /// The unexpected partial response outcome.
    UnexpectedPartialResponse,
    /// The resume validator mismatch outcome.
    ResumeValidatorMismatch,
    /// The invalid content range outcome.
    InvalidContentRange {
        /// The expected start value.
        expected_start: u64,
        /// The header value.
        header: Option<String>,
    },
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RemoteUrl(error) => write!(f, "net acquire source error: {error}"),
            Self::HttpStatus { url, status, .. } => {
                write!(f, "net acquire HTTP status {status}: {url}")
            }
            Self::Transport {
                url,
                phase,
                message,
                ..
            } => {
                write!(
                    f,
                    "net acquire transport error during {phase:?} for {url}: {message}"
                )
            }
            Self::Protocol { url, kind, .. } => {
                write!(f, "net acquire protocol error for {url}: {kind:?}")
            }
            Self::LimitExceeded {
                url, max, actual, ..
            } => {
                write!(
                    f,
                    "net acquire byte limit exceeded for {url}: {actual} > {max}"
                )
            }

            Self::Local {
                action,
                path,
                source,
                ..
            } => {
                write!(
                    f,
                    "net acquire failed to {action} {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for DownloadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RemoteUrl(error) => Some(error),
            Self::Local { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl DownloadError {
    #[cfg(feature = "http-ureq")]
    fn local(
        url: Option<&RemoteUrl>,
        action: &'static str,
        path: impl AsRef<Path>,
        source: io::Error,
    ) -> Self {
        Self::Local {
            url: url.map(|url| url.as_url().clone()),
            action,
            path: path.as_ref().to_path_buf(),
            source,
        }
    }

    #[cfg(feature = "http-ureq")]
    fn transport(
        url: &RemoteUrl,
        phase: TransportPhase,
        message: impl Into<String>,
        attempts: Vec<AttemptEvidence>,
        resume: Option<ResumeEvidence>,
    ) -> Self {
        Self::Transport {
            url: url.as_url().clone(),
            phase,
            message: message.into(),
            attempts,
            resume,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Admitted absolute HTTP or HTTPS URL.
pub struct RemoteUrl(url::Url);

impl RemoteUrl {
    /// Parses and admits an absolute HTTP or HTTPS URL.
    pub fn parse(input: &str) -> Result<Self, RemoteUrlError> {
        let url = url::Url::parse(input).map_err(|_| RemoteUrlError::Invalid {
            input: input.to_string(),
        })?;
        match url.scheme() {
            "http" | "https" => Ok(Self(url)),
            scheme => Err(RemoteUrlError::UnsupportedScheme {
                scheme: scheme.to_string(),
            }),
        }
    }

    /// Returns the admitted URL as a string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    #[cfg(feature = "http-ureq")]
    fn local_error(&self, action: &'static str, path: &Path, source: io::Error) -> DownloadError {
        DownloadError::local(Some(self), action, path, source)
    }

    /// Borrows the parsed URL value.
    pub fn as_url(&self) -> &url::Url {
        &self.0
    }

    /// Consumes this wrapper and returns the parsed URL value.
    pub fn into_url(self) -> url::Url {
        self.0
    }
}

impl std::str::FromStr for RemoteUrl {
    type Err = RemoteUrlError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        RemoteUrl::parse(input)
    }
}

impl TryFrom<String> for RemoteUrl {
    type Error = RemoteUrlError;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        input.parse()
    }
}

/// Optional serde deserialization (feature `serde`): the http(s) scheme gate stays here.
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for RemoteUrl {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let input = String::deserialize(deserializer)?;
        RemoteUrl::parse(&input).map_err(serde::de::Error::custom)
    }
}

impl From<RemoteUrlError> for DownloadError {
    fn from(error: RemoteUrlError) -> Self {
        Self::RemoteUrl(error)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
/// Caller-selected timeout, size, header, retry, and resume policy for acquisition.
pub struct DownloadPolicy {
    /// The timeout value.
    pub timeout: Option<Duration>,
    /// The max bytes value.
    pub max_bytes: Option<u64>,
    /// The headers value.
    pub headers: Vec<(String, String)>,
    /// The retry value.
    pub retry: RetryPolicy,
    /// The resume value.
    pub resume: ResumePolicy,
}

impl DownloadPolicy {
    /// Sets the timeout applied to each acquisition attempt.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the maximum accepted artifact size.
    pub fn max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = Some(max_bytes);
        self
    }

    /// Adds one caller-selected request header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Replaces the acquisition retry policy.
    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Replaces the acquisition resume policy.
    pub fn resume(mut self, resume: ResumePolicy) -> Self {
        self.resume = resume;
        self
    }
}

/// A sustained outbound-attempt rate and its burst capacity.
///
/// One admitted cell means permission to enter one outbound request attempt.
/// Retries consume another cell. This rate does not measure response bytes or
/// maximum in-flight concurrency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptRate {
    attempts_per_second: NonZeroU32,
    burst_attempts: NonZeroU32,
}

impl AttemptRate {
    /// The pub outcome.
    pub const fn new(attempts_per_second: NonZeroU32, burst_attempts: NonZeroU32) -> Self {
        Self {
            attempts_per_second,
            burst_attempts,
        }
    }

    /// The pub outcome.
    pub const fn attempts_per_second(self) -> NonZeroU32 {
        self.attempts_per_second
    }

    /// The pub outcome.
    pub const fn burst_attempts(self) -> NonZeroU32 {
        self.burst_attempts
    }
}

/// A concrete shared GCRA gate for outbound attempt entry.
///
/// Cloning an `Arc<RateAdmission>` into multiple resources coordinates them
/// through one atomic limiter state. Each call makes one single-cell GCRA
/// decision; the returned permit reports accumulated governor-requested wait,
/// not method wall time.
///
/// This is an attempt-rate gate, not a semaphore or body-copy byte pacer.
pub struct RateAdmission {
    rate: AttemptRate,
    #[cfg(feature = "http-ureq")]
    limiter: governor::DefaultDirectRateLimiter,
}

impl RateAdmission {
    /// Constructs a shared attempt admission gate for `rate`.
    pub fn new(rate: AttemptRate) -> Self {
        #[cfg(feature = "http-ureq")]
        let quota = governor::Quota::per_second(rate.attempts_per_second())
            .allow_burst(rate.burst_attempts());
        Self {
            rate,
            #[cfg(feature = "http-ureq")]
            limiter: governor::RateLimiter::direct(quota),
        }
    }

    /// The pub outcome.
    pub const fn rate(&self) -> AttemptRate {
        self.rate
    }

    #[cfg(feature = "http-ureq")]
    fn check(&self) -> Option<Duration> {
        match self.limiter.check() {
            Ok(_) => None,
            Err(not_until) => Some(not_until.wait_time_from(self.limiter.clock().now())),
        }
    }

    #[cfg(feature = "http-ureq")]
    fn enter_sync(&self) -> Duration {
        let mut waited = Duration::ZERO;
        while let Some(wait) = self.check() {
            std::thread::sleep(wait);
            waited = waited.saturating_add(wait);
        }
        waited
    }
}

impl fmt::Debug for RateAdmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateAdmission")
            .field("rate", &self.rate)
            .finish_non_exhaustive()
    }
}

/// Body-copy byte pacing policy.
///
/// Pacing applies after Pulith observes and accepts a response body chunk but
/// before that chunk enters the staging artifact. It is not raw socket
/// bandwidth control: the HTTP client, TLS stack, or kernel may already have
/// buffered bytes before Pulith observes the chunk.
/// A body-copy byte rate and its maximum burst capacity.
///
/// `bytes_per_second` is the sustained GCRA rate. `burst_bytes` is the largest
/// accounting batch admitted at once; observed body chunks larger than the
/// burst are split into bounded batches before staged writing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteRate {
    bytes_per_second: NonZeroU32,
    burst_bytes: NonZeroU32,
}

impl ByteRate {
    /// The pub outcome.
    pub const fn new(bytes_per_second: NonZeroU32, burst_bytes: NonZeroU32) -> Self {
        Self {
            bytes_per_second,
            burst_bytes,
        }
    }

    /// The pub outcome.
    pub const fn bytes_per_second(self) -> NonZeroU32 {
        self.bytes_per_second
    }

    /// The pub outcome.
    pub const fn burst_bytes(self) -> NonZeroU32 {
        self.burst_bytes
    }
}

/// A concrete shared GCRA body-copy byte pacer.
///
/// Cloning an `Arc<ByteRatePacer>` into multiple resources coordinates them
/// through one atomic limiter state. Each call makes one bounded `check_n`
/// decision. Zero-byte calls are immediate.
///
/// This remains decoded body-copy pacing, not raw socket bandwidth control.
/// Cancellation after accepted sub-batches may conservatively consume budget
/// without staging the chunk; it cannot permit an overrun.
pub struct ByteRatePacer {
    rate: ByteRate,
    #[cfg(feature = "http-ureq")]
    limiter: governor::DefaultDirectRateLimiter,
}

impl ByteRatePacer {
    /// Constructs a shared decoded-byte pacer for `rate`.
    pub fn new(rate: ByteRate) -> Self {
        #[cfg(feature = "http-ureq")]
        let quota =
            governor::Quota::per_second(rate.bytes_per_second()).allow_burst(rate.burst_bytes());
        Self {
            rate,
            #[cfg(feature = "http-ureq")]
            limiter: governor::RateLimiter::direct(quota),
        }
    }

    /// The pub outcome.
    pub const fn rate(&self) -> ByteRate {
        self.rate
    }

    #[cfg(feature = "http-ureq")]
    fn next_batch(&self, remaining: u64) -> Option<NonZeroU32> {
        NonZeroU32::new(remaining.min(u64::from(self.rate.burst_bytes().get())) as u32)
    }

    #[cfg(feature = "http-ureq")]
    fn check_batch(&self, batch: NonZeroU32) -> Option<Duration> {
        match self.limiter.check_n(batch) {
            Ok(Ok(_)) => None,
            Ok(Err(not_until)) => Some(not_until.wait_time_from(self.limiter.clock().now())),
            Err(_) => unreachable!("pacing batches never exceed configured burst"),
        }
    }

    #[cfg(feature = "http-ureq")]
    fn before_chunk_sync(&self, bytes: u64) -> Duration {
        let mut remaining = bytes;
        let mut waited = Duration::ZERO;
        while let Some(batch) = self.next_batch(remaining) {
            while let Some(wait) = self.check_batch(batch) {
                std::thread::sleep(wait);
                waited = waited.saturating_add(wait);
            }
            remaining -= u64::from(batch.get());
        }
        waited
    }
}

impl fmt::Debug for ByteRatePacer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ByteRatePacer")
            .field("rate", &self.rate)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Bounded exponential retry policy with optional delay cap and `Retry-After` support.
pub struct RetryPolicy {
    /// The max retries value.
    pub max_retries: u32,
    /// The base delay value.
    pub base_delay: Duration,
    /// The max delay value.
    pub max_delay: Option<Duration>,
    /// The respect retry after value.
    pub respect_retry_after: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::disabled()
    }
}

impl RetryPolicy {
    /// Constructs a policy that performs no retries.
    pub fn disabled() -> Self {
        Self {
            max_retries: 0,
            base_delay: Duration::ZERO,
            max_delay: None,
            respect_retry_after: true,
        }
    }

    /// Constructs bounded exponential retry delays.
    pub fn exponential(max_retries: u32, base_delay: Duration) -> Self {
        Self {
            max_retries,
            base_delay,
            max_delay: None,
            respect_retry_after: true,
        }
    }

    /// Caps the computed retry delay.
    pub fn max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = Some(max_delay);
        self
    }

    /// Selects whether valid `Retry-After` response values influence retry delay.
    pub fn respect_retry_after(mut self, respect: bool) -> Self {
        self.respect_retry_after = respect;
        self
    }
}

/// Caller-selected execution policy for HTTP HEAD inspection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HttpInspectPolicy {
    /// The timeout value.
    pub timeout: Option<Duration>,
    /// The retry value.
    pub retry: RetryPolicy,
}

impl HttpInspectPolicy {
    /// Sets the timeout applied to each inspection attempt.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Replaces the inspection retry policy.
    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }
}

/// HTTP-specific facts reported by a HEAD response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteObservation {
    /// The status value.
    pub status: u16,
    /// The declared content length value.
    pub declared_content_length: Option<u64>,
}

/// Evidence for one HTTP inspection or admission attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpInspectAttemptEvidence {
    /// The attempt value.
    pub attempt: u32,
    /// The status value.
    pub status: Option<u16>,
    /// The admission wait value.
    pub admission_wait: Option<Duration>,
    /// The planned delay value.
    pub planned_delay: Option<Duration>,
}

#[cfg(feature = "http-ureq")]
impl HttpInspectAttemptEvidence {
    fn new(attempt: u32, status: Option<u16>, admission_wait: Option<Duration>) -> Self {
        Self {
            attempt,
            status,
            admission_wait,
            planned_delay: None,
        }
    }

    fn with_planned_delay(mut self, planned_delay: Option<Duration>) -> Self {
        self.planned_delay = planned_delay;
        self
    }
}

/// Evidence for a completed HTTP inspection, including redirect authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteInspectEvidence {
    /// The requested url value.
    pub requested_url: url::Url,
    /// The final url value.
    pub final_url: url::Url,
    /// The attempts value.
    pub attempts: Vec<HttpInspectAttemptEvidence>,
}

/// Failures that prevented HTTP inspection from receiving a final response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HttpInspectError {
    /// The transport outcome.
    Transport {
        /// The url value.
        url: url::Url,
        /// The message value.
        message: String,
        /// The attempts value.
        attempts: Vec<HttpInspectAttemptEvidence>,
    },

    /// The protocol outcome.
    Protocol {
        /// The url value.
        url: url::Url,
        /// The message value.
        message: String,
        /// The attempts value.
        attempts: Vec<HttpInspectAttemptEvidence>,
    },
}

impl fmt::Display for HttpInspectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport { url, message, .. } => {
                write!(f, "HTTP inspection transport error for {url}: {message}")
            }

            Self::Protocol { url, message, .. } => {
                write!(f, "HTTP inspection protocol error for {url}: {message}")
            }
        }
    }
}

impl std::error::Error for HttpInspectError {}

/// Controls whether acquisition restarts or resumes from validator-bound bytes.
///
/// Resume without a strong validator is intentionally unavailable:
///
/// ```compile_fail
/// use pulith::net::ResumePolicy;
/// let _ = ResumePolicy::unvalidated("artifact.part");
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResumePolicy {
    /// The restart only outcome.
    RestartOnly,
    /// The if range outcome.
    IfRange {
        /// The partial path value.
        partial_path: PathBuf,
        /// The validator value.
        validator: Validator,
    },
}

impl Default for ResumePolicy {
    fn default() -> Self {
        Self::restart_only()
    }
}

impl ResumePolicy {
    /// Selects acquisition that always starts from an empty stage.
    pub fn restart_only() -> Self {
        Self::RestartOnly
    }

    /// Selects validated range continuation from a caller-owned partial file.
    pub fn if_range(partial_path: impl Into<PathBuf>, validator: Validator) -> Self {
        Self::IfRange {
            partial_path: partial_path.into(),
            validator,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Strong HTTP validator admitted for safe range continuation.
pub struct Validator {
    kind: ValidatorKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ValidatorKind {
    StrongEtag(String),
    StrongLastModified(SystemTime),
}

impl Validator {
    /// Constructs a validator only from an RFC entity-tag that is syntactically valid and strong.
    pub fn strong_etag(value: impl Into<String>) -> Option<Self> {
        parse_strong_etag(&value.into()).map(|value| Self {
            kind: ValidatorKind::StrongEtag(value),
        })
    }

    /// Constructs an RFC strong-date validator and normalizes both dates to HTTP-date seconds.
    pub fn strong_last_modified(last_modified: SystemTime, date: SystemTime) -> Option<Self> {
        let last_modified = normalize_http_date(last_modified)?;
        let date = normalize_http_date(date)?;
        date.duration_since(last_modified)
            .ok()
            .filter(|age| *age >= Duration::from_secs(60))
            .map(|_| Self {
                kind: ValidatorKind::StrongLastModified(last_modified),
            })
    }

    #[cfg(any(test, feature = "http-ureq"))]
    fn if_range_value(&self) -> String {
        match &self.kind {
            ValidatorKind::StrongEtag(value) => value.clone(),
            ValidatorKind::StrongLastModified(time) => httpdate::fmt_http_date(*time),
        }
    }

    #[cfg(any(test, feature = "http-ureq"))]
    fn permits_response(&self, etag: Option<&str>, last_modified: Option<&str>) -> bool {
        match &self.kind {
            ValidatorKind::StrongEtag(expected) => etag
                .map(|value| parse_strong_etag(value).as_ref() == Some(expected))
                .unwrap_or(true),
            ValidatorKind::StrongLastModified(expected) => last_modified
                .map(|value| httpdate::parse_http_date(value).ok().as_ref() == Some(expected))
                .unwrap_or(true),
        }
    }
}

/// An HTTP address and its download policy.
#[derive(Clone)]
pub struct RemoteSource {
    pub(crate) url: RemoteUrl,
    pub(crate) policy: DownloadPolicy,
}

impl PartialEq for RemoteSource {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url && self.policy == other.policy
    }
}

impl Eq for RemoteSource {}

impl RemoteSource {
    /// Constructs a remote source with the default download policy.
    pub fn new(url: RemoteUrl) -> Self {
        Self {
            url,
            policy: DownloadPolicy::default(),
        }
    }

    /// Parse a URL string into a remote source in one step.
    pub fn from_url_str(url: &str) -> Result<Self, RemoteUrlError> {
        Ok(Self::new(RemoteUrl::parse(url)?))
    }

    /// Replaces the acquisition policy.
    pub fn policy(mut self, policy: DownloadPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Borrows the admitted remote URL.
    pub fn url(&self) -> &RemoteUrl {
        &self.url
    }

    #[cfg(feature = "http-ureq")]
    fn prepare_resume(
        &mut self,
        stage: &mut std::fs::File,
    ) -> Result<Option<PlannedResume>, DownloadError> {
        let resume = match std::mem::take(&mut self.policy.resume) {
            ResumePolicy::RestartOnly => None,
            ResumePolicy::IfRange {
                partial_path,
                validator,
            } => {
                use std::io::Read;
                let source =
                    crate::local::LocalPath::new(&partial_path).and_then(|path| path.open_file());
                let partial = match source {
                    Ok(Some(partial)) => partial,
                    Ok(None) => return Ok(None),
                    Err(error) => {
                        return Err(self.url.local_error(
                            "open partial download",
                            &partial_path,
                            io::Error::other(error),
                        ));
                    }
                };
                let limit = self.policy.max_bytes.unwrap_or(u64::MAX);
                let mut prefix = partial.take(limit.saturating_add(1));
                let partial_bytes = std::io::copy(&mut prefix, stage).map_err(|error| {
                    self.url
                        .local_error("snapshot partial download", &partial_path, error)
                })?;
                if partial_bytes > limit {
                    stage
                        .set_len(0)
                        .and_then(|()| stage.rewind())
                        .map_err(|error| {
                            self.url.local_error(
                                "reset oversized partial download",
                                &partial_path,
                                error,
                            )
                        })?;
                    return Ok(None);
                }
                (partial_bytes > 0).then_some(PlannedResume {
                    partial_path,
                    partial_bytes,
                    validator,
                })
            }
        };
        Ok(resume)
    }
}

/// Observed HTTP acquisition facts, excluding the ephemeral stage owned by the output artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadEvidence {
    /// The url value.
    pub url: url::Url,
    /// The status value.
    pub status: u16,
    /// The bytes value.
    pub bytes: u64,
    /// The content length value.
    pub content_length: Option<u64>,
    /// The attempts value.
    pub attempts: Vec<AttemptEvidence>,
    /// The resume value.
    pub resume: Option<ResumeEvidence>,
    /// The validator value.
    pub validator: Option<Validator>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Partial-file identity and outcome observed during one resumed acquisition.
pub struct ResumeEvidence {
    /// The outcome value.
    pub outcome: ResumeOutcome,
    /// The partial path value.
    pub partial_path: PathBuf,
    /// The partial bytes value.
    pub partial_bytes: u64,
    /// The validator value.
    pub validator: Validator,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// How an HTTP response used or discarded the prepared partial input.
pub enum ResumeOutcome {
    /// The partial appended outcome.
    PartialAppended,
    /// The range ignored restarted outcome.
    RangeIgnoredRestarted,
    /// The range unsatisfiable restarted outcome.
    RangeUnsatisfiableRestarted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Transport, admission, pacing, and byte facts for one acquisition attempt.
pub struct AttemptEvidence {
    /// The attempt value.
    pub attempt: u32,
    /// The status value.
    pub status: Option<u16>,
    /// The bytes value.
    pub bytes: u64,
    /// The content length value.
    pub content_length: Option<u64>,
    /// The retry after value.
    pub retry_after: Option<Duration>,
    /// The planned delay value.
    pub planned_delay: Option<Duration>,
    /// The admission wait value.
    pub admission_wait: Option<Duration>,
    /// The pacing wait value.
    pub pacing_wait: Duration,
    /// The outcome value.
    pub outcome: AttemptOutcome,
}

#[cfg(feature = "http-ureq")]
impl AttemptEvidence {
    fn new(attempt: u32, outcome: AttemptOutcome) -> Self {
        Self {
            attempt,
            status: None,
            bytes: 0,
            content_length: None,
            retry_after: None,
            planned_delay: None,
            admission_wait: None,
            pacing_wait: Duration::ZERO,
            outcome,
        }
    }

    fn response(
        attempt: u32,
        status: u16,
        content_length: Option<u64>,
        admission_wait: Option<Duration>,
        outcome: AttemptOutcome,
    ) -> Self {
        Self::new(attempt, outcome)
            .with_status(status)
            .with_content_length(content_length)
            .with_admission_wait(admission_wait)
    }

    #[cfg(feature = "http-ureq")]
    fn transfer(
        attempt: u32,
        status: u16,
        bytes: u64,
        content_length: Option<u64>,
        admission_wait: Option<Duration>,
        outcome: AttemptOutcome,
    ) -> Self {
        Self::response(attempt, status, content_length, admission_wait, outcome).with_bytes(bytes)
    }

    fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    #[cfg(feature = "http-ureq")]
    fn with_bytes(mut self, bytes: u64) -> Self {
        self.bytes = bytes;
        self
    }

    fn with_content_length(mut self, content_length: Option<u64>) -> Self {
        self.content_length = content_length;
        self
    }

    fn with_retry_after(mut self, retry_after: Option<Duration>) -> Self {
        self.retry_after = retry_after;
        self
    }

    fn with_planned_delay(mut self, planned_delay: Option<Duration>) -> Self {
        self.planned_delay = planned_delay;
        self
    }

    fn with_admission_wait(mut self, admission_wait: Option<Duration>) -> Self {
        self.admission_wait = admission_wait;
        self
    }

    #[cfg(feature = "http-ureq")]
    fn with_pacing_wait(mut self, pacing_wait: Duration) -> Self {
        self.pacing_wait = pacing_wait;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Terminal classification of one acquisition attempt.
pub enum AttemptOutcome {
    /// The success outcome.
    Success,
    /// The retryable status outcome.
    RetryableStatus,
    /// The retryable network error outcome.
    RetryableNetworkError,
    /// The non retryable status outcome.
    NonRetryableStatus,
    /// The non retryable network error outcome.
    NonRetryableNetworkError,
    /// The local failure outcome.
    LocalFailure,
    /// The limit exceeded outcome.
    LimitExceeded,
}

#[cfg(feature = "http-ureq")]
type RetryDelay = Arc<dyn Fn(Duration) + Send + Sync>;

#[cfg(feature = "http-ureq")]
#[derive(Clone)]
/// Shareable synchronous HTTP agent, admission gate, and decoded-byte pacer.
pub struct UreqResources {
    agent: ureq::Agent,
    delay: RetryDelay,
    admission: Option<Arc<RateAdmission>>,
    byte_pacer: Option<Arc<ByteRatePacer>>,
}

#[cfg(feature = "http-ureq")]
impl Default for UreqResources {
    /// Direct transport only. Pulith never reads process proxy variables implicitly; a proxy is
    /// an explicit resource injected through [`UreqResources::from_agent`].
    fn default() -> Self {
        let config = ureq::Agent::config_builder().proxy(None).build();
        Self::from_agent(config.new_agent())
    }
}

#[cfg(feature = "http-ureq")]
impl UreqResources {
    /// Constructs adapter resources around an existing ureq agent.
    pub fn from_agent(agent: ureq::Agent) -> Self {
        Self {
            agent,
            delay: Arc::new(std::thread::sleep),
            admission: None,
            byte_pacer: None,
        }
    }

    #[cfg(test)]
    fn with_delay(mut self, delay: RetryDelay) -> Self {
        self.delay = delay;
        self
    }

    /// Attaches a shared attempt admission gate.
    pub fn with_admission(mut self, admission: Arc<RateAdmission>) -> Self {
        self.admission = Some(admission);
        self
    }

    /// Attaches a shared decoded-byte pacer.
    pub fn with_byte_pacer(mut self, byte_pacer: Arc<ByteRatePacer>) -> Self {
        self.byte_pacer = Some(byte_pacer);
        self
    }
}

#[cfg(feature = "http-ureq")]
impl UreqResources {
    /// Downloads into an unnamed byte file that can be verified and consumed without reopening it.
    pub fn download(
        &self,
        mut source: RemoteSource,
    ) -> Result<(crate::local::FileSnapshot, DownloadEvidence), DownloadError> {
        let path = std::env::temp_dir();
        let mut file = tempfile::tempfile().map_err(|error| {
            source
                .url
                .local_error("create download stage", &path, error)
        })?;
        let resume = source.prepare_resume(&mut file)?;
        let evidence = source.download_into(self, &mut file, &path, resume)?;
        file.rewind().map_err(|source| DownloadError::Local {
            url: Some(evidence.url.clone()),
            action: "rewind download",
            path: path.clone(),
            source,
        })?;
        Ok((crate::local::FileSnapshot { file, source: path }, evidence))
    }

    /// Issues a HEAD request and returns the status, declared length, and attempt history.
    pub fn inspect_headers(
        &self,
        url: &RemoteUrl,
        policy: HttpInspectPolicy,
    ) -> Result<(RemoteObservation, RemoteInspectEvidence), HttpInspectError> {
        use ureq::ResponseExt as _;

        let requested_url = url.as_url().clone();
        let mut attempts = Vec::new();
        for attempt in 0..=policy.retry.max_retries {
            let admission_wait = self
                .admission
                .as_ref()
                .map(|admission| admission.enter_sync());

            let mut request_config = self
                .agent
                .head(url.as_str())
                .config()
                .http_status_as_error(false);
            if let Some(timeout) = policy.timeout {
                request_config = request_config.timeout_global(Some(timeout));
            }
            let response = match request_config.build().call() {
                Ok(response) => response,
                Err(error) => {
                    let will_retry = attempt < policy.retry.max_retries;
                    let planned_delay = will_retry.then(|| retry_delay(policy.retry, attempt));
                    attempts.push(
                        HttpInspectAttemptEvidence::new(attempt, None, admission_wait)
                            .with_planned_delay(planned_delay),
                    );
                    if let Some(delay) = planned_delay {
                        (self.delay)(delay);
                        continue;
                    }
                    return Err(HttpInspectError::Transport {
                        url: requested_url.clone(),
                        message: error.to_string(),
                        attempts,
                    });
                }
            };

            let status = response.status().as_u16();
            let declared_content_length = response
                .headers()
                .get("content-length")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| parse_retry_after(value, SystemTime::now()));
            let final_url = url::Url::parse(&response.get_uri().to_string()).map_err(|error| {
                attempts.push(HttpInspectAttemptEvidence::new(
                    attempt,
                    Some(status),
                    admission_wait,
                ));
                HttpInspectError::Protocol {
                    url: requested_url.clone(),
                    message: error.to_string(),
                    attempts: attempts.clone(),
                }
            })?;
            let will_retry = should_retry_status(status) && attempt < policy.retry.max_retries;
            let planned_delay =
                will_retry.then(|| planned_retry_delay(policy.retry, attempt, retry_after));
            attempts.push(
                HttpInspectAttemptEvidence::new(attempt, Some(status), admission_wait)
                    .with_planned_delay(planned_delay),
            );
            if let Some(delay) = planned_delay {
                (self.delay)(delay);
                continue;
            }

            return Ok((
                RemoteObservation {
                    status,
                    declared_content_length,
                },
                RemoteInspectEvidence {
                    requested_url,
                    final_url,
                    attempts,
                },
            ));
        }
        unreachable!("HTTP inspection retry loop always returns")
    }
}

#[cfg(feature = "http-ureq")]
impl RemoteSource {
    fn download_into(
        self,
        resources: &UreqResources,
        stage: &mut std::fs::File,
        stage_path: &Path,
        mut prepared_resume: Option<PlannedResume>,
    ) -> Result<DownloadEvidence, DownloadError> {
        let mut attempts = Vec::new();
        let mut resume = None;
        let max_attempts = self
            .policy
            .retry
            .max_retries
            .saturating_add(u32::from(prepared_resume.is_some()));
        for attempt in 0..=max_attempts {
            let resume_context = prepared_resume.clone();
            let admission_wait = resources
                .admission
                .as_ref()
                .map(|admission| admission.enter_sync());
            let mut request = resources.agent.get(self.url.as_str());
            for (name, value) in &self.policy.headers {
                request = request.header(name, value);
            }
            if let Some(resume) = &resume_context {
                request = request.header("Range", format!("bytes={}-", resume.partial_bytes));
                request = request.header("If-Range", resume.validator.if_range_value());
            }
            let mut request_config = request.config().http_status_as_error(false);
            if let Some(timeout) = self.policy.timeout {
                request_config = request_config.timeout_global(Some(timeout));
            }
            let request = request_config.build();

            let mut response = match request.call() {
                Ok(response) => response,
                Err(err) => {
                    let will_retry = attempt < self.policy.retry.max_retries;
                    let planned_delay = will_retry.then(|| retry_delay(self.policy.retry, attempt));
                    attempts.push(
                        AttemptEvidence::new(attempt, AttemptOutcome::RetryableNetworkError)
                            .with_planned_delay(planned_delay)
                            .with_admission_wait(admission_wait),
                    );
                    if let Some(delay) = planned_delay {
                        (resources.delay)(delay);
                        continue;
                    }
                    return Err(DownloadError::transport(
                        &self.url,
                        TransportPhase::SendRequest,
                        err.to_string(),
                        attempts,
                        resume,
                    ));
                }
            };

            let status = response.status().as_u16();
            let content_length = response.body().content_length();
            let header = |name| response.headers().get(name).and_then(|v| v.to_str().ok());
            let facts = ResponseFacts {
                status,
                content_length,
                retry_after: header("retry-after")
                    .and_then(|value| parse_retry_after(value, SystemTime::now())),
                validator: selected_response_validator(
                    header("etag"),
                    header("last-modified"),
                    header("date"),
                ),
                etag: header("etag").map(str::to_owned),
                last_modified: header("last-modified").map(str::to_owned),
                content_range: header("content-range").map(str::to_owned),
            };
            let (append_resume, response_validator) = match facts.admit(
                &self,
                attempt,
                resume_context,
                admission_wait,
                &mut attempts,
                &mut resume,
            )? {
                ResponseAdmission::Restart => {
                    prepared_resume = None;
                    continue;
                }
                ResponseAdmission::Retry(wait) => {
                    (resources.delay)(wait);
                    continue;
                }
                ResponseAdmission::Transfer(append, validator) => (append, validator),
            };

            let initial_bytes = if let Some(append) = &append_resume {
                append.resume.partial_bytes
            } else {
                0
            };
            stage.set_len(initial_bytes).map_err(|error| {
                self.url
                    .local_error("reset download stage", stage_path, error)
            })?;
            stage
                .seek(std::io::SeekFrom::Start(initial_bytes))
                .map_err(|error| {
                    self.url
                        .local_error("seek download stage", stage_path, error)
                })?;
            let active_pacer = resources.byte_pacer.as_ref();
            let copy = match copy_response_body(
                response.body_mut().as_reader(),
                stage,
                self.policy.max_bytes,
                initial_bytes,
                active_pacer,
            ) {
                Ok(copy) => copy,
                Err(BodyCopyError::Transport { message, bytes }) => {
                    let will_retry = attempt < self.policy.retry.max_retries;
                    let planned_delay = will_retry.then(|| retry_delay(self.policy.retry, attempt));
                    attempts.push(
                        AttemptEvidence::transfer(
                            attempt,
                            status,
                            bytes,
                            content_length,
                            admission_wait,
                            AttemptOutcome::RetryableNetworkError,
                        )
                        .with_planned_delay(planned_delay),
                    );
                    if let Some(delay) = planned_delay {
                        (resources.delay)(delay);
                        continue;
                    }
                    return Err(DownloadError::transport(
                        &self.url,
                        TransportPhase::ReadBody,
                        message,
                        attempts,
                        resume,
                    ));
                }
                Err(BodyCopyError::LimitExceeded { max, actual, bytes }) => {
                    attempts.push(AttemptEvidence::transfer(
                        attempt,
                        status,
                        bytes,
                        content_length,
                        admission_wait,
                        AttemptOutcome::LimitExceeded,
                    ));
                    return Err(DownloadError::LimitExceeded {
                        url: self.url.as_url().clone(),
                        max,
                        actual,
                        attempts,
                        resume,
                    });
                }

                Err(BodyCopyError::Local {
                    action,
                    path,
                    source: io_error,
                    bytes,
                }) => {
                    attempts.push(AttemptEvidence::transfer(
                        attempt,
                        status,
                        bytes,
                        content_length,
                        admission_wait,
                        AttemptOutcome::LocalFailure,
                    ));
                    return Err(DownloadError::Local {
                        url: Some(self.url.as_url().clone()),
                        action,
                        path,
                        source: io_error,
                    });
                }
            };
            if let Some(append) = &append_resume {
                append.validate(copy.bytes, &self.url, &attempts, &resume)?;
            }
            stage.flush().map_err(|error| {
                self.url
                    .local_error("flush download temp file", stage_path, error)
            })?;

            if let Some(append) = append_resume {
                resume = Some(append.resume.into_evidence(ResumeOutcome::PartialAppended));
            }

            attempts.push(
                AttemptEvidence::transfer(
                    attempt,
                    status,
                    copy.bytes,
                    content_length,
                    admission_wait,
                    AttemptOutcome::Success,
                )
                .with_pacing_wait(copy.pacing_wait),
            );
            return Ok(DownloadEvidence {
                url: self.url.into_url(),
                status,
                bytes: copy.bytes,
                content_length,
                attempts,
                resume,
                validator: response_validator,
            });
        }
        unreachable!("retry loop always returns")
    }
}

#[cfg(feature = "http-ureq")]
fn reject_known_oversize(
    content_length: Option<u64>,
    max_bytes: Option<u64>,
) -> Result<(), (u64, u64)> {
    if let (Some(actual), Some(max)) = (content_length, max_bytes)
        && actual > max
    {
        return Err((max, actual));
    }
    Ok(())
}

#[cfg(any(test, feature = "http-ureq"))]
fn retry_delay(policy: RetryPolicy, retry_index: u32) -> Duration {
    let delay = policy
        .base_delay
        .saturating_mul(2_u32.saturating_pow(retry_index));
    policy.max_delay.map_or(delay, |max| delay.min(max))
}

#[cfg(feature = "http-ureq")]
fn planned_retry_delay(
    policy: RetryPolicy,
    retry_index: u32,
    retry_after: Option<Duration>,
) -> Duration {
    if policy.respect_retry_after
        && let Some(delay) = retry_after
    {
        return delay;
    }
    retry_delay(policy, retry_index)
}

#[cfg(any(test, feature = "http-ureq"))]
fn should_retry_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

#[cfg(any(test, feature = "http-ureq"))]
fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()
        .and_then(|retry_at| retry_at.duration_since(now).ok())
}

#[cfg(feature = "http-ureq")]
#[derive(Clone, Debug, Eq, PartialEq)]
struct PlannedResume {
    partial_path: PathBuf,
    partial_bytes: u64,
    validator: Validator,
}

#[cfg(feature = "http-ureq")]
struct ResponseFacts {
    status: u16,
    content_length: Option<u64>,
    retry_after: Option<Duration>,
    validator: Option<Validator>,
    etag: Option<String>,
    last_modified: Option<String>,
    content_range: Option<String>,
}

#[cfg(feature = "http-ureq")]
enum ResponseAdmission {
    Restart,
    Retry(Duration),
    Transfer(Option<AppendResume>, Option<Validator>),
}

#[cfg(feature = "http-ureq")]
struct AppendResume {
    resume: PlannedResume,
    range: ParsedContentRange,
    header: Option<String>,
}

#[cfg(feature = "http-ureq")]
impl PlannedResume {
    fn into_evidence(self, outcome: ResumeOutcome) -> ResumeEvidence {
        ResumeEvidence {
            outcome,
            partial_path: self.partial_path,
            partial_bytes: self.partial_bytes,
            validator: self.validator,
        }
    }

    fn admit_append(
        self,
        etag: Option<&str>,
        last_modified: Option<&str>,
        header: Option<String>,
        url: &RemoteUrl,
        attempts: &[AttemptEvidence],
        evidence: &Option<ResumeEvidence>,
    ) -> Result<AppendResume, DownloadError> {
        if !self.validator.permits_response(etag, last_modified) {
            return Err(DownloadError::Protocol {
                url: url.as_url().clone(),
                kind: ProtocolError::ResumeValidatorMismatch,
                attempts: attempts.to_vec(),
                resume: evidence.clone(),
            });
        }
        let range = header
            .as_deref()
            .and_then(|value| parse_content_range(value, self.partial_bytes))
            .ok_or_else(|| DownloadError::Protocol {
                url: url.as_url().clone(),
                kind: ProtocolError::InvalidContentRange {
                    expected_start: self.partial_bytes,
                    header: header.clone(),
                },
                attempts: attempts.to_vec(),
                resume: evidence.clone(),
            })?;
        Ok(AppendResume {
            resume: self,
            range,
            header,
        })
    }
}

#[cfg(feature = "http-ureq")]
impl ResponseFacts {
    fn evidence(
        &self,
        attempt: u32,
        admission_wait: Option<Duration>,
        outcome: AttemptOutcome,
    ) -> AttemptEvidence {
        AttemptEvidence::response(
            attempt,
            self.status,
            self.content_length,
            admission_wait,
            outcome,
        )
    }

    fn admit(
        self,
        source: &RemoteSource,
        attempt: u32,
        mut context: Option<PlannedResume>,
        admission_wait: Option<Duration>,
        attempts: &mut Vec<AttemptEvidence>,
        resume: &mut Option<ResumeEvidence>,
    ) -> Result<ResponseAdmission, DownloadError> {
        if let Some(admission) = self.admit_status(
            source,
            attempt,
            &mut context,
            admission_wait,
            attempts,
            resume,
        )? {
            return Ok(admission);
        }
        let append = if self.status == 206 {
            let context = context.ok_or_else(|| DownloadError::Protocol {
                url: source.url.as_url().clone(),
                kind: ProtocolError::UnexpectedPartialResponse,
                attempts: attempts.clone(),
                resume: resume.clone(),
            })?;
            Some(context.admit_append(
                self.etag.as_deref(),
                self.last_modified.as_deref(),
                self.content_range,
                &source.url,
                attempts,
                resume,
            )?)
        } else {
            if let Some(context) = context {
                *resume = Some(context.into_evidence(ResumeOutcome::RangeIgnoredRestarted));
            }
            None
        };
        Ok(ResponseAdmission::Transfer(append, self.validator))
    }

    fn admit_status(
        &self,
        source: &RemoteSource,
        attempt: u32,
        context: &mut Option<PlannedResume>,
        admission_wait: Option<Duration>,
        attempts: &mut Vec<AttemptEvidence>,
        resume: &mut Option<ResumeEvidence>,
    ) -> Result<Option<ResponseAdmission>, DownloadError> {
        if self.status == 416
            && let Some(context) = context.take()
        {
            attempts.push(self.evidence(
                attempt,
                admission_wait,
                AttemptOutcome::NonRetryableStatus,
            ));
            *resume = Some(context.into_evidence(ResumeOutcome::RangeUnsatisfiableRestarted));
            return Ok(Some(ResponseAdmission::Restart));
        }
        if !(200..300).contains(&self.status) {
            let retryable = should_retry_status(self.status);
            let planned = (retryable && attempt < source.policy.retry.max_retries)
                .then(|| planned_retry_delay(source.policy.retry, attempt, self.retry_after));
            let outcome = if retryable {
                AttemptOutcome::RetryableStatus
            } else {
                AttemptOutcome::NonRetryableStatus
            };
            attempts.push(
                self.evidence(attempt, admission_wait, outcome)
                    .with_retry_after(self.retry_after)
                    .with_planned_delay(planned),
            );
            return match planned {
                Some(wait) => Ok(Some(ResponseAdmission::Retry(wait))),
                None => Err(DownloadError::HttpStatus {
                    url: source.url.as_url().clone(),
                    status: self.status,
                    retryable,
                    attempts: std::mem::take(attempts),
                    resume: resume.take(),
                }),
            };
        }
        let Err((max, actual)) =
            reject_known_oversize(self.content_length, source.policy.max_bytes)
        else {
            return Ok(None);
        };
        attempts.push(self.evidence(attempt, admission_wait, AttemptOutcome::LimitExceeded));
        Err(DownloadError::LimitExceeded {
            url: source.url.as_url().clone(),
            max,
            actual,
            attempts: std::mem::take(attempts),
            resume: resume.take(),
        })
    }
}

#[cfg(feature = "http-ureq")]
impl AppendResume {
    fn validate(
        &self,
        bytes: u64,
        url: &RemoteUrl,
        attempts: &[AttemptEvidence],
        evidence: &Option<ResumeEvidence>,
    ) -> Result<(), DownloadError> {
        if self
            .range
            .matches_materialized_bytes(self.resume.partial_bytes, bytes)
        {
            return Ok(());
        }
        Err(DownloadError::Protocol {
            url: url.as_url().clone(),
            kind: ProtocolError::InvalidContentRange {
                expected_start: self.resume.partial_bytes,
                header: self.header.clone(),
            },
            attempts: attempts.to_vec(),
            resume: evidence.clone(),
        })
    }
}

fn normalize_http_date(time: SystemTime) -> Option<SystemTime> {
    const HTTP_DATE_UPPER_BOUND: u64 = 253_402_300_800;

    let seconds = time.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    (seconds < HTTP_DATE_UPPER_BOUND).then(|| SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
}

fn parse_strong_etag(value: &str) -> Option<String> {
    let value = value.trim();
    if value.starts_with("W/") || value.starts_with("w/") {
        return None;
    }
    let bytes = value.as_bytes();
    if bytes.len() < 2 || bytes.first() != Some(&b'"') || bytes.last() != Some(&b'"') {
        return None;
    }
    bytes[1..bytes.len() - 1]
        .iter()
        .all(|byte| *byte == 0x21 || (0x23..=0x7e).contains(byte) || *byte >= 0x80)
        .then(|| value.to_string())
}

#[cfg(any(test, feature = "http-ureq"))]
fn selected_response_validator(
    etag: Option<&str>,
    last_modified: Option<&str>,
    date: Option<&str>,
) -> Option<Validator> {
    etag.and_then(Validator::strong_etag).or_else(|| {
        let last_modified =
            last_modified.and_then(|value| httpdate::parse_http_date(value).ok())?;
        let date = date.and_then(|value| httpdate::parse_http_date(value).ok())?;
        Validator::strong_last_modified(last_modified, date)
    })
}

#[cfg(any(test, feature = "http-ureq"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParsedContentRange {
    end: u64,
    total: u64,
}

#[cfg(any(test, feature = "http-ureq"))]
impl ParsedContentRange {
    fn matches_materialized_bytes(self, start: u64, materialized_bytes: u64) -> bool {
        let Some(fragment_bytes) = self
            .end
            .checked_sub(start)
            .and_then(|bytes| bytes.checked_add(1))
        else {
            return false;
        };
        materialized_bytes.checked_sub(start) == Some(fragment_bytes)
            && materialized_bytes == self.total
    }
}

#[cfg(any(test, feature = "http-ureq"))]
fn parse_content_range(value: &str, expected_start: u64) -> Option<ParsedContentRange> {
    let range = value.trim().strip_prefix("bytes ")?;
    let (span, total) = range.split_once('/')?;
    let (start, end) = span.split_once('-')?;
    let start = start.parse::<u64>().ok()?;
    let end = end.parse::<u64>().ok()?;
    if start != expected_start || end < start {
        return None;
    }
    let total = total.parse::<u64>().ok()?;
    if end >= total || expected_start > total || end.checked_add(1) != Some(total) {
        return None;
    }
    Some(ParsedContentRange { end, total })
}

#[cfg(feature = "http-ureq")]
struct BodyCopyProgress {
    bytes: u64,
    pacing_wait: Duration,
}

#[cfg(feature = "http-ureq")]
enum BodyCopyError {
    Transport {
        message: String,
        bytes: u64,
    },
    LimitExceeded {
        max: u64,
        actual: u64,
        bytes: u64,
    },
    Local {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
        bytes: u64,
    },
}

#[cfg(feature = "http-ureq")]
fn copy_response_body(
    mut reader: impl Read,
    writer: &mut impl Write,
    max_bytes: Option<u64>,
    initial_bytes: u64,
    pacer: Option<&Arc<ByteRatePacer>>,
) -> Result<BodyCopyProgress, BodyCopyError> {
    let mut buffer = [0; 16 * 1024];
    let mut bytes = initial_bytes;
    let mut pacing_wait = Duration::ZERO;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|err| BodyCopyError::Transport {
                message: err.to_string(),
                bytes,
            })?;
        if read == 0 {
            return Ok(BodyCopyProgress { bytes, pacing_wait });
        }
        let actual = bytes.saturating_add(read as u64);
        if let Some(max) = max_bytes
            && actual > max
        {
            return Err(BodyCopyError::LimitExceeded { max, actual, bytes });
        }
        if let Some(pacer) = pacer {
            pacing_wait += pacer.before_chunk_sync(read as u64);
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|err| BodyCopyError::Local {
                action: "write download temp file",
                path: PathBuf::from("<temp>"),
                source: err,
                bytes,
            })?;
        bytes = actual;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "http-ureq")]
    #[cfg(feature = "http-ureq")]
    use std::io::Write as TestWrite;
    #[cfg(feature = "http-ureq")]
    use std::io::{BufRead, BufReader};
    #[cfg(feature = "http-ureq")]
    use std::net::{TcpListener, TcpStream};
    #[cfg(feature = "http-ureq")]
    use std::sync::{Arc as TestArc, Mutex, mpsc};
    #[cfg(feature = "http-ureq")]
    use std::thread;

    fn test_validator() -> Validator {
        Validator::strong_etag("\"abc\"").unwrap()
    }

    #[test]
    fn remote_url_rejects_unsupported_or_relative_urls() {
        assert!(matches!(
            RemoteUrl::parse("file:///tmp/file"),
            Err(RemoteUrlError::UnsupportedScheme { .. })
        ));
        assert!(matches!(
            RemoteUrl::parse("example.com/file"),
            Err(RemoteUrlError::Invalid { .. })
        ));
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn sync_rate_admission_shares_attempt_budget() {
        let admission = RateAdmission::new(AttemptRate::new(
            NonZeroU32::new(20).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ));

        let first = admission.enter_sync();
        let second = admission.enter_sync();

        assert_eq!(first, Duration::ZERO);
        assert!(second > Duration::ZERO);
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn sync_byte_rate_pacer_zero_bytes_is_immediate() {
        let pacer = ByteRatePacer::new(ByteRate::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ));

        assert_eq!(pacer.before_chunk_sync(0), Duration::ZERO);
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn sync_byte_rate_pacer_splits_chunks_larger_than_burst() {
        let pacer = ByteRatePacer::new(ByteRate::new(
            NonZeroU32::new(1_000).unwrap(),
            NonZeroU32::new(2).unwrap(),
        ));

        let waited = pacer.before_chunk_sync(3);

        assert!(waited > Duration::ZERO);
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn sync_byte_rate_pacer_shares_budget_across_calls() {
        let pacer = ByteRatePacer::new(ByteRate::new(
            NonZeroU32::new(10).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ));

        let first = pacer.before_chunk_sync(1);
        let second = pacer.before_chunk_sync(1);

        assert_eq!(first, Duration::ZERO);
        assert!(second > Duration::ZERO);
    }

    #[test]
    fn retry_policy_is_disabled_by_default_and_computes_delay() {
        assert_eq!(DownloadPolicy::default().retry, RetryPolicy::disabled());
        let policy = RetryPolicy::exponential(3, Duration::from_millis(25))
            .max_delay(Duration::from_millis(60));
        assert_eq!(retry_delay(policy, 0), Duration::from_millis(25));
        assert_eq!(retry_delay(policy, 1), Duration::from_millis(50));
        assert_eq!(retry_delay(policy, 2), Duration::from_millis(60));
        assert!(should_retry_status(503));
        assert!(!should_retry_status(404));
        assert_eq!(
            parse_retry_after("2", SystemTime::UNIX_EPOCH),
            Some(Duration::from_secs(2))
        );
    }

    #[test]
    fn strong_etag_parser_rejects_weak_etag() {
        assert_eq!(Validator::strong_etag("\"abc\""), Some(test_validator()));
        assert_eq!(Validator::strong_etag("W/\"abc\""), None);
    }

    #[test]
    fn strong_etag_parser_rejects_invalid_entity_tag_characters() {
        assert_eq!(Validator::strong_etag("\"a b\""), None);
        assert_eq!(Validator::strong_etag("\"a\"b\""), None);
        assert_eq!(Validator::strong_etag("\"a\u{7f}b\""), None);
        assert_eq!(Validator::strong_etag("\"a\nb\""), None);
        assert!(Validator::strong_etag("\"!#~é\"").is_some());
    }

    #[test]
    fn strong_last_modified_requires_the_rfc_date_gap() {
        let last_modified = SystemTime::UNIX_EPOCH;
        assert_eq!(
            Validator::strong_last_modified(last_modified, last_modified + Duration::from_secs(59)),
            None
        );
        assert!(
            Validator::strong_last_modified(last_modified, last_modified + Duration::from_secs(60))
                .is_some()
        );
    }

    #[test]
    fn strong_last_modified_normalizes_to_http_date_precision() {
        let last_modified =
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000) + Duration::from_millis(500);
        let date = last_modified + Duration::from_secs(60);
        let validator = Validator::strong_last_modified(last_modified, date).unwrap();
        let encoded = httpdate::fmt_http_date(last_modified);

        assert_eq!(validator.if_range_value(), encoded);
        assert!(validator.permits_response(None, Some(&encoded)));
    }

    #[test]
    fn strong_last_modified_rejects_times_outside_http_date_range() {
        assert_eq!(
            Validator::strong_last_modified(
                SystemTime::UNIX_EPOCH - Duration::from_secs(1),
                SystemTime::UNIX_EPOCH + Duration::from_secs(60),
            ),
            None
        );
        let after_http_date = SystemTime::UNIX_EPOCH + Duration::from_secs(253_402_300_800);
        assert_eq!(
            Validator::strong_last_modified(after_http_date, after_http_date),
            None
        );
    }

    #[test]
    fn selected_response_validator_prefers_strong_etag_over_last_modified() {
        let last_modified = "Wed, 21 Oct 2015 07:28:00 GMT";
        let date = "Wed, 21 Oct 2015 07:30:00 GMT";
        assert_eq!(
            selected_response_validator(Some("\"abc\""), Some(last_modified), Some(date)),
            Some(test_validator())
        );
        assert!(selected_response_validator(None, Some(last_modified), Some(date)).is_some());
        assert_eq!(
            selected_response_validator(None, Some(last_modified), None),
            None
        );
    }

    #[test]
    fn content_range_requires_expected_resume_start() {
        assert_eq!(
            parse_content_range("bytes 5-9/10", 5),
            Some(ParsedContentRange { end: 9, total: 10 })
        );
        assert_eq!(parse_content_range("bytes 4-9/10", 5), None);
        assert_eq!(parse_content_range("bytes 5-9/*", 5), None);
    }

    #[test]
    fn content_range_requires_a_known_terminal_interval() {
        assert_eq!(parse_content_range("bytes 5-7/11", 5), None);
        assert_eq!(parse_content_range("bytes 5-10/*", 5), None);
    }

    #[test]
    fn content_range_must_match_observed_fragment_bytes() {
        let range = parse_content_range("bytes 5-7/8", 5).unwrap();
        assert!(range.matches_materialized_bytes(5, 8));
        assert!(!range.matches_materialized_bytes(5, 7));
        assert!(!range.matches_materialized_bytes(5, 9));
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_inspect_uses_head_and_reports_declared_metadata() {
        let server = serve_once(200, b"body-not-materialized", &[]);
        let (observation, evidence) = UreqResources::default()
            .inspect_headers(
                &RemoteUrl::parse(&server.url).unwrap(),
                HttpInspectPolicy::default(),
            )
            .unwrap();
        let request = server.next_request();
        server.join();

        assert!(request.starts_with("HEAD /artifact.bin HTTP/1.1\r\n"));
        assert_eq!(observation.status, 200);
        assert_eq!(
            observation.declared_content_length,
            Some(b"body-not-materialized".len() as u64)
        );
        assert_eq!(evidence.attempts.len(), 1);
        assert_eq!(evidence.requested_url, evidence.final_url);
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_inspect_returns_non_success_status_as_observation_without_get_fallback() {
        let server = serve_once(405, b"method not allowed", &[]);
        let (observation, evidence) = UreqResources::default()
            .inspect_headers(
                &RemoteUrl::parse(&server.url).unwrap(),
                HttpInspectPolicy::default(),
            )
            .unwrap();
        let request = server.next_request();
        server.join();

        assert!(request.starts_with("HEAD "));
        assert_eq!(observation.status, 405);
        assert_eq!(evidence.attempts.len(), 1);
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_inspect_retries_status_and_returns_final_observation() {
        let server = serve_sequence(vec![(503, b"retry", &[]), (404, b"missing", &[])]);
        let resources = UreqResources::default().with_admission(TestArc::new(RateAdmission::new(
            AttemptRate::new(NonZeroU32::new(1_000).unwrap(), NonZeroU32::new(2).unwrap()),
        )));
        let policy = HttpInspectPolicy::default().retry(RetryPolicy {
            max_retries: 1,
            base_delay: Duration::ZERO,
            max_delay: None,
            respect_retry_after: false,
        });
        let (observation, evidence) = resources
            .inspect_headers(&RemoteUrl::parse(&server.url).unwrap(), policy)
            .unwrap();
        assert!(server.next_request().starts_with("HEAD "));
        assert!(server.next_request().starts_with("HEAD "));
        server.join();

        assert_eq!(observation.status, 404);
        assert_eq!(evidence.attempts.len(), 2);
        assert_eq!(evidence.attempts[0].status, Some(503));
        assert_eq!(evidence.attempts[1].status, Some(404));
        assert_eq!(evidence.attempts[0].admission_wait, Some(Duration::ZERO));
        assert_eq!(evidence.attempts[1].admission_wait, Some(Duration::ZERO));
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_inspect_records_requested_and_final_redirect_urls() {
        let server = serve_redirect_then(200, b"final");
        let requested = RemoteUrl::parse(&server.url).unwrap();
        let (_, evidence) = UreqResources::default()
            .inspect_headers(&requested, HttpInspectPolicy::default())
            .unwrap();
        assert!(server.next_request().starts_with("HEAD /artifact.bin "));
        assert!(server.next_request().starts_with("HEAD /final "));
        server.join();

        assert_eq!(evidence.requested_url, requested.into_url());
        assert!(evidence.final_url.as_str().ends_with("/final"));
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_inspect_preserves_transport_attempt_evidence() {
        let (url, server) = closed_endpoint();

        let error = UreqResources::default()
            .inspect_headers(
                &RemoteUrl::parse(&url).unwrap(),
                HttpInspectPolicy::default(),
            )
            .unwrap_err();
        server.join().unwrap();

        match error {
            HttpInspectError::Transport { attempts, .. } => {
                assert_eq!(attempts.len(), 1);
                assert_eq!(attempts[0].status, None);
            }
            other => panic!("expected transport error, got {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_download_returns_owned_snapshot() {
        let body = b"downloaded bytes";
        let server = serve_once(200, body, &[]);
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

        let (artifact, evidence) = UreqResources::default().download(source).unwrap();
        server.join();

        let path = snapshot_bytes(artifact);
        assert_eq!(path, body);
        assert_eq!(evidence.status, 200);
        assert_eq!(evidence.bytes, body.len() as u64);
        assert_eq!(evidence.content_length, Some(body.len() as u64));
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_download_does_not_publish_materialize_target() {
        let server = serve_once(200, b"replacement", &[]);
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.bin");
        std::fs::write(&target, b"original").unwrap();
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

        let (artifact, _) = UreqResources::default().download(source).unwrap();
        server.join();

        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        drop(artifact);
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_download_does_not_create_target_parent() {
        let server = serve_once(200, b"staged", &[]);
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("absent/target.bin");
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

        let (artifact, _) = UreqResources::default().download(source).unwrap();
        server.join();

        assert!(!target.parent().unwrap().exists());
        assert_eq!(snapshot_bytes(artifact), b"staged");
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_download_rejects_non_success_status_without_touching_target() {
        let server = serve_once(404, b"not found", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        std::fs::write(&destination, b"old").unwrap();
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

        let error = UreqResources::default().download(source).unwrap_err();
        server.join();

        assert!(matches!(
            error,
            DownloadError::HttpStatus {
                status: 404,
                retryable: false,
                ..
            }
        ));
        assert_eq!(std::fs::read(&destination).unwrap(), b"old");
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_download_enforces_max_bytes_without_publishing_target() {
        let server = serve_once(200, b"too large", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let policy = DownloadPolicy::default().max_bytes(3);
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

        let error = UreqResources::default().download(source).unwrap_err();
        server.join();

        assert!(matches!(error, DownloadError::LimitExceeded { max: 3, .. }));
        assert!(!destination.exists());
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn oversized_partial_restarts_with_a_bounded_full_download() {
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"too large").unwrap();
        let server = serve_once(200, b"ok", &[]);
        let policy = DownloadPolicy::default()
            .max_bytes(2)
            .resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::from_url_str(&server.url)
            .unwrap()
            .policy(policy);
        let (file, evidence) = UreqResources::default().download(source).unwrap();
        assert_eq!(snapshot_bytes(file), b"ok");
        assert!(evidence.resume.is_none());
        assert!(!request_has_header_name(&server.next_request(), "range"));
        server.join();
        assert_eq!(std::fs::read(partial).unwrap(), b"too large");
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_resume_limit_evidence_records_materialized_partial_bytes() {
        let server = serve_once(206, b" world", &[("Content-Range", "bytes 5-10/11")]);
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let policy = DownloadPolicy::default()
            .max_bytes(8)
            .resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

        let error = UreqResources::default().download(source).unwrap_err();
        server.join();

        let attempts = match error {
            DownloadError::LimitExceeded { attempts, .. } => attempts,
            other => panic!("expected limit error, got {other:?}"),
        };
        assert_eq!(attempts.last().unwrap().bytes, 5);
        assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_retries_retryable_status_and_records_attempts() {
        let server = serve_sequence(vec![
            (503, b"busy", &[("Retry-After", "2")]),
            (200, b"ok", &[]),
        ]);
        let policy =
            DownloadPolicy::default().retry(RetryPolicy::exponential(1, Duration::from_millis(10)));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
        let sleeps = TestArc::new(Mutex::new(Vec::new()));
        let resources = UreqResources::default().with_delay({
            let sleeps = TestArc::clone(&sleeps);
            TestArc::new(move |duration| sleeps.lock().unwrap().push(duration))
        });
        let (artifact, evidence) = resources.download(source).unwrap();
        server.join();

        assert_eq!(snapshot_bytes(artifact), b"ok");
        assert_eq!(*sleeps.lock().unwrap(), vec![Duration::from_secs(2)]);
        assert_eq!(evidence.attempts.len(), 2);
        assert_eq!(evidence.attempts[0].status, Some(503));
        assert_eq!(
            evidence.attempts[0].outcome,
            AttemptOutcome::RetryableStatus
        );
        assert_eq!(evidence.attempts[1].outcome, AttemptOutcome::Success);
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_concrete_byte_rate_pacer_downloads() {
        let server = serve_once(200, b"concrete paced", &[]);
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap())
            .policy(DownloadPolicy::default());
        let pacer = ByteRatePacer::new(ByteRate::new(
            NonZeroU32::new(1_000_000).unwrap(),
            NonZeroU32::new(16_384).unwrap(),
        ));
        let resources = UreqResources::default().with_byte_pacer(TestArc::new(pacer));
        let (artifact, evidence) = resources.download(source).unwrap();
        server.join();

        assert_eq!(snapshot_bytes(artifact), b"concrete paced");
        assert_eq!(evidence.attempts[0].pacing_wait, Duration::ZERO);
        assert_eq!(evidence.attempts[0].outcome, AttemptOutcome::Success);
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_concrete_rate_admission_downloads() {
        let server = serve_once(200, b"rate admitted", &[]);
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap())
            .policy(DownloadPolicy::default());
        let admission = RateAdmission::new(AttemptRate::new(
            NonZeroU32::new(1_000).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ));
        let resources = UreqResources::default().with_admission(TestArc::new(admission));
        let (artifact, evidence) = resources.download(source).unwrap();
        server.join();

        assert_eq!(snapshot_bytes(artifact), b"rate admitted");
        assert_eq!(evidence.attempts[0].admission_wait, Some(Duration::ZERO));
        assert_eq!(evidence.attempts[0].outcome, AttemptOutcome::Success);
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_resume_206_appends_after_valid_content_range() {
        let server = serve_sequence(vec![(
            206,
            b" world",
            &[("Content-Range", "bytes 5-10/11")],
        )]);
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let policy =
            DownloadPolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

        let (artifact, evidence) = UreqResources::default().download(source).unwrap();
        server.join();

        assert_eq!(snapshot_bytes(artifact), b"hello world");
        assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
        assert_eq!(evidence.bytes, 11);
        assert_eq!(
            evidence.resume.unwrap().outcome,
            ResumeOutcome::PartialAppended
        );
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_resume_200_to_range_restarts_full_with_fresh_stage() {
        let server = serve_sequence(vec![(200, b"fresh", &[])]);
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"stale").unwrap();
        let policy =
            DownloadPolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

        let (artifact, evidence) = UreqResources::default().download(source).unwrap();
        server.join();

        assert_eq!(snapshot_bytes(artifact), b"fresh");
        assert_eq!(std::fs::read(&partial).unwrap(), b"stale");
        assert_eq!(
            evidence.resume.unwrap().outcome,
            ResumeOutcome::RangeIgnoredRestarted
        );
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_resume_416_restarts_once_without_range_headers() {
        let server = serve_sequence(vec![(416, b"", &[]), (200, b"fresh", &[])]);
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"stale partial").unwrap();
        let policy =
            DownloadPolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

        let (artifact, evidence) = UreqResources::default().download(source).unwrap();
        let first_request = server.next_request();
        let second_request = server.next_request();
        server.join();

        assert!(request_has_header_name(&first_request, "Range"));
        assert!(request_has_header_name(&first_request, "If-Range"));
        assert!(!request_has_header_name(&second_request, "Range"));
        assert!(!request_has_header_name(&second_request, "If-Range"));
        assert_eq!(snapshot_bytes(artifact), b"fresh");
        assert_eq!(std::fs::read(&partial).unwrap(), b"stale partial");
        assert_eq!(
            evidence.resume.unwrap().outcome,
            ResumeOutcome::RangeUnsatisfiableRestarted
        );
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_resume_missing_content_range_rejects_without_publishing_target() {
        let server = serve_sequence(vec![(206, b" world", &[])]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let policy =
            DownloadPolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

        let error = UreqResources::default().download(source).unwrap_err();
        server.join();

        assert!(matches!(
            error,
            DownloadError::Protocol {
                kind: ProtocolError::InvalidContentRange { .. },
                ..
            }
        ));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_resume_rejects_body_shorter_than_declared_range() {
        let server = serve_sequence(vec![(206, b" wo", &[("Content-Range", "bytes 5-10/11")])]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let policy =
            DownloadPolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

        let error = UreqResources::default().download(source).unwrap_err();
        server.join();

        assert!(matches!(
            error,
            DownloadError::Protocol {
                kind: ProtocolError::InvalidContentRange { .. },
                ..
            }
        ));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_resume_uses_prepared_snapshot_after_caller_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let partial_to_truncate = partial.clone();
        let server = serve_once_with_before_response(
            206,
            b" world",
            &[("Content-Range", "bytes 5-10/11"), ("ETag", "\"abc\"")],
            move || std::fs::write(partial_to_truncate, b"hel").unwrap(),
        );
        let policy =
            DownloadPolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

        let (artifact, evidence) = UreqResources::default().download(source).unwrap();
        server.join();

        assert!(!destination.exists());
        assert_eq!(snapshot_bytes(artifact), b"hello world");
        assert_eq!(evidence.resume.unwrap().partial_bytes, 5);
        assert_eq!(std::fs::read(&partial).unwrap(), b"hel");
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_if_range_resume_sends_range_and_if_range_and_appends_206() {
        let server = serve_sequence(vec![(
            206,
            b" world",
            &[("Content-Range", "bytes 5-10/11"), ("ETag", "\"abc\"")],
        )]);
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let validator = test_validator();
        let policy =
            DownloadPolicy::default().resume(ResumePolicy::if_range(&partial, validator.clone()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

        let (artifact, evidence) = UreqResources::default().download(source).unwrap();
        let request = server.next_request();
        server.join();

        assert!(request_has_header(&request, "range", "bytes=5-"));
        assert!(request_has_header(&request, "if-range", "\"abc\""));
        assert_eq!(snapshot_bytes(artifact), b"hello world");
        let resume = evidence.resume.unwrap();
        assert_eq!(resume.outcome, ResumeOutcome::PartialAppended);
        assert_eq!(resume.validator, validator);
        assert_eq!(evidence.validator, Some(test_validator()));
    }

    #[test]
    #[cfg(feature = "http-ureq")]
    fn ureq_if_range_rejects_conflicting_response_validator() {
        let server = serve_sequence(vec![(
            206,
            b" world",
            &[("Content-Range", "bytes 5-10/11"), ("ETag", "\"next\"")],
        )]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let policy =
            DownloadPolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

        let error = UreqResources::default().download(source).unwrap_err();
        server.join();

        assert!(matches!(
            error,
            DownloadError::Protocol {
                kind: ProtocolError::ResumeValidatorMismatch,
                ..
            }
        ));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
    }

    #[cfg(feature = "http-ureq")]
    fn snapshot_bytes(mut file: crate::local::FileSnapshot) -> Vec<u8> {
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut bytes).unwrap();
        bytes
    }

    #[cfg(feature = "http-ureq")]
    struct TestServer {
        url: String,
        handle: thread::JoinHandle<()>,
        requests: mpsc::Receiver<String>,
    }

    #[cfg(feature = "http-ureq")]
    impl TestServer {
        fn next_request(&self) -> String {
            self.requests.recv().unwrap()
        }

        fn join(self) {
            self.handle.join().unwrap();
        }
    }

    /// A transport failure owned by the test: accept one connection and close it without an HTTP
    /// response. This needs neither a port that stays free after `drop` nor any host networking
    /// assumption. Default HTTP resources ignore ambient proxy settings.
    #[cfg(feature = "http-ureq")]
    fn closed_endpoint() -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            drop(stream);
        });
        (format!("http://{address}/resource"), handle)
    }

    #[cfg(feature = "http-ureq")]
    fn serve_once(
        status: u16,
        body: &'static [u8],
        headers: &'static [(&'static str, &'static str)],
    ) -> TestServer {
        serve_sequence(vec![(status, body, headers)])
    }

    #[cfg(feature = "http-ureq")]
    fn request_has_header(request: &str, name: &str, value: &str) -> bool {
        request.lines().any(|line| {
            line.split_once(':')
                .is_some_and(|(actual_name, actual_value)| {
                    actual_name.eq_ignore_ascii_case(name) && actual_value.trim() == value
                })
        })
    }

    #[cfg(feature = "http-ureq")]
    fn request_has_header_name(request: &str, name: &str) -> bool {
        request.lines().any(|line| {
            line.split_once(':')
                .is_some_and(|(actual_name, _)| actual_name.eq_ignore_ascii_case(name))
        })
    }

    #[cfg(feature = "http-ureq")]
    type TestResponse = (u16, &'static [u8], &'static [(&'static str, &'static str)]);

    #[cfg(feature = "http-ureq")]
    fn serve_once_with_before_response(
        status: u16,
        body: &'static [u8],
        headers: &'static [(&'static str, &'static str)],
        before_response: impl FnOnce() + Send + 'static,
    ) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (requests, request_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_request(
                stream,
                status,
                body,
                headers,
                Some(Box::new(before_response)),
                &requests,
            );
        });
        TestServer {
            url: format!("http://{addr}/artifact.bin"),
            handle,
            requests: request_rx,
        }
    }

    #[cfg(feature = "http-ureq")]
    fn serve_sequence(responses: Vec<TestResponse>) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (requests, request_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            for (status, body, headers) in responses {
                let (stream, _) = listener.accept().unwrap();
                handle_request(stream, status, body, headers, None, &requests);
            }
        });
        TestServer {
            url: format!("http://{addr}/artifact.bin"),
            handle,
            requests: request_rx,
        }
    }

    #[cfg(feature = "http-ureq")]
    fn serve_redirect_then(status: u16, body: &'static [u8]) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (requests, request_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let location = format!("http://{addr}/final");
            let (first, _) = listener.accept().unwrap();
            handle_request(
                first,
                302,
                &[],
                &[("Location", location.as_str())],
                None,
                &requests,
            );
            let (second, _) = listener.accept().unwrap();
            handle_request(second, status, body, &[], None, &requests);
        });
        TestServer {
            url: format!("http://{addr}/artifact.bin"),
            handle,
            requests: request_rx,
        }
    }

    #[cfg(feature = "http-ureq")]
    fn handle_request(
        mut stream: TcpStream,
        status: u16,
        body: &[u8],
        headers: &[(&str, &str)],
        before_response: Option<Box<dyn FnOnce() + Send>>,
        requests: &mpsc::Sender<String>,
    ) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        let mut request = String::new();
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            request.push_str(&line);
            if line == "\r\n" || line.is_empty() {
                break;
            }
        }
        requests.send(request).unwrap();
        if let Some(before_response) = before_response {
            before_response();
        }

        let reason = match status {
            200 => "OK",
            404 => "Not Found",
            _ => "Status",
        };
        write!(
            stream,
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        )
        .unwrap();
        for (name, value) in headers {
            write!(stream, "{name}: {value}\r\n").unwrap();
        }
        write!(stream, "\r\n").unwrap();
        TestWrite::write_all(&mut stream, body).unwrap();
        TestWrite::flush(&mut stream).unwrap();
    }
}
