//! HTTP resource semantics and concrete acquisition/inspection adapters.
//!
//! Inspection uses HEAD only. A received final status is an observation rather than an acquisition
//! failure. Evidence records the requested URL, the post-redirect final URL, method, admission wait,
//! retry status, and planned delay. `declared_content_length` is response metadata: it is not body
//! bytes observed by Pulith, artifact identity, validator continuity, provenance, or trust evidence.
//! Inspection never falls back to GET, copies a response body, stages a file, or publishes a target.

use std::fmt;
#[cfg(feature = "http-async")]
use std::future::Future;
use std::io;
#[cfg(feature = "http-sync")]
use std::io::{Read, Write};
use std::num::NonZeroU32;
#[cfg(any(feature = "http-async", feature = "http-sync"))]
use std::path::Path;
use std::path::PathBuf;
#[cfg(feature = "http-async")]
use std::pin::Pin;
#[cfg(any(feature = "http-async", feature = "http-sync"))]
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[cfg(any(feature = "http-async", feature = "http-sync"))]
use governor::clock::Clock;

#[cfg(any(feature = "http-async", feature = "http-sync"))]
use crate::local::LocalMaterial;
#[cfg(feature = "http-sync")]
use crate::{Acquire, Inspect};
#[cfg(any(feature = "http-async", feature = "http-sync"))]
use crate::{Acquired, Inspected, Materialize};
#[cfg(feature = "http-async")]
use crate::{AsyncAcquire, AsyncInspect};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RemoteUrlError {
    Invalid { input: String },
    UnsupportedScheme { scheme: String },
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
pub enum AcquireError {
    RemoteUrl(RemoteUrlError),
    HttpStatus {
        url: url::Url,
        status: u16,
        retryable: bool,
        attempts: Vec<AttemptEvidence>,
        resume: Option<ResumeEvidence>,
    },
    Transport {
        url: url::Url,
        phase: TransportPhase,
        message: String,
        attempts: Vec<AttemptEvidence>,
        resume: Option<ResumeEvidence>,
    },
    Protocol {
        url: url::Url,
        kind: ProtocolError,
        attempts: Vec<AttemptEvidence>,
        resume: Option<ResumeEvidence>,
    },
    LimitExceeded {
        url: url::Url,
        max: u64,
        actual: u64,
        attempts: Vec<AttemptEvidence>,
        resume: Option<ResumeEvidence>,
    },

    Local {
        url: Option<url::Url>,
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportPhase {
    SendRequest,
    ReadBody,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    UnexpectedPartialResponse,
    ResumeValidatorMismatch,
    InvalidContentRange {
        expected_start: u64,
        header: Option<String>,
    },
}

impl fmt::Display for AcquireError {
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

impl std::error::Error for AcquireError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RemoteUrl(error) => Some(error),
            Self::Local { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl AcquireError {
    #[cfg(any(feature = "http-sync", feature = "http-async"))]
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

    #[cfg(any(feature = "http-sync", feature = "http-async"))]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteUrl {
    url: url::Url,
}

impl RemoteUrl {
    pub fn parse(input: &str) -> Result<Self, RemoteUrlError> {
        let url = url::Url::parse(input).map_err(|_| RemoteUrlError::Invalid {
            input: input.to_string(),
        })?;
        match url.scheme() {
            "http" | "https" => Ok(Self { url }),
            scheme => Err(RemoteUrlError::UnsupportedScheme {
                scheme: scheme.to_string(),
            }),
        }
    }

    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    pub fn as_url(&self) -> &url::Url {
        &self.url
    }

    pub fn into_url(self) -> url::Url {
        self.url
    }
}

impl From<RemoteUrlError> for AcquireError {
    fn from(error: RemoteUrlError) -> Self {
        Self::RemoteUrl(error)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AcquirePolicy {
    pub timeout: Option<Duration>,
    pub max_bytes: Option<u64>,
    pub headers: Vec<(String, String)>,
    pub retry: RetryPolicy,
    pub resume: ResumePolicy,
}

impl AcquirePolicy {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = Some(max_bytes);
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

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
    pub const fn new(attempts_per_second: NonZeroU32, burst_attempts: NonZeroU32) -> Self {
        Self {
            attempts_per_second,
            burst_attempts,
        }
    }

    pub const fn attempts_per_second(self) -> NonZeroU32 {
        self.attempts_per_second
    }

    pub const fn burst_attempts(self) -> NonZeroU32 {
        self.burst_attempts
    }
}

/// A concrete shared GCRA gate for outbound attempt entry.
///
/// Cloning an `Arc<RateAdmission>` into multiple resources coordinates them
/// through one atomic limiter state. Sync and async implementations make the
/// same single-cell decision; only the wait effect differs. The returned
/// permit reports accumulated governor-requested wait, not method wall time.
///
/// This is an attempt-rate gate, not a semaphore or body-copy byte pacer.
pub struct RateAdmission {
    rate: AttemptRate,
    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    limiter: governor::DefaultDirectRateLimiter,
}

impl RateAdmission {
    pub fn new(rate: AttemptRate) -> Self {
        #[cfg(any(feature = "http-async", feature = "http-sync"))]
        let quota = governor::Quota::per_second(rate.attempts_per_second())
            .allow_burst(rate.burst_attempts());
        Self {
            rate,
            #[cfg(any(feature = "http-async", feature = "http-sync"))]
            limiter: governor::RateLimiter::direct(quota),
        }
    }

    pub const fn rate(&self) -> AttemptRate {
        self.rate
    }

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    fn check(&self) -> Option<Duration> {
        match self.limiter.check() {
            Ok(_) => None,
            Err(not_until) => Some(not_until.wait_time_from(self.limiter.clock().now())),
        }
    }

    #[cfg(feature = "http-sync")]
    fn enter_sync(&self) -> Duration {
        let mut waited = Duration::ZERO;
        while let Some(wait) = self.check() {
            std::thread::sleep(wait);
            waited = waited.saturating_add(wait);
        }
        waited
    }

    #[cfg(feature = "http-async")]
    async fn enter_async(&self) -> Duration {
        let mut waited = Duration::ZERO;
        while let Some(wait) = self.check() {
            tokio::time::sleep(wait).await;
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
    pub const fn new(bytes_per_second: NonZeroU32, burst_bytes: NonZeroU32) -> Self {
        Self {
            bytes_per_second,
            burst_bytes,
        }
    }

    pub const fn bytes_per_second(self) -> NonZeroU32 {
        self.bytes_per_second
    }

    pub const fn burst_bytes(self) -> NonZeroU32 {
        self.burst_bytes
    }
}

/// A concrete shared GCRA body-copy byte pacer.
///
/// Cloning an `Arc<ByteRatePacer>` into multiple resources coordinates them
/// through one atomic limiter state. Sync and async implementations make the
/// same bounded `check_n` decisions; only the wait effect differs. Zero-byte
/// calls are immediate.
///
/// This remains decoded body-copy pacing, not raw socket bandwidth control.
/// Cancellation after accepted sub-batches may conservatively consume budget
/// without staging the chunk; it cannot permit an overrun.
pub struct ByteRatePacer {
    rate: ByteRate,
    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    limiter: governor::DefaultDirectRateLimiter,
}

impl ByteRatePacer {
    pub fn new(rate: ByteRate) -> Self {
        #[cfg(any(feature = "http-async", feature = "http-sync"))]
        let quota =
            governor::Quota::per_second(rate.bytes_per_second()).allow_burst(rate.burst_bytes());
        Self {
            rate,
            #[cfg(any(feature = "http-async", feature = "http-sync"))]
            limiter: governor::RateLimiter::direct(quota),
        }
    }

    pub const fn rate(&self) -> ByteRate {
        self.rate
    }

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    fn next_batch(&self, remaining: u64) -> Option<NonZeroU32> {
        NonZeroU32::new(remaining.min(u64::from(self.rate.burst_bytes().get())) as u32)
    }

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    fn check_batch(&self, batch: NonZeroU32) -> Option<Duration> {
        match self.limiter.check_n(batch) {
            Ok(Ok(_)) => None,
            Ok(Err(not_until)) => Some(not_until.wait_time_from(self.limiter.clock().now())),
            Err(_) => unreachable!("pacing batches never exceed configured burst"),
        }
    }

    #[cfg(feature = "http-sync")]
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

    #[cfg(feature = "http-async")]
    async fn before_chunk_async(&self, bytes: u64) -> Duration {
        let mut remaining = bytes;
        let mut waited = Duration::ZERO;
        while let Some(batch) = self.next_batch(remaining) {
            while let Some(wait) = self.check_batch(batch) {
                tokio::time::sleep(wait).await;
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
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Option<Duration>,
    pub respect_retry_after: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::disabled()
    }
}

impl RetryPolicy {
    pub fn disabled() -> Self {
        Self {
            max_retries: 0,
            base_delay: Duration::ZERO,
            max_delay: None,
            respect_retry_after: true,
        }
    }

    pub fn exponential(max_retries: u32, base_delay: Duration) -> Self {
        Self {
            max_retries,
            base_delay,
            max_delay: None,
            respect_retry_after: true,
        }
    }

    pub fn max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = Some(max_delay);
        self
    }

    pub fn respect_retry_after(mut self, respect: bool) -> Self {
        self.respect_retry_after = respect;
        self
    }
}

/// Caller-selected execution policy for HTTP HEAD inspection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HttpInspectPolicy {
    pub timeout: Option<Duration>,
    pub retry: RetryPolicy,
}

impl HttpInspectPolicy {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }
}

/// HTTP-specific facts reported by a HEAD response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpObservation {
    pub status: u16,
    pub declared_content_length: Option<u64>,
}

/// Evidence for one HTTP inspection or admission attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpInspectAttemptEvidence {
    pub attempt: u32,
    pub status: Option<u16>,
    pub admission_wait: Option<Duration>,
    pub planned_delay: Option<Duration>,
}

#[cfg(any(feature = "http-async", feature = "http-sync"))]
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
pub struct HttpInspectEvidence {
    pub requested_url: url::Url,
    pub final_url: url::Url,
    pub attempts: Vec<HttpInspectAttemptEvidence>,
}

/// Failures that prevented HTTP inspection from receiving a final response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HttpInspectError {
    Transport {
        url: url::Url,
        message: String,
        attempts: Vec<HttpInspectAttemptEvidence>,
    },

    Protocol {
        url: url::Url,
        message: String,
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
    RestartOnly,
    IfRange {
        partial_path: PathBuf,
        validator: Validator,
    },
}

impl Default for ResumePolicy {
    fn default() -> Self {
        Self::restart_only()
    }
}

impl ResumePolicy {
    pub fn restart_only() -> Self {
        Self::RestartOnly
    }

    pub fn if_range(partial_path: impl Into<PathBuf>, validator: Validator) -> Self {
        Self::IfRange {
            partial_path: partial_path.into(),
            validator,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

    #[cfg(any(test, feature = "http-sync", feature = "http-async"))]
    fn if_range_value(&self) -> String {
        match &self.kind {
            ValidatorKind::StrongEtag(value) => value.clone(),
            ValidatorKind::StrongLastModified(time) => httpdate::fmt_http_date(*time),
        }
    }

    #[cfg(any(test, feature = "http-sync", feature = "http-async"))]
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

/// HTTP source identity and acquisition policy; it carries no publication destination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSource {
    pub(crate) url: RemoteUrl,
    pub(crate) policy: AcquirePolicy,
}

impl RemoteSource {
    pub fn new(url: RemoteUrl) -> Self {
        Self {
            url,
            policy: AcquirePolicy::default(),
        }
    }

    pub fn policy(mut self, policy: AcquirePolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn url(&self) -> &RemoteUrl {
        &self.url
    }
}

/// Observed HTTP acquisition facts, excluding the ephemeral stage owned by the output material.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpAcquireEvidence {
    pub url: url::Url,
    pub status: u16,
    pub bytes: u64,
    pub content_length: Option<u64>,
    pub attempts: Vec<AttemptEvidence>,
    pub resume: Option<ResumeEvidence>,
    pub validator: Option<Validator>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumeEvidence {
    pub outcome: ResumeOutcome,
    pub partial_path: PathBuf,
    pub partial_bytes: u64,
    pub validator: Validator,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResumeOutcome {
    PartialAppended,
    RangeIgnoredRestarted,
    RangeUnsatisfiableRestarted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptEvidence {
    pub attempt: u32,
    pub status: Option<u16>,
    pub bytes: u64,
    pub content_length: Option<u64>,
    pub retry_after: Option<Duration>,
    pub planned_delay: Option<Duration>,
    pub admission_wait: Option<Duration>,
    pub pacing_wait: Duration,
    pub outcome: AttemptOutcome,
}

#[cfg(any(test, feature = "http-sync", feature = "http-async"))]
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

    #[cfg(any(feature = "http-sync", feature = "http-async"))]
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

    #[cfg(any(feature = "http-sync", feature = "http-async"))]
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

    #[cfg(any(feature = "http-sync", feature = "http-async"))]
    fn with_pacing_wait(mut self, pacing_wait: Duration) -> Self {
        self.pacing_wait = pacing_wait;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttemptOutcome {
    Success,
    RetryableStatus,
    RetryableNetworkError,
    NonRetryableStatus,
    NonRetryableNetworkError,
    LocalFailure,
    LimitExceeded,
}

#[cfg(feature = "http-sync")]
type SyncSleep = Arc<dyn Fn(Duration) + Send + Sync>;

#[cfg(feature = "http-async")]
type AsyncSleepFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

#[cfg(feature = "http-async")]
type AsyncSleep = Arc<dyn Fn(Duration) -> AsyncSleepFuture + Send + Sync>;

#[cfg(feature = "http-sync")]
#[derive(Clone)]
pub struct SyncHttpResources {
    agent: ureq::Agent,
    delay: SyncSleep,
    admission: Option<Arc<RateAdmission>>,
    byte_pacer: Option<Arc<ByteRatePacer>>,
}

#[cfg(feature = "http-sync")]
impl Default for SyncHttpResources {
    fn default() -> Self {
        Self {
            agent: ureq::Agent::new_with_defaults(),
            delay: Arc::new(std::thread::sleep),
            admission: None,
            byte_pacer: None,
        }
    }
}

#[cfg(feature = "http-sync")]
impl SyncHttpResources {
    pub fn from_agent(agent: ureq::Agent) -> Self {
        Self {
            agent,
            ..Self::default()
        }
    }

    #[cfg(test)]
    fn with_delay(mut self, delay: SyncSleep) -> Self {
        self.delay = delay;
        self
    }

    pub fn with_admission(mut self, admission: Arc<RateAdmission>) -> Self {
        self.admission = Some(admission);
        self
    }

    pub fn with_byte_pacer(mut self, byte_pacer: Arc<ByteRatePacer>) -> Self {
        self.byte_pacer = Some(byte_pacer);
        self
    }
}

#[cfg(feature = "http-sync")]
impl std::fmt::Debug for SyncHttpResources {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SyncHttpResources")
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "http-sync")]
/// Synchronous HEAD inspection implemented by `ureq`.
#[derive(Clone, Debug)]
pub struct SyncHttpInspect {
    resources: SyncHttpResources,
    policy: HttpInspectPolicy,
}

#[cfg(feature = "http-sync")]
impl Default for SyncHttpInspect {
    fn default() -> Self {
        Self::new(SyncHttpResources::default(), HttpInspectPolicy::default())
    }
}

#[cfg(feature = "http-sync")]
impl SyncHttpInspect {
    pub fn new(resources: SyncHttpResources, policy: HttpInspectPolicy) -> Self {
        Self { resources, policy }
    }
}

#[cfg(feature = "http-sync")]
impl Inspect<RemoteUrl> for SyncHttpInspect {
    type Error = HttpInspectError;
    type Output = Inspected<RemoteUrl, HttpObservation, HttpInspectEvidence>;

    fn inspect(&self, node: RemoteUrl) -> Result<Self::Output, Self::Error> {
        use ureq::ResponseExt as _;

        let requested_url = node.as_url().clone();
        let mut attempts = Vec::new();
        for attempt in 0..=self.policy.retry.max_retries {
            let admission_wait = self
                .resources
                .admission
                .as_ref()
                .map(|admission| admission.enter_sync());

            let mut request_config = self
                .resources
                .agent
                .head(node.as_str())
                .config()
                .http_status_as_error(false);
            if let Some(timeout) = self.policy.timeout {
                request_config = request_config.timeout_global(Some(timeout));
            }
            let response = match request_config.build().call() {
                Ok(response) => response,
                Err(error) => {
                    let will_retry = attempt < self.policy.retry.max_retries;
                    let planned_delay = will_retry.then(|| retry_delay(self.policy.retry, attempt));
                    attempts.push(
                        HttpInspectAttemptEvidence::new(attempt, None, admission_wait)
                            .with_planned_delay(planned_delay),
                    );
                    if let Some(delay) = planned_delay {
                        (self.resources.delay)(delay);
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
            let will_retry = should_retry_status(status) && attempt < self.policy.retry.max_retries;
            let planned_delay =
                will_retry.then(|| planned_retry_delay(self.policy.retry, attempt, retry_after));
            attempts.push(
                HttpInspectAttemptEvidence::new(attempt, Some(status), admission_wait)
                    .with_planned_delay(planned_delay),
            );
            if let Some(delay) = planned_delay {
                (self.resources.delay)(delay);
                continue;
            }

            return Ok(Inspected {
                input: node,
                observation: HttpObservation {
                    status,
                    declared_content_length,
                },
                evidence: HttpInspectEvidence {
                    requested_url,
                    final_url,
                    attempts,
                },
            });
        }
        unreachable!("HTTP inspection retry loop always returns")
    }
}

#[cfg(feature = "http-sync")]
#[derive(Clone, Debug, Default)]
pub struct SyncHttpAcquire {
    resources: SyncHttpResources,
}

#[cfg(feature = "http-sync")]
impl SyncHttpAcquire {
    pub fn new(resources: SyncHttpResources) -> Self {
        Self { resources }
    }
}

#[cfg(feature = "http-sync")]
impl<I, T> Acquire<Materialize<I, RemoteSource, T>> for SyncHttpAcquire {
    type Error = AcquireError;
    type Output = Acquired<Materialize<I, RemoteSource, T>, LocalMaterial, HttpAcquireEvidence>;

    fn acquire(&self, node: Materialize<I, RemoteSource, T>) -> Result<Self::Output, Self::Error> {
        let source = node.source.clone();

        let mut attempts = Vec::new();
        let mut resume = None;
        let mut resume_suppressed = false;
        let max_attempts = source
            .policy
            .retry
            .max_retries
            .saturating_add(u32::from(planned_resume(&source.policy.resume).is_some()));
        for attempt in 0..=max_attempts {
            let resume_context = (!resume_suppressed)
                .then(|| planned_resume(&source.policy.resume))
                .flatten();
            let admission_wait = self
                .resources
                .admission
                .as_ref()
                .map(|admission| admission.enter_sync());
            let mut request = self.resources.agent.get(source.url.as_str());
            for (name, value) in &source.policy.headers {
                request = request.header(name, value);
            }
            if let Some(resume) = &resume_context {
                request = request.header("Range", format!("bytes={}-", resume.partial_bytes));
                request = request.header("If-Range", resume.validator.if_range_value());
            }
            let mut request_config = request.config().http_status_as_error(false);
            if let Some(timeout) = source.policy.timeout {
                request_config = request_config.timeout_global(Some(timeout));
            }
            let request = request_config.build();

            let mut response = match request.call() {
                Ok(response) => response,
                Err(err) => {
                    let will_retry = attempt < source.policy.retry.max_retries;
                    let planned_delay =
                        will_retry.then(|| retry_delay(source.policy.retry, attempt));
                    attempts.push(
                        AttemptEvidence::new(attempt, AttemptOutcome::RetryableNetworkError)
                            .with_planned_delay(planned_delay)
                            .with_admission_wait(admission_wait),
                    );
                    if let Some(delay) = planned_delay {
                        (self.resources.delay)(delay);
                        continue;
                    }
                    return Err(AcquireError::transport(
                        &source.url,
                        TransportPhase::SendRequest,
                        err.to_string(),
                        attempts,
                        resume,
                    ));
                }
            };

            let status = response.status().as_u16();
            let content_length = response.body().content_length();
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| parse_retry_after(value, SystemTime::now()));
            let response_validator = selected_response_validator(
                response
                    .headers()
                    .get("etag")
                    .and_then(|value| value.to_str().ok()),
                response
                    .headers()
                    .get("last-modified")
                    .and_then(|value| value.to_str().ok()),
                response
                    .headers()
                    .get("date")
                    .and_then(|value| value.to_str().ok()),
            );
            if status == 416
                && let Some(resume_context) = resume_context
            {
                attempts.push(AttemptEvidence::response(
                    attempt,
                    status,
                    content_length,
                    admission_wait,
                    AttemptOutcome::NonRetryableStatus,
                ));
                resume =
                    Some(resume_context.into_evidence(ResumeOutcome::RangeUnsatisfiableRestarted));
                resume_suppressed = true;
                continue;
            }
            if !response.status().is_success() {
                let retryable = should_retry_status(status);
                let will_retry = retryable && attempt < source.policy.retry.max_retries;
                let planned_delay = will_retry
                    .then(|| planned_retry_delay(source.policy.retry, attempt, retry_after));
                attempts.push(
                    AttemptEvidence::response(
                        attempt,
                        status,
                        content_length,
                        admission_wait,
                        if retryable {
                            AttemptOutcome::RetryableStatus
                        } else {
                            AttemptOutcome::NonRetryableStatus
                        },
                    )
                    .with_retry_after(retry_after)
                    .with_planned_delay(planned_delay),
                );
                if let Some(delay) = planned_delay {
                    (self.resources.delay)(delay);
                    continue;
                }
                return Err(AcquireError::HttpStatus {
                    url: source.url.as_url().clone(),
                    status,
                    retryable,
                    attempts,
                    resume,
                });
            }
            if let Err((max, actual)) =
                reject_known_oversize(content_length, source.policy.max_bytes)
            {
                attempts.push(AttemptEvidence::response(
                    attempt,
                    status,
                    content_length,
                    admission_wait,
                    AttemptOutcome::LimitExceeded,
                ));
                return Err(AcquireError::LimitExceeded {
                    url: source.url.as_url().clone(),
                    max,
                    actual,
                    attempts,
                    resume,
                });
            }

            let append_resume = if status == 206 {
                let resume_context =
                    resume_context
                        .as_ref()
                        .ok_or_else(|| AcquireError::Protocol {
                            url: source.url.as_url().clone(),
                            kind: ProtocolError::UnexpectedPartialResponse,
                            attempts: attempts.clone(),
                            resume: resume.clone(),
                        })?;
                let response_etag = response
                    .headers()
                    .get("etag")
                    .and_then(|value| value.to_str().ok());
                let response_last_modified = response
                    .headers()
                    .get("last-modified")
                    .and_then(|value| value.to_str().ok());
                if !resume_context
                    .validator
                    .permits_response(response_etag, response_last_modified)
                {
                    return Err(AcquireError::Protocol {
                        url: source.url.as_url().clone(),
                        kind: ProtocolError::ResumeValidatorMismatch,
                        attempts: attempts.clone(),
                        resume: resume.clone(),
                    });
                }
                let content_range = response
                    .headers()
                    .get("content-range")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                let parsed_range = content_range
                    .as_deref()
                    .and_then(|value| parse_content_range(value, resume_context.partial_bytes))
                    .ok_or_else(|| AcquireError::Protocol {
                        url: source.url.as_url().clone(),
                        kind: ProtocolError::InvalidContentRange {
                            expected_start: resume_context.partial_bytes,
                            header: content_range.clone(),
                        },
                        attempts: attempts.clone(),
                        resume: resume.clone(),
                    })?;
                Some((resume_context.clone(), parsed_range, content_range))
            } else {
                if let Some(resume_context) = resume_context {
                    resume =
                        Some(resume_context.into_evidence(ResumeOutcome::RangeIgnoredRestarted));
                }
                None
            };

            let mut temp = tempfile::NamedTempFile::new().map_err(|err| {
                AcquireError::local(
                    Some(&source.url),
                    "create download temp file",
                    std::env::temp_dir(),
                    err,
                )
            })?;
            let initial_bytes = if let Some((resume_context, _, _)) = &append_resume {
                let mut partial_file =
                    std::fs::File::open(&resume_context.partial_path).map_err(|err| {
                        AcquireError::local(
                            Some(&source.url),
                            "open partial download",
                            &resume_context.partial_path,
                            err,
                        )
                    })?;
                std::io::copy(&mut partial_file, temp.as_file_mut()).map_err(|err| {
                    AcquireError::local(
                        Some(&source.url),
                        "copy partial download",
                        temp.path(),
                        err,
                    )
                })?
            } else {
                0
            };
            let active_pacer = self.resources.byte_pacer.as_ref();
            let copy = match copy_response_body(
                response.body_mut().as_reader(),
                temp.as_file_mut(),
                source.policy.max_bytes,
                initial_bytes,
                active_pacer,
            ) {
                Ok(copy) => copy,
                Err(BodyCopyError::Transport { message, bytes }) => {
                    let will_retry = attempt < source.policy.retry.max_retries;
                    let planned_delay =
                        will_retry.then(|| retry_delay(source.policy.retry, attempt));
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
                        (self.resources.delay)(delay);
                        continue;
                    }
                    return Err(AcquireError::transport(
                        &source.url,
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
                    return Err(AcquireError::LimitExceeded {
                        url: source.url.as_url().clone(),
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
                    return Err(AcquireError::Local {
                        url: Some(source.url.as_url().clone()),
                        action,
                        path,
                        source: io_error,
                    });
                }
            };
            if let Some((resume_context, parsed_range, content_range)) = &append_resume
                && !parsed_range
                    .matches_materialized_bytes(resume_context.partial_bytes, copy.bytes)
            {
                return Err(AcquireError::Protocol {
                    url: source.url.as_url().clone(),
                    kind: ProtocolError::InvalidContentRange {
                        expected_start: resume_context.partial_bytes,
                        header: content_range.clone(),
                    },
                    attempts,
                    resume,
                });
            }
            temp.as_file_mut().flush().map_err(|err| {
                AcquireError::local(
                    Some(&source.url),
                    "flush download temp file",
                    temp.path(),
                    err,
                )
            })?;

            let staged_path = temp.into_temp_path();
            if let Some((resume_context, _, _)) = append_resume {
                resume = Some(resume_context.into_evidence(ResumeOutcome::PartialAppended));
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
            return Ok(Acquired {
                input: node,
                material: LocalMaterial::StagedFile { path: staged_path },
                evidence: HttpAcquireEvidence {
                    url: source.url.into_url(),
                    status,
                    bytes: copy.bytes,
                    content_length,
                    attempts,
                    resume,
                    validator: response_validator,
                },
            });
        }
        unreachable!("retry loop always returns")
    }
}

#[cfg(feature = "http-async")]
#[derive(Clone)]
pub struct AsyncHttpResources {
    client: reqwest::Client,
    delay: AsyncSleep,
    admission: Option<Arc<RateAdmission>>,
    byte_pacer: Option<Arc<ByteRatePacer>>,
}

#[cfg(feature = "http-async")]
impl Default for AsyncHttpResources {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
            delay: Arc::new(|duration| Box::pin(tokio::time::sleep(duration))),
            admission: None,
            byte_pacer: None,
        }
    }
}

#[cfg(feature = "http-async")]
impl AsyncHttpResources {
    pub fn from_client(client: reqwest::Client) -> Self {
        Self {
            client,
            ..Self::default()
        }
    }

    #[cfg(test)]
    fn with_delay(mut self, delay: AsyncSleep) -> Self {
        self.delay = delay;
        self
    }

    pub fn with_admission(mut self, admission: Arc<RateAdmission>) -> Self {
        self.admission = Some(admission);
        self
    }

    pub fn with_byte_pacer(mut self, byte_pacer: Arc<ByteRatePacer>) -> Self {
        self.byte_pacer = Some(byte_pacer);
        self
    }
}

#[cfg(feature = "http-async")]
impl std::fmt::Debug for AsyncHttpResources {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AsyncHttpResources")
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "http-async")]
/// Tokio-backed asynchronous HEAD inspection implemented by `reqwest`.
#[derive(Clone, Debug)]
pub struct AsyncHttpInspect {
    resources: AsyncHttpResources,
    policy: HttpInspectPolicy,
}

#[cfg(feature = "http-async")]
impl Default for AsyncHttpInspect {
    fn default() -> Self {
        Self::new(AsyncHttpResources::default(), HttpInspectPolicy::default())
    }
}

#[cfg(feature = "http-async")]
impl AsyncHttpInspect {
    pub fn new(resources: AsyncHttpResources, policy: HttpInspectPolicy) -> Self {
        Self { resources, policy }
    }
}

#[cfg(feature = "http-async")]
impl AsyncInspect<RemoteUrl> for AsyncHttpInspect {
    type Error = HttpInspectError;
    type Output = Inspected<RemoteUrl, HttpObservation, HttpInspectEvidence>;

    fn inspect<'a>(
        &'a self,
        node: RemoteUrl,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + 'a
    where
        RemoteUrl: 'a,
    {
        inspect_reqwest(
            self.resources.client.clone(),
            self.resources.delay.clone(),
            self.resources.admission.clone(),
            self.policy.clone(),
            node,
        )
    }
}

#[cfg(feature = "http-async")]
#[allow(
    clippy::result_large_err,
    reason = "HttpInspectError keeps URL and attempt evidence direct; this async boundary does not establish future-layout pressure"
)]
async fn inspect_reqwest(
    client: reqwest::Client,
    delay: AsyncSleep,
    admission: Option<Arc<RateAdmission>>,
    policy: HttpInspectPolicy,
    node: RemoteUrl,
) -> Result<Inspected<RemoteUrl, HttpObservation, HttpInspectEvidence>, HttpInspectError> {
    let requested_url = node.as_url().clone();
    let mut attempts = Vec::new();
    for attempt in 0..=policy.retry.max_retries {
        let admission_wait = match admission.as_ref() {
            Some(admission) => Some(admission.enter_async().await),
            None => None,
        };

        let mut request = client.head(node.as_str());
        if let Some(timeout) = policy.timeout {
            request = request.timeout(timeout);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                let will_retry = attempt < policy.retry.max_retries;
                let planned_delay = will_retry.then(|| retry_delay(policy.retry, attempt));
                attempts.push(
                    HttpInspectAttemptEvidence::new(attempt, None, admission_wait)
                        .with_planned_delay(planned_delay),
                );
                if let Some(wait) = planned_delay {
                    (delay)(wait).await;
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
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        let final_url = response.url().clone();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| parse_retry_after(value, SystemTime::now()));
        let will_retry = should_retry_status(status) && attempt < policy.retry.max_retries;
        let planned_delay =
            will_retry.then(|| planned_retry_delay(policy.retry, attempt, retry_after));
        attempts.push(
            HttpInspectAttemptEvidence::new(attempt, Some(status), admission_wait)
                .with_planned_delay(planned_delay),
        );
        if let Some(wait) = planned_delay {
            (delay)(wait).await;
            continue;
        }

        return Ok(Inspected {
            input: node,
            observation: HttpObservation {
                status,
                declared_content_length,
            },
            evidence: HttpInspectEvidence {
                requested_url,
                final_url,
                attempts,
            },
        });
    }
    unreachable!("HTTP inspection retry loop always returns")
}

#[cfg(feature = "http-async")]
#[derive(Clone, Debug, Default)]
pub struct AsyncHttpAcquire {
    resources: AsyncHttpResources,
}

#[cfg(feature = "http-async")]
impl AsyncHttpAcquire {
    pub fn new(resources: AsyncHttpResources) -> Self {
        Self { resources }
    }
}

#[cfg(feature = "http-async")]
impl<I, T> AsyncAcquire<Materialize<I, RemoteSource, T>> for AsyncHttpAcquire {
    type Error = AcquireError;
    type Output = Acquired<Materialize<I, RemoteSource, T>, LocalMaterial, HttpAcquireEvidence>;

    fn acquire<'a>(
        &'a self,
        node: Materialize<I, RemoteSource, T>,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + 'a
    where
        Materialize<I, RemoteSource, T>: 'a,
    {
        let client = self.resources.client.clone();
        let delay = self.resources.delay.clone();
        let admission = self.resources.admission.clone();
        let byte_pacer = self.resources.byte_pacer.clone();
        async move { acquire_reqwest(client, delay, admission, byte_pacer, node).await }
    }
}

#[cfg(feature = "http-async")]
#[allow(
    clippy::result_large_err,
    reason = "AcquireError keeps URL, retry, and resume evidence direct; this async boundary does not establish future-layout pressure"
)]
async fn acquire_reqwest<I, T>(
    client: reqwest::Client,
    delay: AsyncSleep,
    admission: Option<Arc<RateAdmission>>,
    byte_pacer: Option<Arc<ByteRatePacer>>,
    node: Materialize<I, RemoteSource, T>,
) -> Result<
    Acquired<Materialize<I, RemoteSource, T>, LocalMaterial, HttpAcquireEvidence>,
    AcquireError,
> {
    let source = node.source.clone();

    let mut attempts = Vec::new();
    let mut resume = None;
    let mut resume_suppressed = false;
    let max_attempts = source
        .policy
        .retry
        .max_retries
        .saturating_add(u32::from(planned_resume(&source.policy.resume).is_some()));
    'attempts: for attempt in 0..=max_attempts {
        let resume_context = (!resume_suppressed)
            .then(|| planned_resume(&source.policy.resume))
            .flatten();
        let admission_wait = match admission.as_ref() {
            Some(admission) => Some(admission.enter_async().await),
            None => None,
        };
        let mut request = client.get(source.url.as_str());
        for (name, value) in &source.policy.headers {
            request = request.header(name, value);
        }
        if let Some(resume) = &resume_context {
            request = request.header(
                reqwest::header::RANGE,
                format!("bytes={}-", resume.partial_bytes),
            );
            request = request.header(reqwest::header::IF_RANGE, resume.validator.if_range_value());
        }
        if let Some(timeout) = source.policy.timeout {
            request = request.timeout(timeout);
        }

        let mut response = match request.send().await {
            Ok(response) => response,
            Err(err) => {
                let will_retry = attempt < source.policy.retry.max_retries;
                let planned_delay = will_retry.then(|| retry_delay(source.policy.retry, attempt));
                attempts.push(
                    AttemptEvidence::new(attempt, AttemptOutcome::RetryableNetworkError)
                        .with_planned_delay(planned_delay)
                        .with_admission_wait(admission_wait),
                );
                if let Some(wait) = planned_delay {
                    (delay)(wait).await;
                    continue 'attempts;
                }
                return Err(AcquireError::transport(
                    &source.url,
                    TransportPhase::SendRequest,
                    err.to_string(),
                    attempts,
                    resume,
                ));
            }
        };
        let status = response.status().as_u16();
        let content_length = response.content_length();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| parse_retry_after(value, SystemTime::now()));
        let response_validator = selected_response_validator(
            response
                .headers()
                .get(reqwest::header::ETAG)
                .and_then(|value| value.to_str().ok()),
            response
                .headers()
                .get(reqwest::header::LAST_MODIFIED)
                .and_then(|value| value.to_str().ok()),
            response
                .headers()
                .get(reqwest::header::DATE)
                .and_then(|value| value.to_str().ok()),
        );
        if status == 416
            && let Some(resume_context) = resume_context
        {
            attempts.push(AttemptEvidence::response(
                attempt,
                status,
                content_length,
                admission_wait,
                AttemptOutcome::NonRetryableStatus,
            ));
            resume = Some(resume_context.into_evidence(ResumeOutcome::RangeUnsatisfiableRestarted));
            resume_suppressed = true;
            continue 'attempts;
        }
        if !response.status().is_success() {
            let retryable = should_retry_status(status);
            let will_retry = retryable && attempt < source.policy.retry.max_retries;
            let planned_delay =
                will_retry.then(|| planned_retry_delay(source.policy.retry, attempt, retry_after));
            attempts.push(
                AttemptEvidence::response(
                    attempt,
                    status,
                    content_length,
                    admission_wait,
                    if retryable {
                        AttemptOutcome::RetryableStatus
                    } else {
                        AttemptOutcome::NonRetryableStatus
                    },
                )
                .with_retry_after(retry_after)
                .with_planned_delay(planned_delay),
            );
            if let Some(wait) = planned_delay {
                (delay)(wait).await;
                continue;
            }
            return Err(AcquireError::HttpStatus {
                url: source.url.as_url().clone(),
                status,
                retryable,
                attempts,
                resume,
            });
        }
        if let Err((max, actual)) = reject_known_oversize(content_length, source.policy.max_bytes) {
            attempts.push(AttemptEvidence::response(
                attempt,
                status,
                content_length,
                admission_wait,
                AttemptOutcome::LimitExceeded,
            ));
            return Err(AcquireError::LimitExceeded {
                url: source.url.as_url().clone(),
                max,
                actual,
                attempts,
                resume,
            });
        }

        let append_resume = if status == 206 {
            let resume_context = resume_context
                .as_ref()
                .ok_or_else(|| AcquireError::Protocol {
                    url: source.url.as_url().clone(),
                    kind: ProtocolError::UnexpectedPartialResponse,
                    attempts: attempts.clone(),
                    resume: resume.clone(),
                })?;
            let response_etag = response
                .headers()
                .get(reqwest::header::ETAG)
                .and_then(|value| value.to_str().ok());
            let response_last_modified = response
                .headers()
                .get(reqwest::header::LAST_MODIFIED)
                .and_then(|value| value.to_str().ok());
            if !resume_context
                .validator
                .permits_response(response_etag, response_last_modified)
            {
                return Err(AcquireError::Protocol {
                    url: source.url.as_url().clone(),
                    kind: ProtocolError::ResumeValidatorMismatch,
                    attempts: attempts.clone(),
                    resume: resume.clone(),
                });
            }
            let content_range = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let parsed_range = content_range
                .as_deref()
                .and_then(|value| parse_content_range(value, resume_context.partial_bytes))
                .ok_or_else(|| AcquireError::Protocol {
                    url: source.url.as_url().clone(),
                    kind: ProtocolError::InvalidContentRange {
                        expected_start: resume_context.partial_bytes,
                        header: content_range.clone(),
                    },
                    attempts: attempts.clone(),
                    resume: resume.clone(),
                })?;
            Some((resume_context.clone(), parsed_range, content_range))
        } else {
            if let Some(resume_context) = resume_context {
                resume = Some(resume_context.into_evidence(ResumeOutcome::RangeIgnoredRestarted));
            }
            None
        };

        let mut stage = if let Some((resume_context, _, _)) = &append_resume {
            StagedDownload::<Open>::from_partial(&resume_context.partial_path).await?
        } else {
            StagedDownload::<Open>::new()?
        };
        let active_pacer = byte_pacer.as_ref();
        while let Some(chunk) = match response.chunk().await {
            Ok(chunk) => chunk,
            Err(err) => {
                let will_retry = attempt < source.policy.retry.max_retries;
                let planned_delay = will_retry.then(|| retry_delay(source.policy.retry, attempt));
                attempts.push(
                    AttemptEvidence::transfer(
                        attempt,
                        status,
                        stage.bytes,
                        content_length,
                        admission_wait,
                        AttemptOutcome::RetryableNetworkError,
                    )
                    .with_planned_delay(planned_delay),
                );
                if let Some(wait) = planned_delay {
                    (delay)(wait).await;
                    continue 'attempts;
                }
                return Err(AcquireError::transport(
                    &source.url,
                    TransportPhase::ReadBody,
                    err.to_string(),
                    attempts,
                    resume,
                ));
            }
        } {
            match stage
                .write_chunk(&chunk, source.policy.max_bytes, active_pacer)
                .await
            {
                Ok(()) => {}
                Err(StageWriteError::LimitExceeded { max, actual }) => {
                    attempts.push(AttemptEvidence::transfer(
                        attempt,
                        status,
                        stage.bytes,
                        content_length,
                        admission_wait,
                        AttemptOutcome::LimitExceeded,
                    ));
                    return Err(AcquireError::LimitExceeded {
                        url: source.url.as_url().clone(),
                        max,
                        actual,
                        attempts,
                        resume,
                    });
                }

                Err(StageWriteError::Local {
                    action,
                    path,
                    source: io_error,
                }) => {
                    attempts.push(AttemptEvidence::transfer(
                        attempt,
                        status,
                        stage.bytes,
                        content_length,
                        admission_wait,
                        AttemptOutcome::LocalFailure,
                    ));
                    return Err(AcquireError::Local {
                        url: Some(source.url.as_url().clone()),
                        action,
                        path,
                        source: io_error,
                    });
                }
            }
        }
        let bytes = stage.bytes;
        if let Some((resume_context, parsed_range, content_range)) = &append_resume
            && !parsed_range.matches_materialized_bytes(resume_context.partial_bytes, bytes)
        {
            return Err(AcquireError::Protocol {
                url: source.url.as_url().clone(),
                kind: ProtocolError::InvalidContentRange {
                    expected_start: resume_context.partial_bytes,
                    header: content_range.clone(),
                },
                attempts,
                resume,
            });
        }
        let pacing_wait = stage.pacing_wait;
        let stage = stage.finish().await?;
        let staged_path = stage.into_temp_path();
        if let Some((resume_context, _, _)) = append_resume {
            resume = Some(resume_context.into_evidence(ResumeOutcome::PartialAppended));
        }

        attempts.push(
            AttemptEvidence::transfer(
                attempt,
                status,
                bytes,
                content_length,
                admission_wait,
                AttemptOutcome::Success,
            )
            .with_pacing_wait(pacing_wait),
        );
        return Ok(Acquired {
            input: node,
            material: LocalMaterial::StagedFile { path: staged_path },
            evidence: HttpAcquireEvidence {
                url: source.url.into_url(),
                status,
                bytes,
                content_length,
                attempts,
                resume,
                validator: response_validator,
            },
        });
    }
    unreachable!("retry loop always returns")
}

#[cfg(any(feature = "http-async", feature = "http-sync"))]
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

#[cfg(any(test, feature = "http-sync", feature = "http-async"))]
fn retry_delay(policy: RetryPolicy, retry_index: u32) -> Duration {
    let delay = policy
        .base_delay
        .saturating_mul(2_u32.saturating_pow(retry_index));
    policy.max_delay.map_or(delay, |max| delay.min(max))
}

#[cfg(any(feature = "http-sync", feature = "http-async"))]
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

#[cfg(any(test, feature = "http-sync", feature = "http-async"))]
fn should_retry_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

#[cfg(any(test, feature = "http-sync", feature = "http-async"))]
fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()
        .and_then(|retry_at| retry_at.duration_since(now).ok())
}

#[cfg(any(feature = "http-sync", feature = "http-async"))]
#[derive(Clone, Debug, Eq, PartialEq)]
struct PlannedResume {
    partial_path: PathBuf,
    partial_bytes: u64,
    validator: Validator,
}

#[cfg(any(feature = "http-sync", feature = "http-async"))]
impl PlannedResume {
    fn into_evidence(self, outcome: ResumeOutcome) -> ResumeEvidence {
        ResumeEvidence {
            outcome,
            partial_path: self.partial_path,
            partial_bytes: self.partial_bytes,
            validator: self.validator,
        }
    }
}

#[cfg(any(feature = "http-sync", feature = "http-async"))]
fn planned_resume(policy: &ResumePolicy) -> Option<PlannedResume> {
    let (partial_path, validator) = match policy {
        ResumePolicy::RestartOnly => return None,
        ResumePolicy::IfRange {
            partial_path,
            validator,
        } => (partial_path.clone(), validator.clone()),
    };
    let partial_bytes = std::fs::metadata(&partial_path).ok()?.len();
    (partial_bytes > 0).then_some(PlannedResume {
        partial_path,
        partial_bytes,
        validator,
    })
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

#[cfg(any(test, feature = "http-sync", feature = "http-async"))]
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

#[cfg(any(test, feature = "http-sync", feature = "http-async"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParsedContentRange {
    end: u64,
    total: u64,
}

#[cfg(any(test, feature = "http-sync", feature = "http-async"))]
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

#[cfg(any(test, feature = "http-sync", feature = "http-async"))]
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

#[cfg(feature = "http-sync")]
struct BodyCopyProgress {
    bytes: u64,
    pacing_wait: Duration,
}

#[cfg(feature = "http-sync")]
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

#[cfg(feature = "http-sync")]
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

#[cfg(feature = "http-async")]
enum StageWriteError {
    LimitExceeded {
        max: u64,
        actual: u64,
    },
    Local {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

#[cfg(feature = "http-async")]
struct Open {
    file: tokio::fs::File,
}

#[cfg(feature = "http-async")]
struct Closed;

#[cfg(feature = "http-async")]
struct StagedDownload<State> {
    temp: tempfile::NamedTempFile,
    bytes: u64,
    pacing_wait: Duration,
    writer: State,
}

#[cfg(feature = "http-async")]
impl StagedDownload<Open> {
    #[allow(
        clippy::result_large_err,
        reason = "AcquireError intentionally carries complete retry and resume evidence"
    )]
    fn new() -> Result<Self, AcquireError> {
        let temp = tempfile::NamedTempFile::new().map_err(|err| {
            AcquireError::local(None, "create download temp file", std::env::temp_dir(), err)
        })?;
        let file = tokio::fs::File::from_std(temp.reopen().map_err(|err| {
            AcquireError::local(None, "reopen download temp file", temp.path(), err)
        })?);
        Ok(Self {
            temp,
            bytes: 0,
            pacing_wait: Duration::ZERO,
            writer: Open { file },
        })
    }

    #[allow(
        clippy::result_large_err,
        reason = "AcquireError keeps complete local staging context direct; this async boundary does not establish future-layout pressure"
    )]
    async fn from_partial(partial_path: &Path) -> Result<Self, AcquireError> {
        let temp = tempfile::NamedTempFile::new().map_err(|err| {
            AcquireError::local(None, "create download temp file", std::env::temp_dir(), err)
        })?;
        let bytes = std::fs::copy(partial_path, temp.path())
            .map_err(|err| AcquireError::local(None, "copy partial download", temp.path(), err))?;
        let file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(temp.path())
            .await
            .map_err(|err| {
                AcquireError::local(None, "open partial download temp", temp.path(), err)
            })?;
        Ok(Self {
            temp,
            bytes,
            pacing_wait: Duration::ZERO,
            writer: Open { file },
        })
    }

    async fn write_chunk(
        &mut self,
        chunk: &[u8],
        max_bytes: Option<u64>,
        pacer: Option<&Arc<ByteRatePacer>>,
    ) -> Result<(), StageWriteError> {
        let actual = self.bytes.saturating_add(chunk.len() as u64);
        if let Some(max) = max_bytes
            && actual > max
        {
            return Err(StageWriteError::LimitExceeded { max, actual });
        }
        if let Some(pacer) = pacer {
            self.pacing_wait += pacer.before_chunk_async(chunk.len() as u64).await;
        }
        use tokio::io::AsyncWriteExt;
        self.writer
            .file
            .write_all(chunk)
            .await
            .map_err(|err| StageWriteError::Local {
                action: "write download temp file",
                path: self.temp.path().to_path_buf(),
                source: err,
            })?;
        self.bytes = actual;
        Ok(())
    }

    #[allow(
        clippy::result_large_err,
        reason = "AcquireError keeps complete local staging context direct; this async boundary does not establish future-layout pressure"
    )]
    async fn finish(mut self) -> Result<StagedDownload<Closed>, AcquireError> {
        use tokio::io::AsyncWriteExt;
        self.writer.file.flush().await.map_err(|err| {
            AcquireError::local(None, "flush download temp file", self.temp.path(), err)
        })?;
        drop(self.writer);
        Ok(StagedDownload {
            temp: self.temp,
            bytes: self.bytes,
            pacing_wait: self.pacing_wait,
            writer: Closed,
        })
    }
}

#[cfg(feature = "http-async")]
impl StagedDownload<Closed> {
    fn into_temp_path(self) -> tempfile::TempPath {
        self.temp.into_temp_path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(all(
        any(feature = "http-async", feature = "http-sync"),
        feature = "hash",
        feature = "blake3"
    ))]
    use crate::hash::{ArtifactDescriptor, Blake3, DigestValue, HashVerify};
    #[cfg(all(
        any(feature = "http-async", feature = "http-sync"),
        feature = "hash",
        feature = "blake3"
    ))]
    use crate::local::LocalApply;
    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    use crate::local::LocalTarget;
    #[cfg(feature = "http-sync")]
    use crate::{Acquire, Inspect};
    #[cfg(all(
        any(feature = "http-async", feature = "http-sync"),
        feature = "hash",
        feature = "blake3"
    ))]
    use crate::{Apply, Verify};
    #[cfg(feature = "http-async")]
    use crate::{AsyncAcquire, AsyncInspect};
    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    use crate::{Materialize, MaterializeMode};
    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    use std::io::Write as TestWrite;
    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    use std::io::{BufRead, BufReader};
    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    use std::net::{TcpListener, TcpStream};
    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    use std::sync::{Arc as TestArc, Mutex, mpsc};
    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    use std::thread;

    fn test_validator() -> Validator {
        Validator::strong_etag("\"abc\"").unwrap()
    }

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    fn materialize(
        source: RemoteSource,
        target: impl Into<PathBuf>,
    ) -> Materialize<&'static str, RemoteSource, LocalTarget> {
        Materialize::new(
            "artifact",
            source,
            LocalTarget::new(target),
            MaterializeMode::ReplaceOrCreate,
        )
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
    #[cfg(feature = "http-sync")]
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
    #[cfg(feature = "http-async")]
    fn async_rate_admission_shares_attempt_budget() {
        block_on_reqwest(async {
            let admission = RateAdmission::new(AttemptRate::new(
                NonZeroU32::new(20).unwrap(),
                NonZeroU32::new(1).unwrap(),
            ));

            let first = admission.enter_async().await;
            let second = admission.enter_async().await;

            assert_eq!(first, Duration::ZERO);
            assert!(second > Duration::ZERO);
        });
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn sync_byte_rate_pacer_zero_bytes_is_immediate() {
        let pacer = ByteRatePacer::new(ByteRate::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ));

        assert_eq!(pacer.before_chunk_sync(0), Duration::ZERO);
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn sync_byte_rate_pacer_splits_chunks_larger_than_burst() {
        let pacer = ByteRatePacer::new(ByteRate::new(
            NonZeroU32::new(1_000).unwrap(),
            NonZeroU32::new(2).unwrap(),
        ));

        let waited = pacer.before_chunk_sync(3);

        assert!(waited > Duration::ZERO);
    }

    #[test]
    #[cfg(feature = "http-sync")]
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
    #[cfg(feature = "http-async")]
    fn async_byte_rate_pacer_zero_bytes_is_immediate() {
        block_on_reqwest(async {
            let pacer = ByteRatePacer::new(ByteRate::new(
                NonZeroU32::new(1).unwrap(),
                NonZeroU32::new(1).unwrap(),
            ));

            assert_eq!(pacer.before_chunk_async(0).await, Duration::ZERO);
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn async_byte_rate_pacer_splits_chunks_larger_than_burst() {
        block_on_reqwest(async {
            let pacer = ByteRatePacer::new(ByteRate::new(
                NonZeroU32::new(10).unwrap(),
                NonZeroU32::new(2).unwrap(),
            ));

            let waited = pacer.before_chunk_async(3).await;

            assert!(waited > Duration::ZERO);
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn async_byte_rate_pacer_shares_budget_across_calls() {
        block_on_reqwest(async {
            let pacer = ByteRatePacer::new(ByteRate::new(
                NonZeroU32::new(10).unwrap(),
                NonZeroU32::new(1).unwrap(),
            ));

            let first = pacer.before_chunk_async(1).await;
            let second = pacer.before_chunk_async(1).await;

            assert_eq!(first, Duration::ZERO);
            assert!(second > Duration::ZERO);
        });
    }

    #[test]
    fn retry_policy_is_disabled_by_default_and_computes_delay() {
        assert_eq!(AcquirePolicy::default().retry, RetryPolicy::disabled());
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
    #[cfg(feature = "http-sync")]
    fn ureq_inspect_uses_head_and_reports_declared_metadata() {
        let server = serve_once(200, b"body-not-materialized", &[]);
        let inspected = SyncHttpInspect::default()
            .inspect(RemoteUrl::parse(&server.url).unwrap())
            .unwrap();
        let request = server.next_request();
        server.join();

        assert!(request.starts_with("HEAD /artifact.bin HTTP/1.1\r\n"));
        assert_eq!(inspected.observation.status, 200);
        assert_eq!(
            inspected.observation.declared_content_length,
            Some(b"body-not-materialized".len() as u64)
        );
        assert_eq!(inspected.evidence.attempts.len(), 1);
        assert_eq!(
            inspected.evidence.requested_url,
            inspected.evidence.final_url
        );
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_inspect_returns_non_success_status_as_observation_without_get_fallback() {
        let server = serve_once(405, b"method not allowed", &[]);
        let inspected = SyncHttpInspect::default()
            .inspect(RemoteUrl::parse(&server.url).unwrap())
            .unwrap();
        let request = server.next_request();
        server.join();

        assert!(request.starts_with("HEAD "));
        assert_eq!(inspected.observation.status, 405);
        assert_eq!(inspected.evidence.attempts.len(), 1);
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_inspect_retries_status_and_returns_final_observation() {
        let server = serve_sequence(vec![(503, b"retry", &[]), (404, b"missing", &[])]);
        let resources =
            SyncHttpResources::default().with_admission(TestArc::new(RateAdmission::new(
                AttemptRate::new(NonZeroU32::new(1_000).unwrap(), NonZeroU32::new(2).unwrap()),
            )));
        let policy = HttpInspectPolicy::default().retry(RetryPolicy {
            max_retries: 1,
            base_delay: Duration::ZERO,
            max_delay: None,
            respect_retry_after: false,
        });
        let inspected = SyncHttpInspect::new(resources, policy)
            .inspect(RemoteUrl::parse(&server.url).unwrap())
            .unwrap();
        assert!(server.next_request().starts_with("HEAD "));
        assert!(server.next_request().starts_with("HEAD "));
        server.join();

        assert_eq!(inspected.observation.status, 404);
        assert_eq!(inspected.evidence.attempts.len(), 2);
        assert_eq!(inspected.evidence.attempts[0].status, Some(503));
        assert_eq!(inspected.evidence.attempts[1].status, Some(404));
        assert_eq!(
            inspected.evidence.attempts[0].admission_wait,
            Some(Duration::ZERO)
        );
        assert_eq!(
            inspected.evidence.attempts[1].admission_wait,
            Some(Duration::ZERO)
        );
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_inspect_records_requested_and_final_redirect_urls() {
        let server = serve_redirect_then(200, b"final");
        let requested = RemoteUrl::parse(&server.url).unwrap();
        let inspected = SyncHttpInspect::default()
            .inspect(requested.clone())
            .unwrap();
        assert!(server.next_request().starts_with("HEAD /artifact.bin "));
        assert!(server.next_request().starts_with("HEAD /final "));
        server.join();

        assert_eq!(inspected.evidence.requested_url, requested.into_url());
        assert!(inspected.evidence.final_url.as_str().ends_with("/final"));
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_inspect_preserves_transport_attempt_evidence() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);

        let error = SyncHttpInspect::default()
            .inspect(RemoteUrl::parse(&format!("http://{address}/resource")).unwrap())
            .unwrap_err();

        match error {
            HttpInspectError::Transport { attempts, .. } => {
                assert_eq!(attempts.len(), 1);
                assert_eq!(attempts[0].status, None);
            }
            other => panic!("expected transport error, got {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_inspect_uses_head_and_reports_declared_metadata() {
        let server = serve_once(200, b"body-not-materialized", &[]);
        let inspected = block_on_reqwest(
            AsyncHttpInspect::default().inspect(RemoteUrl::parse(&server.url).unwrap()),
        )
        .unwrap();
        let request = server.next_request();
        server.join();

        assert!(request.starts_with("HEAD /artifact.bin HTTP/1.1\r\n"));
        assert_eq!(inspected.observation.status, 200);
        assert_eq!(
            inspected.observation.declared_content_length,
            Some(b"body-not-materialized".len() as u64)
        );
        assert_eq!(inspected.evidence.attempts.len(), 1);
        assert_eq!(
            inspected.evidence.requested_url,
            inspected.evidence.final_url
        );
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_inspect_returns_non_success_status_as_observation_without_get_fallback() {
        let server = serve_once(405, b"method not allowed", &[]);
        let inspected = block_on_reqwest(
            AsyncHttpInspect::default().inspect(RemoteUrl::parse(&server.url).unwrap()),
        )
        .unwrap();
        let request = server.next_request();
        server.join();

        assert!(request.starts_with("HEAD "));
        assert_eq!(inspected.observation.status, 405);
        assert_eq!(inspected.evidence.attempts.len(), 1);
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_inspect_retries_status_and_returns_final_observation() {
        let server = serve_sequence(vec![(503, b"retry", &[]), (404, b"missing", &[])]);
        let resources =
            AsyncHttpResources::default().with_admission(TestArc::new(RateAdmission::new(
                AttemptRate::new(NonZeroU32::new(1_000).unwrap(), NonZeroU32::new(2).unwrap()),
            )));
        let policy = HttpInspectPolicy::default().retry(RetryPolicy {
            max_retries: 1,
            base_delay: Duration::ZERO,
            max_delay: None,
            respect_retry_after: false,
        });
        let inspected = block_on_reqwest(
            AsyncHttpInspect::new(resources, policy)
                .inspect(RemoteUrl::parse(&server.url).unwrap()),
        )
        .unwrap();
        assert!(server.next_request().starts_with("HEAD "));
        assert!(server.next_request().starts_with("HEAD "));
        server.join();

        assert_eq!(inspected.observation.status, 404);
        assert_eq!(inspected.evidence.attempts.len(), 2);
        assert_eq!(inspected.evidence.attempts[0].status, Some(503));
        assert_eq!(inspected.evidence.attempts[1].status, Some(404));
        assert_eq!(
            inspected.evidence.attempts[0].admission_wait,
            Some(Duration::ZERO)
        );
        assert_eq!(
            inspected.evidence.attempts[1].admission_wait,
            Some(Duration::ZERO)
        );
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_inspect_records_requested_and_final_redirect_urls() {
        let server = serve_redirect_then(200, b"final");
        let requested = RemoteUrl::parse(&server.url).unwrap();
        let inspected =
            block_on_reqwest(AsyncHttpInspect::default().inspect(requested.clone())).unwrap();
        assert!(server.next_request().starts_with("HEAD /artifact.bin "));
        assert!(server.next_request().starts_with("HEAD /final "));
        server.join();

        assert_eq!(inspected.evidence.requested_url, requested.into_url());
        assert!(inspected.evidence.final_url.as_str().ends_with("/final"));
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_inspect_preserves_transport_attempt_evidence() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);

        let error = block_on_reqwest(
            AsyncHttpInspect::default()
                .inspect(RemoteUrl::parse(&format!("http://{address}/resource")).unwrap()),
        )
        .unwrap_err();

        match error {
            HttpInspectError::Transport { attempts, .. } => {
                assert_eq!(attempts.len(), 1);
                assert_eq!(attempts[0].status, None);
            }
            other => panic!("expected transport error, got {other:?}"),
        }
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_acquire_downloads_file_to_local_material() {
        let body = b"downloaded bytes";
        let server = serve_once(200, body, &[]);
        let temp = tempfile::tempdir().unwrap();
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());
        let chosen = materialize(source, temp.path().join("out"));

        let acquired = SyncHttpAcquire::default().acquire(chosen).unwrap();
        server.join();

        assert!(matches!(
            &acquired.material,
            LocalMaterial::StagedFile { .. }
        ));
        assert_eq!(std::fs::read(acquired.material.path()).unwrap(), body);
        assert_eq!(acquired.evidence.status, 200);
        assert_eq!(acquired.evidence.bytes, body.len() as u64);
        assert_eq!(acquired.evidence.content_length, Some(body.len() as u64));
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_acquire_does_not_publish_materialize_target() {
        let server = serve_once(200, b"replacement", &[]);
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.bin");
        std::fs::write(&target, b"original").unwrap();
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

        let acquired = SyncHttpAcquire::default()
            .acquire(materialize(source, &target))
            .unwrap();
        server.join();

        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        drop(acquired);
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_acquire_does_not_create_target_parent() {
        let server = serve_once(200, b"staged", &[]);
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("absent/target.bin");
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

        let acquired = SyncHttpAcquire::default()
            .acquire(materialize(source, &target))
            .unwrap();
        server.join();

        assert!(!target.parent().unwrap().exists());
        assert_eq!(std::fs::read(acquired.material.path()).unwrap(), b"staged");
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_acquired_material_is_removed_when_abandoned() {
        let server = serve_once(200, b"temporary", &[]);
        let temp = tempfile::tempdir().unwrap();
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

        let acquired = SyncHttpAcquire::default()
            .acquire(materialize(source, temp.path().join("target.bin")))
            .unwrap();
        server.join();
        let staged_path = acquired.material.path().to_path_buf();
        assert!(staged_path.exists());
        drop(acquired);

        assert!(!staged_path.exists());
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_acquire_rejects_non_success_status_without_touching_target() {
        let server = serve_once(404, b"not found", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        std::fs::write(&destination, b"old").unwrap();
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());
        let chosen = materialize(source, &destination);

        let error = SyncHttpAcquire::default().acquire(chosen).unwrap_err();
        server.join();

        assert!(matches!(
            error,
            AcquireError::HttpStatus {
                status: 404,
                retryable: false,
                ..
            }
        ));
        assert_eq!(std::fs::read(&destination).unwrap(), b"old");
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_acquire_enforces_max_bytes_without_publishing_target() {
        let server = serve_once(200, b"too large", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let policy = AcquirePolicy::default().max_bytes(3);
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
        let chosen = materialize(source, &destination);

        let error = SyncHttpAcquire::default().acquire(chosen).unwrap_err();
        server.join();

        assert!(matches!(error, AcquireError::LimitExceeded { max: 3, .. }));
        assert!(!destination.exists());
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_resume_limit_evidence_records_materialized_partial_bytes() {
        let server = serve_once(206, b" world", &[("Content-Range", "bytes 5-10/11")]);
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let policy = AcquirePolicy::default()
            .max_bytes(8)
            .resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

        let error = SyncHttpAcquire::default()
            .acquire(materialize(source, temp.path().join("out")))
            .unwrap_err();
        server.join();

        let attempts = match error {
            AcquireError::LimitExceeded { attempts, .. } => attempts,
            other => panic!("expected limit error, got {other:?}"),
        };
        assert_eq!(attempts.last().unwrap().bytes, 5);
        assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_retries_retryable_status_and_records_attempts() {
        let server = serve_sequence(vec![
            (503, b"busy", &[("Retry-After", "2")]),
            (200, b"ok", &[]),
        ]);
        let temp = tempfile::tempdir().unwrap();
        let policy =
            AcquirePolicy::default().retry(RetryPolicy::exponential(1, Duration::from_millis(10)));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
        let sleeps = TestArc::new(Mutex::new(Vec::new()));
        let resources = SyncHttpResources::default().with_delay({
            let sleeps = TestArc::clone(&sleeps);
            TestArc::new(move |duration| sleeps.lock().unwrap().push(duration))
        });
        let chosen = materialize(source, temp.path().join("out"));

        let acquired = SyncHttpAcquire::new(resources).acquire(chosen).unwrap();
        server.join();

        assert_eq!(std::fs::read(acquired.material.path()).unwrap(), b"ok");
        assert_eq!(*sleeps.lock().unwrap(), vec![Duration::from_secs(2)]);
        assert_eq!(acquired.evidence.attempts.len(), 2);
        assert_eq!(acquired.evidence.attempts[0].status, Some(503));
        assert_eq!(
            acquired.evidence.attempts[0].outcome,
            AttemptOutcome::RetryableStatus
        );
        assert_eq!(
            acquired.evidence.attempts[1].outcome,
            AttemptOutcome::Success
        );
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_concrete_byte_rate_pacer_downloads() {
        let server = serve_once(200, b"concrete paced", &[]);
        let temp = tempfile::tempdir().unwrap();
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap())
            .policy(AcquirePolicy::default());
        let pacer = ByteRatePacer::new(ByteRate::new(
            NonZeroU32::new(1_000_000).unwrap(),
            NonZeroU32::new(16_384).unwrap(),
        ));
        let resources = SyncHttpResources::default().with_byte_pacer(TestArc::new(pacer));
        let chosen = materialize(source, temp.path().join("out"));

        let acquired = SyncHttpAcquire::new(resources).acquire(chosen).unwrap();
        server.join();

        assert_eq!(
            std::fs::read(acquired.material.path()).unwrap(),
            b"concrete paced"
        );
        assert_eq!(acquired.evidence.attempts[0].pacing_wait, Duration::ZERO);
        assert_eq!(
            acquired.evidence.attempts[0].outcome,
            AttemptOutcome::Success
        );
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_concrete_rate_admission_downloads() {
        let server = serve_once(200, b"rate admitted", &[]);
        let temp = tempfile::tempdir().unwrap();
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap())
            .policy(AcquirePolicy::default());
        let admission = RateAdmission::new(AttemptRate::new(
            NonZeroU32::new(1_000).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ));
        let resources = SyncHttpResources::default().with_admission(TestArc::new(admission));
        let chosen = materialize(source, temp.path().join("out"));

        let acquired = SyncHttpAcquire::new(resources).acquire(chosen).unwrap();
        server.join();

        assert_eq!(
            std::fs::read(acquired.material.path()).unwrap(),
            b"rate admitted"
        );
        assert_eq!(
            acquired.evidence.attempts[0].admission_wait,
            Some(Duration::ZERO)
        );
        assert_eq!(
            acquired.evidence.attempts[0].outcome,
            AttemptOutcome::Success
        );
    }

    #[test]
    #[cfg(feature = "http-sync")]
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
            AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
        let chosen = materialize(source, temp.path().join("out"));

        let acquired = SyncHttpAcquire::default().acquire(chosen).unwrap();
        server.join();

        assert_eq!(
            std::fs::read(acquired.material.path()).unwrap(),
            b"hello world"
        );
        assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
        assert_eq!(acquired.evidence.bytes, 11);
        assert_eq!(
            acquired.evidence.resume.unwrap().outcome,
            ResumeOutcome::PartialAppended
        );
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_resume_200_to_range_restarts_full_with_fresh_stage() {
        let server = serve_sequence(vec![(200, b"fresh", &[])]);
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"stale").unwrap();
        let policy =
            AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
        let chosen = materialize(source, temp.path().join("out"));

        let acquired = SyncHttpAcquire::default().acquire(chosen).unwrap();
        server.join();

        assert_eq!(std::fs::read(acquired.material.path()).unwrap(), b"fresh");
        assert_eq!(std::fs::read(&partial).unwrap(), b"stale");
        assert_eq!(
            acquired.evidence.resume.unwrap().outcome,
            ResumeOutcome::RangeIgnoredRestarted
        );
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_resume_416_restarts_once_without_range_headers() {
        let server = serve_sequence(vec![(416, b"", &[]), (200, b"fresh", &[])]);
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"stale partial").unwrap();
        let policy =
            AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
        let chosen = materialize(source, temp.path().join("out"));

        let acquired = SyncHttpAcquire::default().acquire(chosen).unwrap();
        let first_request = server.next_request();
        let second_request = server.next_request();
        server.join();

        assert!(request_has_header_name(&first_request, "Range"));
        assert!(request_has_header_name(&first_request, "If-Range"));
        assert!(!request_has_header_name(&second_request, "Range"));
        assert!(!request_has_header_name(&second_request, "If-Range"));
        assert_eq!(std::fs::read(acquired.material.path()).unwrap(), b"fresh");
        assert_eq!(std::fs::read(&partial).unwrap(), b"stale partial");
        assert_eq!(
            acquired.evidence.resume.unwrap().outcome,
            ResumeOutcome::RangeUnsatisfiableRestarted
        );
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_resume_missing_content_range_rejects_without_publishing_target() {
        let server = serve_sequence(vec![(206, b" world", &[])]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let policy =
            AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
        let chosen = materialize(source, temp.path().join("out"));

        let error = SyncHttpAcquire::default().acquire(chosen).unwrap_err();
        server.join();

        assert!(matches!(
            error,
            AcquireError::Protocol {
                kind: ProtocolError::InvalidContentRange { .. },
                ..
            }
        ));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_resume_rejects_body_shorter_than_declared_range() {
        let server = serve_sequence(vec![(206, b" wo", &[("Content-Range", "bytes 5-10/11")])]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let policy =
            AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
        let chosen = materialize(source, temp.path().join("out"));

        let error = SyncHttpAcquire::default().acquire(chosen).unwrap_err();
        server.join();

        assert!(matches!(
            error,
            AcquireError::Protocol {
                kind: ProtocolError::InvalidContentRange { .. },
                ..
            }
        ));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
    }

    #[test]
    #[cfg(feature = "http-sync")]
    fn ureq_resume_rejects_partial_changed_after_request() {
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
            AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
        let chosen = materialize(source, temp.path().join("out"));

        let error = SyncHttpAcquire::default().acquire(chosen).unwrap_err();
        server.join();

        assert!(matches!(
            error,
            AcquireError::Protocol {
                kind: ProtocolError::InvalidContentRange { .. },
                ..
            }
        ));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&partial).unwrap(), b"hel");
    }

    #[test]
    #[cfg(feature = "http-sync")]
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
            AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, validator.clone()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
        let chosen = materialize(source, temp.path().join("out"));

        let acquired = SyncHttpAcquire::default().acquire(chosen).unwrap();
        let request = server.next_request();
        server.join();

        assert!(request_has_header(&request, "range", "bytes=5-"));
        assert!(request_has_header(&request, "if-range", "\"abc\""));
        assert_eq!(
            std::fs::read(acquired.material.path()).unwrap(),
            b"hello world"
        );
        let resume = acquired.evidence.resume.unwrap();
        assert_eq!(resume.outcome, ResumeOutcome::PartialAppended);
        assert_eq!(resume.validator, validator);
        assert_eq!(acquired.evidence.validator, Some(test_validator()));
    }

    #[test]
    #[cfg(feature = "http-sync")]
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
            AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
        let chosen = materialize(source, temp.path().join("out"));

        let error = SyncHttpAcquire::default().acquire(chosen).unwrap_err();
        server.join();

        assert!(matches!(
            error,
            AcquireError::Protocol {
                kind: ProtocolError::ResumeValidatorMismatch,
                ..
            }
        ));
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_acquire_downloads_file_to_local_material() {
        block_on_reqwest(async {
            let body = b"async downloaded bytes";
            let server = serve_once(200, body, &[]);
            let temp = tempfile::tempdir().unwrap();
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());
            let chosen = materialize(source, temp.path().join("out"));

            let acquired = AsyncHttpAcquire::default().acquire(chosen).await.unwrap();
            server.join();

            assert!(matches!(
                &acquired.material,
                LocalMaterial::StagedFile { .. }
            ));
            assert_eq!(std::fs::read(acquired.material.path()).unwrap(), body);
            assert_eq!(acquired.evidence.status, 200);
            assert_eq!(acquired.evidence.bytes, body.len() as u64);
            assert_eq!(acquired.evidence.content_length, Some(body.len() as u64));
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_acquire_does_not_publish_materialize_target() {
        block_on_reqwest(async {
            let server = serve_once(200, b"replacement", &[]);
            let temp = tempfile::tempdir().unwrap();
            let target = temp.path().join("target.bin");
            std::fs::write(&target, b"original").unwrap();
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

            let acquired = AsyncHttpAcquire::default()
                .acquire(materialize(source, &target))
                .await
                .unwrap();
            server.join();

            assert_eq!(std::fs::read(&target).unwrap(), b"original");
            drop(acquired);
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_acquire_does_not_create_target_parent() {
        block_on_reqwest(async {
            let server = serve_once(200, b"staged", &[]);
            let temp = tempfile::tempdir().unwrap();
            let target = temp.path().join("absent/target.bin");
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

            let acquired = AsyncHttpAcquire::default()
                .acquire(materialize(source, &target))
                .await
                .unwrap();
            server.join();

            assert!(!target.parent().unwrap().exists());
            assert_eq!(std::fs::read(acquired.material.path()).unwrap(), b"staged");
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_acquired_material_is_removed_when_abandoned() {
        block_on_reqwest(async {
            let server = serve_once(200, b"temporary", &[]);
            let temp = tempfile::tempdir().unwrap();
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());

            let acquired = AsyncHttpAcquire::default()
                .acquire(materialize(source, temp.path().join("target.bin")))
                .await
                .unwrap();
            server.join();
            let staged_path = acquired.material.path().to_path_buf();
            assert!(staged_path.exists());
            drop(acquired);

            assert!(!staged_path.exists());
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_acquire_rejects_non_success_status_without_touching_target() {
        block_on_reqwest(async {
            let server = serve_once(404, b"not found", &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            std::fs::write(&destination, b"old").unwrap();
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());
            let chosen = materialize(source, &destination);

            let error = AsyncHttpAcquire::default()
                .acquire(chosen)
                .await
                .unwrap_err();
            server.join();

            assert!(matches!(
                error,
                AcquireError::HttpStatus {
                    status: 404,
                    retryable: false,
                    ..
                }
            ));
            assert_eq!(std::fs::read(&destination).unwrap(), b"old");
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_acquire_enforces_max_bytes_without_publishing_target() {
        block_on_reqwest(async {
            let server = serve_once(200, b"too large", &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let policy = AcquirePolicy::default().max_bytes(3);
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
            let chosen = materialize(source, &destination);

            let error = AsyncHttpAcquire::default()
                .acquire(chosen)
                .await
                .unwrap_err();
            server.join();

            assert!(matches!(error, AcquireError::LimitExceeded { max: 3, .. }));
            assert!(!destination.exists());
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_resume_limit_evidence_records_materialized_partial_bytes() {
        block_on_reqwest(async {
            let server = serve_once(206, b" world", &[("Content-Range", "bytes 5-10/11")]);
            let temp = tempfile::tempdir().unwrap();
            let partial = temp.path().join("artifact.part");
            std::fs::write(&partial, b"hello").unwrap();
            let policy = AcquirePolicy::default()
                .max_bytes(8)
                .resume(ResumePolicy::if_range(&partial, test_validator()));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

            let error = AsyncHttpAcquire::default()
                .acquire(materialize(source, temp.path().join("out")))
                .await
                .unwrap_err();
            server.join();

            let attempts = match error {
                AcquireError::LimitExceeded { attempts, .. } => attempts,
                other => panic!("expected limit error, got {other:?}"),
            };
            assert_eq!(attempts.last().unwrap().bytes, 5);
            assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_retries_retryable_status_and_records_attempts() {
        block_on_reqwest(async {
            let server = serve_sequence(vec![
                (503, b"busy", &[("Retry-After", "2")]),
                (200, b"ok", &[]),
            ]);
            let temp = tempfile::tempdir().unwrap();
            let policy = AcquirePolicy::default()
                .retry(RetryPolicy::exponential(1, Duration::from_millis(10)));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
            let sleeps = TestArc::new(Mutex::new(Vec::new()));
            let resources = AsyncHttpResources::default().with_delay({
                let sleeps = TestArc::clone(&sleeps);
                TestArc::new(move |duration| {
                    sleeps.lock().unwrap().push(duration);
                    Box::pin(std::future::ready(()))
                })
            });
            let chosen = materialize(source, temp.path().join("out"));

            let acquired = AsyncHttpAcquire::new(resources)
                .acquire(chosen)
                .await
                .unwrap();
            server.join();

            assert_eq!(std::fs::read(acquired.material.path()).unwrap(), b"ok");
            assert_eq!(*sleeps.lock().unwrap(), vec![Duration::from_secs(2)]);
            assert_eq!(acquired.evidence.attempts.len(), 2);
            assert_eq!(acquired.evidence.attempts[0].status, Some(503));
            assert_eq!(
                acquired.evidence.attempts[0].outcome,
                AttemptOutcome::RetryableStatus
            );
            assert_eq!(
                acquired.evidence.attempts[1].outcome,
                AttemptOutcome::Success
            );
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_concrete_byte_rate_pacer_downloads() {
        block_on_reqwest(async {
            let server = serve_once(200, b"async concrete paced", &[]);
            let temp = tempfile::tempdir().unwrap();
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap())
                .policy(AcquirePolicy::default());
            let pacer = ByteRatePacer::new(ByteRate::new(
                NonZeroU32::new(1_000_000).unwrap(),
                NonZeroU32::new(16_384).unwrap(),
            ));
            let resources = AsyncHttpResources::default().with_byte_pacer(TestArc::new(pacer));
            let chosen = materialize(source, temp.path().join("out"));

            let acquired = AsyncHttpAcquire::new(resources)
                .acquire(chosen)
                .await
                .unwrap();
            server.join();

            assert_eq!(
                std::fs::read(acquired.material.path()).unwrap(),
                b"async concrete paced"
            );
            assert_eq!(acquired.evidence.attempts[0].pacing_wait, Duration::ZERO);
            assert_eq!(
                acquired.evidence.attempts[0].outcome,
                AttemptOutcome::Success
            );
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_concrete_rate_admission_downloads() {
        block_on_reqwest(async {
            let server = serve_once(200, b"async rate admitted", &[]);
            let temp = tempfile::tempdir().unwrap();
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap())
                .policy(AcquirePolicy::default());
            let admission = RateAdmission::new(AttemptRate::new(
                NonZeroU32::new(1_000).unwrap(),
                NonZeroU32::new(1).unwrap(),
            ));
            let resources = AsyncHttpResources::default().with_admission(TestArc::new(admission));
            let chosen = materialize(source, temp.path().join("out"));

            let acquired = AsyncHttpAcquire::new(resources)
                .acquire(chosen)
                .await
                .unwrap();
            server.join();

            assert_eq!(
                std::fs::read(acquired.material.path()).unwrap(),
                b"async rate admitted"
            );
            assert_eq!(
                acquired.evidence.attempts[0].admission_wait,
                Some(Duration::ZERO)
            );
            assert_eq!(
                acquired.evidence.attempts[0].outcome,
                AttemptOutcome::Success
            );
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_resume_206_appends_after_valid_content_range() {
        block_on_reqwest(async {
            let server = serve_sequence(vec![(
                206,
                b" world",
                &[("Content-Range", "bytes 5-10/11")],
            )]);
            let temp = tempfile::tempdir().unwrap();
            let partial = temp.path().join("artifact.part");
            std::fs::write(&partial, b"hello").unwrap();
            let policy =
                AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
            let chosen = materialize(source, temp.path().join("out"));

            let acquired = AsyncHttpAcquire::default().acquire(chosen).await.unwrap();
            server.join();

            assert_eq!(
                std::fs::read(acquired.material.path()).unwrap(),
                b"hello world"
            );
            assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
            assert_eq!(acquired.evidence.bytes, 11);
            assert_eq!(
                acquired.evidence.resume.unwrap().outcome,
                ResumeOutcome::PartialAppended
            );
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_resume_missing_content_range_rejects_without_publishing_target() {
        block_on_reqwest(async {
            let server = serve_sequence(vec![(206, b" world", &[])]);
            let temp = tempfile::tempdir().unwrap();
            let target = temp.path().join("artifact.bin");
            let partial = temp.path().join("artifact.part");
            std::fs::write(&partial, b"hello").unwrap();
            let policy =
                AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);

            let error = AsyncHttpAcquire::default()
                .acquire(materialize(source, &target))
                .await
                .unwrap_err();
            server.join();

            assert!(matches!(
                error,
                AcquireError::Protocol {
                    kind: ProtocolError::InvalidContentRange { .. },
                    ..
                }
            ));
            assert!(!target.exists());
            assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_resume_rejects_body_shorter_than_declared_range() {
        block_on_reqwest(async {
            let server = serve_sequence(vec![(206, b" wo", &[("Content-Range", "bytes 5-10/11")])]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let partial = temp.path().join("artifact.part");
            std::fs::write(&partial, b"hello").unwrap();
            let policy =
                AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
            let chosen = materialize(source, temp.path().join("out"));

            let error = AsyncHttpAcquire::default()
                .acquire(chosen)
                .await
                .unwrap_err();
            server.join();

            assert!(matches!(
                error,
                AcquireError::Protocol {
                    kind: ProtocolError::InvalidContentRange { .. },
                    ..
                }
            ));
            assert!(!destination.exists());
            assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_resume_rejects_partial_changed_after_request() {
        block_on_reqwest(async {
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
                AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
            let chosen = materialize(source, temp.path().join("out"));

            let error = AsyncHttpAcquire::default()
                .acquire(chosen)
                .await
                .unwrap_err();
            server.join();

            assert!(matches!(
                error,
                AcquireError::Protocol {
                    kind: ProtocolError::InvalidContentRange { .. },
                    ..
                }
            ));
            assert!(!destination.exists());
            assert_eq!(std::fs::read(&partial).unwrap(), b"hel");
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_if_range_resume_sends_range_and_if_range_and_appends_206() {
        block_on_reqwest(async {
            let server = serve_sequence(vec![(
                206,
                b" world",
                &[("Content-Range", "bytes 5-10/11"), ("ETag", "\"abc\"")],
            )]);
            let temp = tempfile::tempdir().unwrap();
            let partial = temp.path().join("artifact.part");
            std::fs::write(&partial, b"hello").unwrap();
            let validator = test_validator();
            let policy = AcquirePolicy::default()
                .resume(ResumePolicy::if_range(&partial, validator.clone()));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
            let chosen = materialize(source, temp.path().join("out"));

            let acquired = AsyncHttpAcquire::default().acquire(chosen).await.unwrap();
            let request = server.next_request();
            server.join();

            assert!(request_has_header(&request, "range", "bytes=5-"));
            assert!(request_has_header(&request, "if-range", "\"abc\""));
            assert_eq!(
                std::fs::read(acquired.material.path()).unwrap(),
                b"hello world"
            );
            let resume = acquired.evidence.resume.unwrap();
            assert_eq!(resume.outcome, ResumeOutcome::PartialAppended);
            assert_eq!(resume.validator, validator);
            assert_eq!(acquired.evidence.validator, Some(test_validator()));
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_if_range_rejects_conflicting_response_validator() {
        block_on_reqwest(async {
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
                AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
            let chosen = materialize(source, temp.path().join("out"));

            let error = AsyncHttpAcquire::default()
                .acquire(chosen)
                .await
                .unwrap_err();
            server.join();

            assert!(matches!(
                error,
                AcquireError::Protocol {
                    kind: ProtocolError::ResumeValidatorMismatch,
                    ..
                }
            ));
            assert!(!destination.exists());
            assert_eq!(std::fs::read(&partial).unwrap(), b"hello");
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_resume_200_to_range_restarts_full_with_fresh_stage() {
        block_on_reqwest(async {
            let server = serve_sequence(vec![(200, b"fresh", &[])]);
            let temp = tempfile::tempdir().unwrap();
            let partial = temp.path().join("artifact.part");
            std::fs::write(&partial, b"stale").unwrap();
            let policy =
                AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
            let chosen = materialize(source, temp.path().join("out"));

            let acquired = AsyncHttpAcquire::default().acquire(chosen).await.unwrap();
            server.join();

            assert_eq!(std::fs::read(acquired.material.path()).unwrap(), b"fresh");
            assert_eq!(std::fs::read(&partial).unwrap(), b"stale");
            assert_eq!(
                acquired.evidence.resume.unwrap().outcome,
                ResumeOutcome::RangeIgnoredRestarted
            );
        });
    }

    #[test]
    #[cfg(feature = "http-async")]
    fn reqwest_resume_416_restarts_once_without_modifying_partial() {
        block_on_reqwest(async {
            let server = serve_sequence(vec![(416, b"", &[]), (200, b"fresh", &[])]);
            let temp = tempfile::tempdir().unwrap();
            let partial = temp.path().join("artifact.part");
            std::fs::write(&partial, b"stale partial").unwrap();
            let policy =
                AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, test_validator()));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap()).policy(policy);
            let chosen = materialize(source, temp.path().join("out"));

            let acquired = AsyncHttpAcquire::default().acquire(chosen).await.unwrap();
            let first_request = server.next_request();
            let second_request = server.next_request();
            server.join();

            assert!(request_has_header_name(&first_request, "Range"));
            assert!(request_has_header_name(&first_request, "If-Range"));
            assert!(!request_has_header_name(&second_request, "Range"));
            assert!(!request_has_header_name(&second_request, "If-Range"));
            assert_eq!(std::fs::read(acquired.material.path()).unwrap(), b"fresh");
            assert_eq!(std::fs::read(&partial).unwrap(), b"stale partial");
            assert_eq!(
                acquired.evidence.resume.unwrap().outcome,
                ResumeOutcome::RangeUnsatisfiableRestarted
            );
        });
    }

    #[test]
    #[cfg(all(feature = "http-async", feature = "hash", feature = "blake3"))]
    fn reqwest_acquire_flows_into_descriptor_verify() {
        block_on_reqwest(async {
            let body = b"reqwest verified bytes";
            let expected = blake3::hash(body).to_hex().to_string();
            let server = serve_once(200, body, &[]);
            let temp = tempfile::tempdir().unwrap();
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());
            let chosen = materialize(source, temp.path().join("out"));

            let acquired = AsyncHttpAcquire::default().acquire(chosen).await.unwrap();
            server.join();
            let verified = HashVerify::<Blake3>::new()
                .verify(
                    acquired,
                    ArtifactDescriptor::new(expected, body.len() as u64),
                )
                .unwrap();

            assert!(matches!(
                verified.material,
                LocalMaterial::StagedFile { .. }
            ));
        });
    }

    #[test]
    #[cfg(all(feature = "http-async", feature = "hash", feature = "blake3"))]
    fn reqwest_acquire_flows_into_local_apply_after_verify() {
        block_on_reqwest(async {
            let body = b"reqwest apply bytes";
            let expected = blake3::hash(body).to_hex().to_string();
            let server = serve_once(200, body, &[]);
            let temp = tempfile::tempdir().unwrap();
            let final_path = temp.path().join("final.bin");
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());
            let chosen = materialize(source, &final_path);

            let acquired = AsyncHttpAcquire::default().acquire(chosen).await.unwrap();
            server.join();
            let verified = HashVerify::<Blake3>::new()
                .verify(acquired, DigestValue::new(expected))
                .unwrap();
            let applied = LocalApply.apply(verified).unwrap();

            assert_eq!(std::fs::read(final_path).unwrap(), body);
            assert_eq!(applied.evidence.current.files, 1);
        });
    }

    #[test]
    #[cfg(all(feature = "http-sync", feature = "hash", feature = "blake3"))]
    fn ureq_acquire_flows_into_descriptor_verify() {
        let body = b"verified bytes";
        let expected = blake3::hash(body).to_hex().to_string();
        let server = serve_once(200, body, &[]);
        let temp = tempfile::tempdir().unwrap();
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());
        let chosen = materialize(source, temp.path().join("out"));

        let acquired = SyncHttpAcquire::default().acquire(chosen).unwrap();
        server.join();
        let verified = HashVerify::<Blake3>::new()
            .verify(
                acquired,
                ArtifactDescriptor::new(expected, body.len() as u64),
            )
            .unwrap();

        assert!(matches!(
            verified.material,
            LocalMaterial::StagedFile { .. }
        ));
    }

    #[test]
    #[cfg(all(feature = "http-sync", feature = "hash", feature = "blake3"))]
    fn ureq_acquire_flows_into_local_apply_after_verify() {
        let body = b"apply bytes";
        let expected = blake3::hash(body).to_hex().to_string();
        let server = serve_once(200, body, &[]);
        let temp = tempfile::tempdir().unwrap();
        let final_path = temp.path().join("final.bin");
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap());
        let chosen = materialize(source, &final_path);

        let acquired = SyncHttpAcquire::default().acquire(chosen).unwrap();
        server.join();
        let verified = HashVerify::<Blake3>::new()
            .verify(acquired, DigestValue::new(expected))
            .unwrap();
        let applied = LocalApply.apply(verified).unwrap();

        assert_eq!(std::fs::read(final_path).unwrap(), body);
        assert_eq!(applied.evidence.current.files, 1);
    }

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    struct TestServer {
        url: String,
        handle: thread::JoinHandle<()>,
        requests: mpsc::Receiver<String>,
    }

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    impl TestServer {
        fn next_request(&self) -> String {
            self.requests.recv().unwrap()
        }

        fn join(self) {
            self.handle.join().unwrap();
        }
    }

    #[cfg(feature = "http-async")]
    fn block_on_reqwest<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    fn serve_once(
        status: u16,
        body: &'static [u8],
        headers: &'static [(&'static str, &'static str)],
    ) -> TestServer {
        serve_sequence(vec![(status, body, headers)])
    }

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    fn request_has_header(request: &str, name: &str, value: &str) -> bool {
        request.lines().any(|line| {
            line.split_once(':')
                .is_some_and(|(actual_name, actual_value)| {
                    actual_name.eq_ignore_ascii_case(name) && actual_value.trim() == value
                })
        })
    }

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    fn request_has_header_name(request: &str, name: &str) -> bool {
        request.lines().any(|line| {
            line.split_once(':')
                .is_some_and(|(actual_name, _)| actual_name.eq_ignore_ascii_case(name))
        })
    }

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
    type TestResponse = (u16, &'static [u8], &'static [(&'static str, &'static str)]);

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
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

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
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

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
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

    #[cfg(any(feature = "http-async", feature = "http-sync"))]
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
