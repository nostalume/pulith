use std::fmt;
#[cfg(feature = "reqwest")]
use std::future::Future;
use std::io;
#[cfg(feature = "ureq")]
use std::io::{Read, Write};
use std::num::NonZeroU32;
use std::path::Path;
use std::path::PathBuf;
#[cfg(feature = "reqwest")]
use std::pin::Pin;
#[cfg(any(feature = "reqwest", feature = "ureq"))]
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[cfg(any(feature = "reqwest", feature = "ureq"))]
use governor::clock::Clock;

#[cfg(feature = "ureq")]
use crate::AcquireNode;
#[cfg(feature = "reqwest")]
use crate::AsyncAcquireNode;
#[cfg(any(feature = "reqwest", feature = "ureq"))]
use crate::{Acquired, Chosen, LocalMaterial, MaterialKind};

#[derive(Debug)]
pub enum AcquireError {
    InvalidUrl {
        input: String,
    },
    UnsupportedScheme {
        scheme: String,
    },
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
    Admission {
        url: url::Url,
        kind: AdmissionError,
        attempts: Vec<AttemptEvidence>,
        resume: Option<ResumeEvidence>,
    },
    Pacing {
        url: url::Url,
        kind: PacingError,
        attempts: Vec<AttemptEvidence>,
        resume: Option<ResumeEvidence>,
    },
    Local {
        url: Option<url::Url>,
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    UnsafeDestination {
        path: PathBuf,
        kind: UnsafeDestination,
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
    InvalidContentRange {
        expected_start: u64,
        header: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsafeDestination {
    Symlink,
    NonFile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    Unavailable,
    Closed,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PacingError {
    Unavailable,
    Closed,
    Rejected,
}

impl From<PacingError> for AdmissionError {
    fn from(error: PacingError) -> Self {
        match error {
            PacingError::Unavailable => Self::Unavailable,
            PacingError::Closed => Self::Closed,
            PacingError::Rejected => Self::Rejected,
        }
    }
}

impl From<AdmissionError> for PacingError {
    fn from(error: AdmissionError) -> Self {
        match error {
            AdmissionError::Unavailable => Self::Unavailable,
            AdmissionError::Closed => Self::Closed,
            AdmissionError::Rejected => Self::Rejected,
        }
    }
}

impl fmt::Display for AcquireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl { input } => write!(f, "invalid net acquire URL: {input}"),
            Self::UnsupportedScheme { scheme } => {
                write!(f, "unsupported net acquire URL scheme: {scheme}")
            }
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
            Self::Admission { url, kind, .. } => {
                write!(f, "net acquire admission failed for {url}: {kind:?}")
            }
            Self::Pacing { url, kind, .. } => {
                write!(f, "net acquire byte pacing failed for {url}: {kind:?}")
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
            Self::UnsafeDestination { path, kind } => {
                write!(
                    f,
                    "net acquire destination is unsafe ({kind:?}): {}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for AcquireError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Local { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl AcquireError {
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
    #[allow(
        clippy::result_large_err,
        reason = "AcquireError intentionally carries complete retry and resume evidence"
    )]
    pub fn parse(input: &str) -> Result<Self, AcquireError> {
        let url = url::Url::parse(input).map_err(|_| AcquireError::InvalidUrl {
            input: input.to_string(),
        })?;
        match url.scheme() {
            "http" | "https" => Ok(Self { url }),
            scheme => Err(AcquireError::UnsupportedScheme {
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AcquirePolicy {
    pub timeout: Option<Duration>,
    pub max_bytes: Option<u64>,
    pub headers: Vec<(String, String)>,
    pub retry: RetryPolicy,
    pub resume: ResumePolicy,
    pub admission: AdmissionMode,
    pub byte_pacing: BytePacingMode,
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

    pub fn admission(mut self, admission: AdmissionMode) -> Self {
        self.admission = admission;
        self
    }

    pub fn shared_admission(self) -> Self {
        self.admission(AdmissionMode::Shared)
    }

    pub fn byte_pacing(mut self, byte_pacing: BytePacingMode) -> Self {
        self.byte_pacing = byte_pacing;
        self
    }

    pub fn shared_byte_pacing(self) -> Self {
        self.byte_pacing(BytePacingMode::Shared)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AdmissionMode {
    #[default]
    Unbounded,
    Shared,
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
    #[cfg(any(feature = "reqwest", feature = "ureq"))]
    limiter: governor::DefaultDirectRateLimiter,
}

impl RateAdmission {
    pub fn new(rate: AttemptRate) -> Self {
        #[cfg(any(feature = "reqwest", feature = "ureq"))]
        let quota = governor::Quota::per_second(rate.attempts_per_second())
            .allow_burst(rate.burst_attempts());
        Self {
            rate,
            #[cfg(any(feature = "reqwest", feature = "ureq"))]
            limiter: governor::RateLimiter::direct(quota),
        }
    }

    pub const fn rate(&self) -> AttemptRate {
        self.rate
    }

    #[cfg(any(feature = "reqwest", feature = "ureq"))]
    fn check(&self) -> Option<Duration> {
        match self.limiter.check() {
            Ok(_) => None,
            Err(not_until) => Some(not_until.wait_time_from(self.limiter.clock().now())),
        }
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BytePacingMode {
    #[default]
    Unbounded,
    Shared,
}

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
    #[cfg(any(feature = "reqwest", feature = "ureq"))]
    limiter: governor::DefaultDirectRateLimiter,
}

impl ByteRatePacer {
    pub fn new(rate: ByteRate) -> Self {
        #[cfg(any(feature = "reqwest", feature = "ureq"))]
        let quota =
            governor::Quota::per_second(rate.bytes_per_second()).allow_burst(rate.burst_bytes());
        Self {
            rate,
            #[cfg(any(feature = "reqwest", feature = "ureq"))]
            limiter: governor::RateLimiter::direct(quota),
        }
    }

    pub const fn rate(&self) -> ByteRate {
        self.rate
    }

    #[cfg(any(feature = "reqwest", feature = "ureq"))]
    fn next_batch(&self, remaining: u64) -> Option<NonZeroU32> {
        NonZeroU32::new(remaining.min(u64::from(self.rate.burst_bytes().get())) as u32)
    }

    #[cfg(any(feature = "reqwest", feature = "ureq"))]
    fn check_batch(&self, batch: NonZeroU32) -> Result<Option<Duration>, PacingError> {
        match self.limiter.check_n(batch) {
            Ok(Ok(_)) => Ok(None),
            Ok(Err(not_until)) => Ok(Some(not_until.wait_time_from(self.limiter.clock().now()))),
            Err(_) => Err(PacingError::Rejected),
        }
    }
}

impl fmt::Debug for ByteRatePacer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ByteRatePacer")
            .field("rate", &self.rate)
            .finish_non_exhaustive()
    }
}

pub struct AdmissionPermit {
    waited: Duration,
}

impl AdmissionPermit {
    pub fn immediate() -> Self {
        Self {
            waited: Duration::ZERO,
        }
    }

    pub fn waited(waited: Duration) -> Self {
        Self { waited }
    }

    pub fn waited_for(&self) -> Duration {
        self.waited
    }
}

/// Evidence returned when a body chunk is admitted for staged writing.
pub struct BytePacingPermit {
    waited: Duration,
}

impl BytePacingPermit {
    pub fn immediate() -> Self {
        Self {
            waited: Duration::ZERO,
        }
    }

    pub fn waited(waited: Duration) -> Self {
        Self { waited }
    }

    pub fn waited_for(&self) -> Duration {
        self.waited
    }
}

#[cfg(feature = "ureq")]
pub trait SyncAdmission: Send + Sync {
    fn enter(&self) -> Result<AdmissionPermit, AdmissionError>;
}

#[cfg(feature = "reqwest")]
pub trait AsyncAdmission: Send + Sync {
    fn enter(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<AdmissionPermit, AdmissionError>> + Send + '_>>;
}

#[cfg(feature = "ureq")]
impl SyncAdmission for RateAdmission {
    fn enter(&self) -> Result<AdmissionPermit, AdmissionError> {
        let mut waited = Duration::ZERO;
        loop {
            match self.check() {
                None => return Ok(AdmissionPermit::waited(waited)),
                Some(wait) => {
                    std::thread::sleep(wait);
                    waited = waited.saturating_add(wait);
                }
            }
        }
    }
}

#[cfg(feature = "reqwest")]
impl AsyncAdmission for RateAdmission {
    fn enter(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<AdmissionPermit, AdmissionError>> + Send + '_>> {
        Box::pin(async move {
            let mut waited = Duration::ZERO;
            loop {
                match self.check() {
                    None => return Ok(AdmissionPermit::waited(waited)),
                    Some(wait) => {
                        tokio::time::sleep(wait).await;
                        waited = waited.saturating_add(wait);
                    }
                }
            }
        })
    }
}

#[cfg(feature = "ureq")]
/// Synchronous body-copy pacer.
///
/// `before_chunk` is called after `max_bytes` accepts an observed chunk and
/// before the chunk is written into staging.
pub trait SyncBytePacer: Send + Sync {
    fn before_chunk(&self, bytes: u64) -> Result<BytePacingPermit, PacingError>;
}

#[cfg(feature = "reqwest")]
/// Asynchronous body-copy pacer.
///
/// `before_chunk` is awaited after `max_bytes` accepts an observed chunk and
/// before the chunk is written into staging.
pub trait AsyncBytePacer: Send + Sync {
    fn before_chunk(
        &self,
        bytes: u64,
    ) -> Pin<Box<dyn Future<Output = Result<BytePacingPermit, PacingError>> + Send + '_>>;
}

#[cfg(feature = "ureq")]
impl SyncBytePacer for ByteRatePacer {
    fn before_chunk(&self, bytes: u64) -> Result<BytePacingPermit, PacingError> {
        if bytes == 0 {
            return Ok(BytePacingPermit::immediate());
        }

        let mut remaining = bytes;
        let mut waited = Duration::ZERO;
        while let Some(batch) = self.next_batch(remaining) {
            loop {
                match self.check_batch(batch)? {
                    None => break,
                    Some(wait) => {
                        std::thread::sleep(wait);
                        waited = waited.saturating_add(wait);
                    }
                }
            }
            remaining -= u64::from(batch.get());
        }
        Ok(BytePacingPermit::waited(waited))
    }
}

#[cfg(feature = "reqwest")]
impl AsyncBytePacer for ByteRatePacer {
    fn before_chunk(
        &self,
        bytes: u64,
    ) -> Pin<Box<dyn Future<Output = Result<BytePacingPermit, PacingError>> + Send + '_>> {
        Box::pin(async move {
            if bytes == 0 {
                return Ok(BytePacingPermit::immediate());
            }

            let mut remaining = bytes;
            let mut waited = Duration::ZERO;
            while let Some(batch) = self.next_batch(remaining) {
                loop {
                    match self.check_batch(batch)? {
                        None => break,
                        Some(wait) => {
                            tokio::time::sleep(wait).await;
                            waited = waited.saturating_add(wait);
                        }
                    }
                }
                remaining -= u64::from(batch.get());
            }
            Ok(BytePacingPermit::waited(waited))
        })
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumePolicy {
    pub mode: ResumeMode,
}

impl Default for ResumePolicy {
    fn default() -> Self {
        Self::restart_only()
    }
}

impl ResumePolicy {
    pub fn restart_only() -> Self {
        Self {
            mode: ResumeMode::RestartOnly,
        }
    }

    pub fn unvalidated(partial_path: impl Into<PathBuf>) -> Self {
        Self {
            mode: ResumeMode::Unvalidated {
                partial_path: partial_path.into(),
            },
        }
    }

    pub fn if_range(partial_path: impl Into<PathBuf>, validator: Validator) -> Self {
        Self {
            mode: ResumeMode::IfRange {
                partial_path: partial_path.into(),
                validator,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResumeMode {
    RestartOnly,
    Unvalidated {
        partial_path: PathBuf,
    },
    IfRange {
        partial_path: PathBuf,
        validator: Validator,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Validator {
    Etag(String),
    LastModified(SystemTime),
}

impl Validator {
    pub fn strong_etag(value: impl Into<String>) -> Option<Self> {
        parse_strong_etag(&value.into()).map(Self::Etag)
    }

    pub fn last_modified(time: SystemTime) -> Self {
        Self::LastModified(time)
    }

    fn if_range_value(&self) -> String {
        match self {
            Self::Etag(value) => value.clone(),
            Self::LastModified(time) => httpdate::fmt_http_date(*time),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSource {
    pub(crate) url: RemoteUrl,
    pub(crate) destination: PathBuf,
    pub(crate) policy: AcquirePolicy,
}

impl RemoteSource {
    pub fn new(url: RemoteUrl, destination: impl Into<PathBuf>) -> Self {
        Self {
            url,
            destination: destination.into(),
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

    pub fn destination(&self) -> &Path {
        &self.destination
    }

    pub fn policy_ref(&self) -> &AcquirePolicy {
        &self.policy
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcquireEvidence {
    pub url: url::Url,
    pub final_path: PathBuf,
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
    pub validator: Option<Validator>,
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

impl AttemptEvidence {
    pub fn new(attempt: u32, outcome: AttemptOutcome) -> Self {
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

    pub fn response(
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

    pub fn transfer(
        attempt: u32,
        status: u16,
        bytes: u64,
        content_length: Option<u64>,
        admission_wait: Option<Duration>,
        outcome: AttemptOutcome,
    ) -> Self {
        Self::response(attempt, status, content_length, admission_wait, outcome).with_bytes(bytes)
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_bytes(mut self, bytes: u64) -> Self {
        self.bytes = bytes;
        self
    }

    pub fn with_content_length(mut self, content_length: Option<u64>) -> Self {
        self.content_length = content_length;
        self
    }

    pub fn with_retry_after(mut self, retry_after: Option<Duration>) -> Self {
        self.retry_after = retry_after;
        self
    }

    pub fn with_planned_delay(mut self, planned_delay: Option<Duration>) -> Self {
        self.planned_delay = planned_delay;
        self
    }

    pub fn with_admission_wait(mut self, admission_wait: Option<Duration>) -> Self {
        self.admission_wait = admission_wait;
        self
    }

    pub fn with_pacing_wait(mut self, pacing_wait: Duration) -> Self {
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
    AdmissionRejected,
    PacingRejected,
}

#[cfg(feature = "ureq")]
pub type SyncDelay = Arc<dyn Fn(Duration) + Send + Sync>;

#[cfg(feature = "reqwest")]
pub type AsyncDelayFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

#[cfg(feature = "reqwest")]
pub type AsyncDelay = Arc<dyn Fn(Duration) -> AsyncDelayFuture + Send + Sync>;

#[cfg(feature = "ureq")]
#[derive(Clone)]
pub struct UreqResource {
    agent: ureq::Agent,
    delay: SyncDelay,
    admission: Option<Arc<dyn SyncAdmission>>,
    byte_pacer: Option<Arc<dyn SyncBytePacer>>,
}

#[cfg(feature = "ureq")]
impl Default for UreqResource {
    fn default() -> Self {
        Self {
            agent: ureq::Agent::new_with_defaults(),
            delay: Arc::new(std::thread::sleep),
            admission: None,
            byte_pacer: None,
        }
    }
}

#[cfg(feature = "ureq")]
impl UreqResource {
    pub fn from_agent(agent: ureq::Agent) -> Self {
        Self {
            agent,
            ..Self::default()
        }
    }

    pub fn agent(&self) -> &ureq::Agent {
        &self.agent
    }

    pub fn with_delay(mut self, delay: SyncDelay) -> Self {
        self.delay = delay;
        self
    }

    pub fn delay(&self) -> &SyncDelay {
        &self.delay
    }

    pub fn with_admission(mut self, admission: Arc<dyn SyncAdmission>) -> Self {
        self.admission = Some(admission);
        self
    }

    pub fn admission(&self) -> Option<&Arc<dyn SyncAdmission>> {
        self.admission.as_ref()
    }

    pub fn with_byte_pacer(mut self, byte_pacer: Arc<dyn SyncBytePacer>) -> Self {
        self.byte_pacer = Some(byte_pacer);
        self
    }

    pub fn byte_pacer(&self) -> Option<&Arc<dyn SyncBytePacer>> {
        self.byte_pacer.as_ref()
    }
}

#[cfg(feature = "ureq")]
impl std::fmt::Debug for UreqResource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UreqResource")
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "ureq")]
#[derive(Clone, Debug, Default)]
pub struct UreqAcquire<R = UreqResource> {
    resources: R,
}

#[cfg(feature = "ureq")]
impl UreqAcquire<UreqResource> {
    pub fn new() -> Self {
        Self::with_resource(UreqResource::default())
    }

    pub fn with_resource(resources: UreqResource) -> Self {
        Self { resources }
    }

    pub fn resources(&self) -> &UreqResource {
        &self.resources
    }
}

#[cfg(feature = "ureq")]
impl<I> AcquireNode<Chosen<I, RemoteSource>> for UreqAcquire<UreqResource> {
    type Material = LocalMaterial;
    type Evidence = AcquireEvidence;
    type Error = AcquireError;
    type Output = Acquired<I, LocalMaterial, AcquireEvidence>;

    fn acquire_node(&self, node: Chosen<I, RemoteSource>) -> Result<Self::Output, Self::Error> {
        let source = node.source;
        let parent = destination_parent(&source.destination)?;
        std::fs::create_dir_all(&parent).map_err(|err| {
            AcquireError::local(Some(&source.url), "create download parent", &parent, err)
        })?;
        reject_existing_unsafe_destination(&source.destination)?;

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
            let (_admission_permit, admission_wait) = admit_sync_attempt(
                self.resources.admission(),
                &source,
                attempt,
                &mut attempts,
                &resume,
            )?;
            let mut request = self.resources.agent.get(source.url.as_str());
            for (name, value) in &source.policy.headers {
                request = request.header(name, value);
            }
            if let Some(resume) = &resume_context {
                request = request.header("Range", format!("bytes={}-", resume.partial_bytes));
                if let Some(validator) = &resume.validator {
                    request = request.header("If-Range", validator.if_range_value());
                }
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
                let content_range = response
                    .headers()
                    .get("content-range")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string);
                content_range
                    .as_deref()
                    .and_then(|value| parse_content_range(value, resume_context.partial_bytes))
                    .ok_or_else(|| AcquireError::Protocol {
                        url: source.url.as_url().clone(),
                        kind: ProtocolError::InvalidContentRange {
                            expected_start: resume_context.partial_bytes,
                            header: content_range,
                        },
                        attempts: attempts.clone(),
                        resume: resume.clone(),
                    })?;
                Some(resume_context.clone())
            } else {
                if let Some(resume_context) = resume_context {
                    resume =
                        Some(resume_context.into_evidence(ResumeOutcome::RangeIgnoredRestarted));
                }
                None
            };

            let mut temp = tempfile::NamedTempFile::new_in(&parent).map_err(|err| {
                AcquireError::local(Some(&source.url), "create download temp file", &parent, err)
            })?;
            let initial_bytes = if let Some(resume_context) = &append_resume {
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
                })?;
                resume_context.partial_bytes
            } else {
                0
            };
            let active_pacer = match source.policy.byte_pacing {
                BytePacingMode::Unbounded => None,
                BytePacingMode::Shared => Some(self.resources.byte_pacer().ok_or_else(|| {
                    attempts.push(AttemptEvidence::response(
                        attempt,
                        status,
                        content_length,
                        admission_wait,
                        AttemptOutcome::PacingRejected,
                    ));
                    AcquireError::Pacing {
                        url: source.url.as_url().clone(),
                        kind: PacingError::Unavailable,
                        attempts: attempts.clone(),
                        resume: resume.clone(),
                    }
                })?),
            };
            let copy = match copy_response_body(
                response.body_mut().as_reader(),
                temp.as_file_mut(),
                source.policy.max_bytes,
                initial_bytes,
                active_pacer,
            ) {
                Ok(copy) => copy,
                Err(BodyCopyError::Transport(message)) => {
                    let will_retry = attempt < source.policy.retry.max_retries;
                    let planned_delay =
                        will_retry.then(|| retry_delay(source.policy.retry, attempt));
                    attempts.push(
                        AttemptEvidence::response(
                            attempt,
                            status,
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
                Err(BodyCopyError::LimitExceeded { max, actual }) => {
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
                Err(BodyCopyError::Pacing(kind)) => {
                    attempts.push(AttemptEvidence::response(
                        attempt,
                        status,
                        content_length,
                        admission_wait,
                        AttemptOutcome::PacingRejected,
                    ));
                    return Err(AcquireError::Pacing {
                        url: source.url.as_url().clone(),
                        kind,
                        attempts,
                        resume,
                    });
                }
                Err(BodyCopyError::Local {
                    action,
                    path,
                    source: io_error,
                }) => {
                    attempts.push(AttemptEvidence::response(
                        attempt,
                        status,
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
            temp.as_file_mut().flush().map_err(|err| {
                AcquireError::local(
                    Some(&source.url),
                    "flush download temp file",
                    temp.path(),
                    err,
                )
            })?;

            temp.persist(&source.destination).map_err(|err| {
                AcquireError::local(
                    Some(&source.url),
                    "persist downloaded file",
                    &source.destination,
                    err.error,
                )
            })?;
            if let Some(resume_context) = append_resume {
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
            return Ok(Acquired::from_acquire(
                node.input,
                LocalMaterial {
                    path: source.destination.clone(),
                    kind: MaterialKind::File,
                },
                AcquireEvidence {
                    url: source.url.into_url(),
                    final_path: source.destination,
                    status,
                    bytes: copy.bytes,
                    content_length,
                    attempts,
                    resume,
                    validator: response_validator,
                },
            ));
        }
        unreachable!("retry loop always returns")
    }
}

#[cfg(feature = "reqwest")]
#[derive(Clone)]
pub struct ReqwestResource {
    client: reqwest::Client,
    delay: AsyncDelay,
    admission: Option<Arc<dyn AsyncAdmission>>,
    byte_pacer: Option<Arc<dyn AsyncBytePacer>>,
}

#[cfg(feature = "reqwest")]
impl Default for ReqwestResource {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
            delay: Arc::new(|duration| Box::pin(tokio::time::sleep(duration))),
            admission: None,
            byte_pacer: None,
        }
    }
}

#[cfg(feature = "reqwest")]
impl ReqwestResource {
    pub fn from_client(client: reqwest::Client) -> Self {
        Self {
            client,
            ..Self::default()
        }
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn with_delay(mut self, delay: AsyncDelay) -> Self {
        self.delay = delay;
        self
    }

    pub fn delay(&self) -> &AsyncDelay {
        &self.delay
    }

    pub fn with_admission(mut self, admission: Arc<dyn AsyncAdmission>) -> Self {
        self.admission = Some(admission);
        self
    }

    pub fn admission(&self) -> Option<&Arc<dyn AsyncAdmission>> {
        self.admission.as_ref()
    }

    pub fn with_byte_pacer(mut self, byte_pacer: Arc<dyn AsyncBytePacer>) -> Self {
        self.byte_pacer = Some(byte_pacer);
        self
    }

    pub fn byte_pacer(&self) -> Option<&Arc<dyn AsyncBytePacer>> {
        self.byte_pacer.as_ref()
    }
}

#[cfg(feature = "reqwest")]
impl std::fmt::Debug for ReqwestResource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReqwestResource")
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "reqwest")]
#[derive(Clone, Debug, Default)]
pub struct ReqwestAcquire<R = ReqwestResource> {
    resources: R,
}

#[cfg(feature = "reqwest")]
impl ReqwestAcquire<ReqwestResource> {
    pub fn new() -> Self {
        Self::with_resource(ReqwestResource::default())
    }

    pub fn with_resource(resources: ReqwestResource) -> Self {
        Self { resources }
    }

    pub fn resources(&self) -> &ReqwestResource {
        &self.resources
    }
}

#[cfg(feature = "reqwest")]
impl<I: 'static> AsyncAcquireNode<Chosen<I, RemoteSource>> for ReqwestAcquire<ReqwestResource> {
    type Material = LocalMaterial;
    type Evidence = AcquireEvidence;
    type Error = AcquireError;
    type Output = Acquired<I, LocalMaterial, AcquireEvidence>;
    type Future<'a>
        = Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + 'a>>
    where
        Self: 'a,
        Chosen<I, RemoteSource>: 'a;

    fn acquire_node_async(&self, node: Chosen<I, RemoteSource>) -> Self::Future<'_> {
        let client = self.resources.client.clone();
        let delay = self.resources.delay.clone();
        let admission = self.resources.admission.clone();
        let byte_pacer = self.resources.byte_pacer.clone();
        Box::pin(async move { acquire_reqwest(client, delay, admission, byte_pacer, node).await })
    }
}

#[cfg(feature = "reqwest")]
async fn acquire_reqwest<I>(
    client: reqwest::Client,
    delay: AsyncDelay,
    admission: Option<Arc<dyn AsyncAdmission>>,
    byte_pacer: Option<Arc<dyn AsyncBytePacer>>,
    node: Chosen<I, RemoteSource>,
) -> Result<Acquired<I, LocalMaterial, AcquireEvidence>, AcquireError> {
    let source = node.source;
    let parent = destination_parent(&source.destination)?;
    std::fs::create_dir_all(&parent).map_err(|err| {
        AcquireError::local(Some(&source.url), "create download parent", &parent, err)
    })?;
    reject_existing_unsafe_destination(&source.destination)?;

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
        let (_admission_permit, admission_wait) =
            admit_async_attempt(admission.as_ref(), &source, attempt, &mut attempts, &resume)
                .await?;
        let mut request = client.get(source.url.as_str());
        for (name, value) in &source.policy.headers {
            request = request.header(name, value);
        }
        if let Some(resume) = &resume_context {
            request = request.header(
                reqwest::header::RANGE,
                format!("bytes={}-", resume.partial_bytes),
            );
            if let Some(validator) = &resume.validator {
                request = request.header(reqwest::header::IF_RANGE, validator.if_range_value());
            }
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
            let content_range = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            content_range
                .as_deref()
                .and_then(|value| parse_content_range(value, resume_context.partial_bytes))
                .ok_or_else(|| AcquireError::Protocol {
                    url: source.url.as_url().clone(),
                    kind: ProtocolError::InvalidContentRange {
                        expected_start: resume_context.partial_bytes,
                        header: content_range,
                    },
                    attempts: attempts.clone(),
                    resume: resume.clone(),
                })?;
            Some(resume_context.clone())
        } else {
            if let Some(resume_context) = resume_context {
                resume = Some(resume_context.into_evidence(ResumeOutcome::RangeIgnoredRestarted));
            }
            None
        };

        let mut stage = if let Some(resume_context) = &append_resume {
            StagedDownload::<Open>::from_partial(&parent, &resume_context.partial_path).await?
        } else {
            StagedDownload::<Open>::new_in(&parent)?
        };
        let active_pacer = match source.policy.byte_pacing {
            BytePacingMode::Unbounded => None,
            BytePacingMode::Shared => Some(byte_pacer.as_ref().ok_or_else(|| {
                attempts.push(AttemptEvidence::transfer(
                    attempt,
                    status,
                    stage.bytes,
                    content_length,
                    admission_wait,
                    AttemptOutcome::PacingRejected,
                ));
                AcquireError::Pacing {
                    url: source.url.as_url().clone(),
                    kind: PacingError::Unavailable,
                    attempts: attempts.clone(),
                    resume: resume.clone(),
                }
            })?),
        };
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
                Err(StageWriteError::Pacing(kind)) => {
                    attempts.push(
                        AttemptEvidence::transfer(
                            attempt,
                            status,
                            stage.bytes,
                            content_length,
                            admission_wait,
                            AttemptOutcome::PacingRejected,
                        )
                        .with_pacing_wait(stage.pacing_wait),
                    );
                    return Err(AcquireError::Pacing {
                        url: source.url.as_url().clone(),
                        kind,
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
        let pacing_wait = stage.pacing_wait;
        let stage = stage.finish().await?;
        stage.persist(&source.destination)?;
        if let Some(resume_context) = append_resume {
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
        return Ok(Acquired::from_acquire(
            node.input,
            LocalMaterial {
                path: source.destination.clone(),
                kind: MaterialKind::File,
            },
            AcquireEvidence {
                url: source.url.into_url(),
                final_path: source.destination,
                status,
                bytes,
                content_length,
                attempts,
                resume,
                validator: response_validator,
            },
        ));
    }
    unreachable!("retry loop always returns")
}

#[cfg(any(feature = "reqwest", feature = "ureq"))]
#[allow(
    clippy::result_large_err,
    reason = "AcquireError intentionally carries complete retry and resume evidence"
)]
fn destination_parent(destination: &Path) -> Result<PathBuf, AcquireError> {
    match destination.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent.to_path_buf()),
        Some(_) | None => std::env::current_dir().map_err(|err| {
            AcquireError::local(None, "resolve current directory", Path::new("."), err)
        }),
    }
}

#[cfg(any(feature = "reqwest", feature = "ureq"))]
#[allow(
    clippy::result_large_err,
    reason = "AcquireError intentionally carries complete retry and resume evidence"
)]
fn reject_existing_unsafe_destination(destination: &Path) -> Result<(), AcquireError> {
    match std::fs::symlink_metadata(destination) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_symlink() || !file_type.is_file() {
                let kind = if file_type.is_symlink() {
                    UnsafeDestination::Symlink
                } else {
                    UnsafeDestination::NonFile
                };
                return Err(AcquireError::UnsafeDestination {
                    path: destination.to_path_buf(),
                    kind,
                });
            }
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(AcquireError::local(
            None,
            "read download destination metadata",
            destination,
            err,
        )),
    }
}

#[cfg(any(feature = "reqwest", feature = "ureq"))]
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

#[cfg(feature = "ureq")]
#[allow(
    clippy::result_large_err,
    reason = "AcquireError intentionally carries complete retry and resume evidence"
)]
fn admit_sync_attempt(
    admission: Option<&Arc<dyn SyncAdmission>>,
    source: &RemoteSource,
    attempt: u32,
    attempts: &mut Vec<AttemptEvidence>,
    resume: &Option<ResumeEvidence>,
) -> Result<(Option<AdmissionPermit>, Option<Duration>), AcquireError> {
    match source.policy.admission {
        AdmissionMode::Unbounded => Ok((None, None)),
        AdmissionMode::Shared => {
            let admission = admission.ok_or_else(|| {
                admission_error(
                    source,
                    attempt,
                    attempts,
                    resume,
                    AdmissionError::Unavailable,
                )
            })?;
            match admission.enter() {
                Ok(permit) => {
                    let waited = permit.waited_for();
                    Ok((Some(permit), Some(waited)))
                }
                Err(kind) => Err(admission_error(source, attempt, attempts, resume, kind)),
            }
        }
    }
}

#[cfg(feature = "reqwest")]
async fn admit_async_attempt(
    admission: Option<&Arc<dyn AsyncAdmission>>,
    source: &RemoteSource,
    attempt: u32,
    attempts: &mut Vec<AttemptEvidence>,
    resume: &Option<ResumeEvidence>,
) -> Result<(Option<AdmissionPermit>, Option<Duration>), AcquireError> {
    match source.policy.admission {
        AdmissionMode::Unbounded => Ok((None, None)),
        AdmissionMode::Shared => {
            let admission = admission.ok_or_else(|| {
                admission_error(
                    source,
                    attempt,
                    attempts,
                    resume,
                    AdmissionError::Unavailable,
                )
            })?;
            match admission.enter().await {
                Ok(permit) => {
                    let waited = permit.waited_for();
                    Ok((Some(permit), Some(waited)))
                }
                Err(kind) => Err(admission_error(source, attempt, attempts, resume, kind)),
            }
        }
    }
}

#[cfg(any(feature = "reqwest", feature = "ureq"))]
fn admission_error(
    source: &RemoteSource,
    attempt: u32,
    attempts: &mut Vec<AttemptEvidence>,
    resume: &Option<ResumeEvidence>,
    kind: AdmissionError,
) -> AcquireError {
    attempts.push(AttemptEvidence::new(
        attempt,
        AttemptOutcome::AdmissionRejected,
    ));
    AcquireError::Admission {
        url: source.url.as_url().clone(),
        kind,
        attempts: attempts.clone(),
        resume: resume.clone(),
    }
}

fn retry_delay(policy: RetryPolicy, retry_index: u32) -> Duration {
    let delay = policy
        .base_delay
        .saturating_mul(2_u32.saturating_pow(retry_index));
    policy.max_delay.map_or(delay, |max| delay.min(max))
}

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

fn should_retry_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()
        .and_then(|retry_at| retry_at.duration_since(now).ok())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PlannedResume {
    partial_path: PathBuf,
    partial_bytes: u64,
    validator: Option<Validator>,
}

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

fn planned_resume(policy: &ResumePolicy) -> Option<PlannedResume> {
    let (partial_path, validator) = match &policy.mode {
        ResumeMode::RestartOnly => return None,
        ResumeMode::Unvalidated { partial_path } => (partial_path.clone(), None),
        ResumeMode::IfRange {
            partial_path,
            validator,
        } => (partial_path.clone(), Some(validator.clone())),
    };
    let partial_bytes = std::fs::metadata(&partial_path).ok()?.len();
    (partial_bytes > 0).then_some(PlannedResume {
        partial_path,
        partial_bytes,
        validator,
    })
}

fn parse_strong_etag(value: &str) -> Option<String> {
    let value = value.trim();
    if value.starts_with("W/") || value.starts_with("w/") {
        return None;
    }
    (value.len() >= 2 && value.starts_with('"') && value.ends_with('"')).then(|| value.to_string())
}

fn selected_response_validator(
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Option<Validator> {
    etag.and_then(Validator::strong_etag).or_else(|| {
        last_modified
            .and_then(|value| httpdate::parse_http_date(value).ok())
            .map(Validator::LastModified)
    })
}

fn parse_content_range(value: &str, expected_start: u64) -> Option<(u64, Option<u64>)> {
    let range = value.trim().strip_prefix("bytes ")?;
    let (span, total) = range.split_once('/')?;
    let (start, end) = span.split_once('-')?;
    let start = start.parse::<u64>().ok()?;
    let end = end.parse::<u64>().ok()?;
    if start != expected_start || end < start {
        return None;
    }
    let total = if total == "*" {
        None
    } else {
        Some(total.parse::<u64>().ok()?)
    };
    if let Some(total) = total
        && (end >= total || expected_start > total)
    {
        return None;
    }
    Some((end, total))
}

#[cfg(feature = "ureq")]
struct BodyCopyProgress {
    bytes: u64,
    pacing_wait: Duration,
}

#[cfg(feature = "ureq")]
enum BodyCopyError {
    Transport(String),
    LimitExceeded {
        max: u64,
        actual: u64,
    },
    Pacing(PacingError),
    Local {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

#[cfg(feature = "ureq")]
fn copy_response_body(
    mut reader: impl Read,
    writer: &mut impl Write,
    max_bytes: Option<u64>,
    initial_bytes: u64,
    pacer: Option<&Arc<dyn SyncBytePacer>>,
) -> Result<BodyCopyProgress, BodyCopyError> {
    let mut buffer = [0; 16 * 1024];
    let mut bytes = initial_bytes;
    let mut pacing_wait = Duration::ZERO;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|err| BodyCopyError::Transport(err.to_string()))?;
        if read == 0 {
            return Ok(BodyCopyProgress { bytes, pacing_wait });
        }
        let actual = bytes.saturating_add(read as u64);
        if let Some(max) = max_bytes
            && actual > max
        {
            return Err(BodyCopyError::LimitExceeded { max, actual });
        }
        if let Some(pacer) = pacer {
            pacing_wait += pacer
                .before_chunk(read as u64)
                .map_err(BodyCopyError::Pacing)?
                .waited_for();
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|err| BodyCopyError::Local {
                action: "write download temp file",
                path: PathBuf::from("<temp>"),
                source: err,
            })?;
        bytes = actual;
    }
}

#[cfg(feature = "reqwest")]
enum StageWriteError {
    LimitExceeded {
        max: u64,
        actual: u64,
    },
    Pacing(PacingError),
    Local {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

#[cfg(feature = "reqwest")]
struct Open {
    file: tokio::fs::File,
}

#[cfg(feature = "reqwest")]
struct Closed;

#[cfg(feature = "reqwest")]
struct StagedDownload<State> {
    temp: tempfile::NamedTempFile,
    bytes: u64,
    pacing_wait: Duration,
    writer: State,
}

#[cfg(feature = "reqwest")]
impl StagedDownload<Open> {
    #[allow(
        clippy::result_large_err,
        reason = "AcquireError intentionally carries complete retry and resume evidence"
    )]
    fn new_in(parent: &Path) -> Result<Self, AcquireError> {
        let temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|err| AcquireError::local(None, "create download temp file", parent, err))?;
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

    async fn from_partial(parent: &Path, partial_path: &Path) -> Result<Self, AcquireError> {
        let temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|err| AcquireError::local(None, "create download temp file", parent, err))?;
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
        pacer: Option<&Arc<dyn AsyncBytePacer>>,
    ) -> Result<(), StageWriteError> {
        let actual = self.bytes.saturating_add(chunk.len() as u64);
        if let Some(max) = max_bytes
            && actual > max
        {
            return Err(StageWriteError::LimitExceeded { max, actual });
        }
        if let Some(pacer) = pacer {
            self.pacing_wait += pacer
                .before_chunk(chunk.len() as u64)
                .await
                .map_err(StageWriteError::Pacing)?
                .waited_for();
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

#[cfg(feature = "reqwest")]
impl StagedDownload<Closed> {
    #[allow(
        clippy::result_large_err,
        reason = "AcquireError intentionally carries complete retry and resume evidence"
    )]
    fn persist(self, destination: &Path) -> Result<(), AcquireError> {
        self.temp.persist(destination).map_err(|err| {
            AcquireError::local(None, "persist downloaded file", destination, err.error)
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "ureq")]
    use crate::AcquireNode;
    #[cfg(feature = "reqwest")]
    use crate::AsyncAcquireNode;
    #[cfg(all(
        any(feature = "reqwest", feature = "ureq"),
        feature = "hash",
        feature = "blake3"
    ))]
    use crate::{
        ApplyNode, Blake3, CreateOrReplace, DigestNeed, HashVerify, IdentityPrepare, LocalApply,
        PrepareNode, VerifyNode,
    };
    use crate::{Intent, Item, LocalTarget};
    use std::io::Write as TestWrite;
    use std::io::{BufRead, BufReader};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc as TestArc, Mutex, mpsc};
    use std::thread;

    #[cfg(feature = "ureq")]
    struct TestSyncAdmission {
        waited: Duration,
        error: Option<AdmissionError>,
        enters: TestArc<Mutex<u32>>,
    }

    #[cfg(feature = "ureq")]
    impl SyncAdmission for TestSyncAdmission {
        fn enter(&self) -> Result<AdmissionPermit, AdmissionError> {
            *self.enters.lock().unwrap() += 1;
            match self.error.clone() {
                Some(error) => Err(error),
                None => Ok(AdmissionPermit::waited(self.waited)),
            }
        }
    }

    #[cfg(feature = "reqwest")]
    struct TestAsyncAdmission {
        waited: Duration,
        error: Option<AdmissionError>,
        enters: TestArc<Mutex<u32>>,
    }

    #[cfg(feature = "reqwest")]
    impl AsyncAdmission for TestAsyncAdmission {
        fn enter(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<AdmissionPermit, AdmissionError>> + Send + '_>>
        {
            Box::pin(async move {
                *self.enters.lock().unwrap() += 1;
                match self.error.clone() {
                    Some(error) => Err(error),
                    None => Ok(AdmissionPermit::waited(self.waited)),
                }
            })
        }
    }

    #[cfg(feature = "ureq")]
    struct TestSyncBytePacer {
        waited: Duration,
        error: Option<PacingError>,
        enters: TestArc<Mutex<u32>>,
        bytes: TestArc<Mutex<Vec<u64>>>,
    }

    #[cfg(feature = "ureq")]
    impl SyncBytePacer for TestSyncBytePacer {
        fn before_chunk(&self, bytes: u64) -> Result<BytePacingPermit, PacingError> {
            *self.enters.lock().unwrap() += 1;
            self.bytes.lock().unwrap().push(bytes);
            match self.error.clone() {
                Some(error) => Err(error),
                None => Ok(BytePacingPermit::waited(self.waited)),
            }
        }
    }

    #[cfg(feature = "reqwest")]
    struct TestAsyncBytePacer {
        waited: Duration,
        error: Option<PacingError>,
        enters: TestArc<Mutex<u32>>,
        bytes: TestArc<Mutex<Vec<u64>>>,
    }

    #[cfg(feature = "reqwest")]
    impl AsyncBytePacer for TestAsyncBytePacer {
        fn before_chunk(
            &self,
            bytes: u64,
        ) -> Pin<Box<dyn Future<Output = Result<BytePacingPermit, PacingError>> + Send + '_>>
        {
            Box::pin(async move {
                *self.enters.lock().unwrap() += 1;
                self.bytes.lock().unwrap().push(bytes);
                match self.error.clone() {
                    Some(error) => Err(error),
                    None => Ok(BytePacingPermit::waited(self.waited)),
                }
            })
        }
    }

    #[test]
    fn remote_url_accepts_http_https() {
        assert_eq!(
            RemoteUrl::parse("http://example.com/file")
                .unwrap()
                .as_str(),
            "http://example.com/file"
        );
        assert_eq!(
            RemoteUrl::parse("https://example.com/file")
                .unwrap()
                .as_str(),
            "https://example.com/file"
        );
    }

    #[test]
    fn remote_url_rejects_unsupported_or_relative_urls() {
        assert!(matches!(
            RemoteUrl::parse("file:///tmp/file"),
            Err(AcquireError::UnsupportedScheme { .. })
        ));
        assert!(matches!(
            RemoteUrl::parse("example.com/file"),
            Err(AcquireError::InvalidUrl { .. })
        ));
    }

    #[test]
    fn pulith_error_wraps_net_acquire_error_as_source() {
        let net = AcquireError::UnsupportedScheme {
            scheme: "file".to_string(),
        };
        let error = crate::PulithError::from(net);

        assert!(matches!(error, crate::PulithError::NetAcquire(_)));
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn remote_source_preserves_destination_and_default_policy() {
        let source = RemoteSource::new(
            RemoteUrl::parse("https://example.com/file").unwrap(),
            "artifact.bin",
        );
        assert_eq!(source.destination, PathBuf::from("artifact.bin"));
        assert_eq!(source.policy, AcquirePolicy::default());
    }

    #[test]
    fn module_short_names_replace_net_prefix() {
        let policy = AcquirePolicy::default()
            .retry(RetryPolicy::disabled())
            .resume(ResumePolicy::restart_only())
            .shared_admission();
        assert_eq!(policy.admission, AdmissionMode::Shared);

        let attempt = AttemptEvidence::new(0, AttemptOutcome::AdmissionRejected);
        assert_eq!(attempt.status, None);
    }

    #[test]
    fn net_admission_defaults_to_unbounded_and_can_be_shared() {
        assert_eq!(AcquirePolicy::default().admission, AdmissionMode::Unbounded);
        assert_eq!(
            AcquirePolicy::default().shared_admission().admission,
            AdmissionMode::Shared
        );
    }

    #[test]
    fn attempt_rate_preserves_rate_and_burst() {
        let rate = AttemptRate::new(NonZeroU32::new(20).unwrap(), NonZeroU32::new(4).unwrap());

        assert_eq!(rate.attempts_per_second().get(), 20);
        assert_eq!(rate.burst_attempts().get(), 4);

        let root_rate =
            crate::AttemptRate::new(NonZeroU32::new(10).unwrap(), NonZeroU32::new(2).unwrap());
        let admission = crate::RateAdmission::new(root_rate);
        assert_eq!(admission.rate(), root_rate);
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn sync_rate_admission_shares_attempt_budget() {
        let admission = RateAdmission::new(AttemptRate::new(
            NonZeroU32::new(20).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ));

        let first = SyncAdmission::enter(&admission).unwrap();
        let second = SyncAdmission::enter(&admission).unwrap();

        assert_eq!(first.waited_for(), Duration::ZERO);
        assert!(second.waited_for() > Duration::ZERO);
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn async_rate_admission_shares_attempt_budget() {
        block_on_reqwest(async {
            let admission = RateAdmission::new(AttemptRate::new(
                NonZeroU32::new(20).unwrap(),
                NonZeroU32::new(1).unwrap(),
            ));

            let first = AsyncAdmission::enter(&admission).await.unwrap();
            let second = AsyncAdmission::enter(&admission).await.unwrap();

            assert_eq!(first.waited_for(), Duration::ZERO);
            assert!(second.waited_for() > Duration::ZERO);
        });
    }

    #[test]
    fn byte_rate_preserves_rate_and_burst() {
        let rate = ByteRate::new(
            std::num::NonZeroU32::new(1_024).unwrap(),
            std::num::NonZeroU32::new(4_096).unwrap(),
        );

        assert_eq!(rate.bytes_per_second().get(), 1_024);
        assert_eq!(rate.burst_bytes().get(), 4_096);

        let root_rate = crate::ByteRate::new(
            NonZeroU32::new(2_048).unwrap(),
            NonZeroU32::new(8_192).unwrap(),
        );
        let root_pacer = crate::ByteRatePacer::new(root_rate);
        assert_eq!(root_pacer.rate(), root_rate);
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn sync_byte_rate_pacer_zero_bytes_is_immediate() {
        let pacer = ByteRatePacer::new(ByteRate::new(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ));

        assert_eq!(
            SyncBytePacer::before_chunk(&pacer, 0).unwrap().waited_for(),
            Duration::ZERO,
        );
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn sync_byte_rate_pacer_splits_chunks_larger_than_burst() {
        let pacer = ByteRatePacer::new(ByteRate::new(
            NonZeroU32::new(1_000).unwrap(),
            NonZeroU32::new(2).unwrap(),
        ));

        let permit = SyncBytePacer::before_chunk(&pacer, 3).unwrap();

        assert!(permit.waited_for() > Duration::ZERO);
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn sync_byte_rate_pacer_shares_budget_across_calls() {
        let pacer = ByteRatePacer::new(ByteRate::new(
            NonZeroU32::new(1_000).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ));

        let first = SyncBytePacer::before_chunk(&pacer, 1).unwrap();
        let second = SyncBytePacer::before_chunk(&pacer, 1).unwrap();

        assert_eq!(first.waited_for(), Duration::ZERO);
        assert!(second.waited_for() > Duration::ZERO);
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn async_byte_rate_pacer_zero_bytes_is_immediate() {
        block_on_reqwest(async {
            let pacer = ByteRatePacer::new(ByteRate::new(
                NonZeroU32::new(1).unwrap(),
                NonZeroU32::new(1).unwrap(),
            ));

            assert_eq!(
                AsyncBytePacer::before_chunk(&pacer, 0)
                    .await
                    .unwrap()
                    .waited_for(),
                Duration::ZERO,
            );
        });
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn async_byte_rate_pacer_splits_chunks_larger_than_burst() {
        block_on_reqwest(async {
            let pacer = ByteRatePacer::new(ByteRate::new(
                NonZeroU32::new(1_000).unwrap(),
                NonZeroU32::new(2).unwrap(),
            ));

            let permit = AsyncBytePacer::before_chunk(&pacer, 3).await.unwrap();

            assert!(permit.waited_for() > Duration::ZERO);
        });
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn async_byte_rate_pacer_shares_budget_across_calls() {
        block_on_reqwest(async {
            let pacer = ByteRatePacer::new(ByteRate::new(
                NonZeroU32::new(1_000).unwrap(),
                NonZeroU32::new(1).unwrap(),
            ));

            let first = AsyncBytePacer::before_chunk(&pacer, 1).await.unwrap();
            let second = AsyncBytePacer::before_chunk(&pacer, 1).await.unwrap();

            assert_eq!(first.waited_for(), Duration::ZERO);
            assert!(second.waited_for() > Duration::ZERO);
        });
    }

    #[test]
    fn attempt_evidence_constructors_encode_default_absence() {
        let attempt = AttemptEvidence::response(
            2,
            503,
            Some(10),
            Some(Duration::from_millis(3)),
            AttemptOutcome::RetryableStatus,
        )
        .with_retry_after(Some(Duration::from_secs(1)))
        .with_planned_delay(Some(Duration::from_secs(2)));

        assert_eq!(attempt.attempt, 2);
        assert_eq!(attempt.status, Some(503));
        assert_eq!(attempt.bytes, 0);
        assert_eq!(attempt.content_length, Some(10));
        assert_eq!(attempt.retry_after, Some(Duration::from_secs(1)));
        assert_eq!(attempt.planned_delay, Some(Duration::from_secs(2)));
        assert_eq!(attempt.admission_wait, Some(Duration::from_millis(3)));
        assert_eq!(attempt.outcome, AttemptOutcome::RetryableStatus);

        assert_eq!(
            AttemptEvidence::new(0, AttemptOutcome::AdmissionRejected),
            AttemptEvidence {
                attempt: 0,
                status: None,
                bytes: 0,
                content_length: None,
                retry_after: None,
                planned_delay: None,
                admission_wait: None,
                pacing_wait: Duration::ZERO,
                outcome: AttemptOutcome::AdmissionRejected,
            }
        );
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
    fn resume_policy_defaults_to_restart_only() {
        assert_eq!(
            AcquirePolicy::default().resume,
            ResumePolicy::restart_only()
        );
    }

    #[test]
    fn resume_policy_modes_encode_restart_unvalidated_and_if_range() {
        let partial = PathBuf::from("artifact.part");
        let validator = Validator::Etag("\"abc\"".to_string());

        assert_eq!(
            ResumePolicy::unvalidated(&partial).mode,
            ResumeMode::Unvalidated {
                partial_path: partial.clone()
            }
        );
        assert_eq!(
            ResumePolicy::if_range(&partial, validator.clone()).mode,
            ResumeMode::IfRange {
                partial_path: partial,
                validator
            }
        );
    }

    #[test]
    fn strong_etag_parser_rejects_weak_etag() {
        assert_eq!(
            Validator::strong_etag("\"abc\""),
            Some(Validator::Etag("\"abc\"".to_string()))
        );
        assert_eq!(Validator::strong_etag("W/\"abc\""), None);
    }

    #[test]
    fn selected_response_validator_prefers_strong_etag_over_last_modified() {
        let validator =
            selected_response_validator(Some("\"abc\""), Some("Wed, 21 Oct 2015 07:28:00 GMT"));
        assert_eq!(validator, Some(Validator::Etag("\"abc\"".to_string())));
        assert!(matches!(
            selected_response_validator(None, Some("Wed, 21 Oct 2015 07:28:00 GMT")),
            Some(Validator::LastModified(_))
        ));
    }

    #[test]
    fn content_range_requires_expected_resume_start() {
        assert_eq!(parse_content_range("bytes 5-9/10", 5), Some((9, Some(10))));
        assert_eq!(parse_content_range("bytes 4-9/10", 5), None);
        assert_eq!(parse_content_range("bytes 5-9/*", 5), Some((9, None)));
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_acquire_downloads_file_to_local_material() {
        let body = b"downloaded bytes";
        let server = serve_once(200, body, &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination);
        let intent = Intent::new(
            Item::new("artifact"),
            LocalTarget::new(temp.path().join("out")),
        );
        let chosen = crate::Chosen {
            input: intent,
            source,
        };

        let acquired = UreqAcquire::new().acquire_node(chosen).unwrap();
        server.join();

        assert_eq!(acquired.material.kind, MaterialKind::File);
        assert_eq!(acquired.material.path, destination);
        assert_eq!(std::fs::read(&acquired.material.path).unwrap(), body);
        assert_eq!(acquired.evidence.status, 200);
        assert_eq!(acquired.evidence.bytes, body.len() as u64);
        assert_eq!(acquired.evidence.content_length, Some(body.len() as u64));
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_acquire_rejects_non_success_status_without_touching_destination() {
        let server = serve_once(404, b"not found", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        std::fs::write(&destination, b"old").unwrap();
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination);
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let error = UreqAcquire::new().acquire_node(chosen).unwrap_err();
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
    #[cfg(feature = "ureq")]
    fn ureq_acquire_enforces_max_bytes_before_persist() {
        let server = serve_once(200, b"too large", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let policy = AcquirePolicy::default().max_bytes(3);
        let source =
            RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination).policy(policy);
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let error = UreqAcquire::new().acquire_node(chosen).unwrap_err();
        server.join();

        assert!(matches!(error, AcquireError::LimitExceeded { max: 3, .. }));
        assert!(!destination.exists());
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_max_bytes_rejects_before_byte_pacing() {
        let enters = TestArc::new(Mutex::new(0));
        let paced_bytes = TestArc::new(Mutex::new(Vec::new()));
        let server = serve_once(200, b"too large", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
            .policy(AcquirePolicy::default().max_bytes(3).shared_byte_pacing());
        let resources = UreqResource::default().with_byte_pacer(TestArc::new(TestSyncBytePacer {
            waited: Duration::from_millis(5),
            error: None,
            enters: TestArc::clone(&enters),
            bytes: TestArc::clone(&paced_bytes),
        }));
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let error = UreqAcquire::with_resource(resources)
            .acquire_node(chosen)
            .unwrap_err();
        server.join();

        assert!(matches!(error, AcquireError::LimitExceeded { max: 3, .. }));
        assert_eq!(*enters.lock().unwrap(), 0);
        assert!(paced_bytes.lock().unwrap().is_empty());
        assert!(!destination.exists());
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_retries_retryable_status_and_records_attempts() {
        let server = serve_sequence(vec![
            (503, b"busy", &[("Retry-After", "2")]),
            (200, b"ok", &[]),
        ]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let policy =
            AcquirePolicy::default().retry(RetryPolicy::exponential(1, Duration::from_millis(10)));
        let source =
            RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination).policy(policy);
        let sleeps = TestArc::new(Mutex::new(Vec::new()));
        let resources = UreqResource::default().with_delay({
            let sleeps = TestArc::clone(&sleeps);
            TestArc::new(move |duration| sleeps.lock().unwrap().push(duration))
        });
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let acquired = UreqAcquire::with_resource(resources)
            .acquire_node(chosen)
            .unwrap();
        server.join();

        assert_eq!(std::fs::read(&destination).unwrap(), b"ok");
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
    #[cfg(feature = "ureq")]
    fn ureq_shared_byte_pacing_records_wait_on_body_copy() {
        let waited = Duration::from_millis(5);
        let enters = TestArc::new(Mutex::new(0));
        let paced_bytes = TestArc::new(Mutex::new(Vec::new()));
        let server = serve_once(200, b"paced body", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let policy = AcquirePolicy::default().shared_byte_pacing();
        let source =
            RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination).policy(policy);
        let resources = UreqResource::default().with_byte_pacer(TestArc::new(TestSyncBytePacer {
            waited,
            error: None,
            enters: TestArc::clone(&enters),
            bytes: TestArc::clone(&paced_bytes),
        }));
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let acquired = UreqAcquire::with_resource(resources)
            .acquire_node(chosen)
            .unwrap();
        server.join();

        assert_eq!(std::fs::read(&destination).unwrap(), b"paced body");
        assert_eq!(*enters.lock().unwrap(), 1);
        assert_eq!(
            *paced_bytes.lock().unwrap(),
            vec![b"paced body".len() as u64]
        );
        assert_eq!(acquired.evidence.attempts[0].pacing_wait, waited);
        assert_eq!(
            acquired.evidence.attempts[0].outcome,
            AttemptOutcome::Success
        );
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_concrete_byte_rate_pacer_downloads() {
        let server = serve_once(200, b"concrete paced", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
            .policy(AcquirePolicy::default().shared_byte_pacing());
        let pacer = ByteRatePacer::new(ByteRate::new(
            NonZeroU32::new(1_000_000).unwrap(),
            NonZeroU32::new(16_384).unwrap(),
        ));
        let resources = UreqResource::default().with_byte_pacer(TestArc::new(pacer));
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let acquired = UreqAcquire::with_resource(resources)
            .acquire_node(chosen)
            .unwrap();
        server.join();

        assert_eq!(std::fs::read(&destination).unwrap(), b"concrete paced");
        assert_eq!(acquired.evidence.attempts[0].pacing_wait, Duration::ZERO);
        assert_eq!(
            acquired.evidence.attempts[0].outcome,
            AttemptOutcome::Success
        );
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_shared_byte_pacing_unavailable_records_rejection_without_persist() {
        let server = serve_once(200, b"unpaced body", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
            .policy(AcquirePolicy::default().shared_byte_pacing());
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let error = UreqAcquire::new().acquire_node(chosen).unwrap_err();
        server.join();

        let attempts = match error {
            AcquireError::Pacing {
                kind: PacingError::Unavailable,
                attempts,
                ..
            } => attempts,
            other => panic!("expected pacing unavailable, got {other:?}"),
        };
        assert!(!destination.exists());
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].status, Some(200));
        assert_eq!(attempts[0].bytes, 0);
        assert_eq!(attempts[0].pacing_wait, Duration::ZERO);
        assert_eq!(attempts[0].outcome, AttemptOutcome::PacingRejected);
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_byte_pacing_rejection_records_pacing_rejected_without_persist() {
        let enters = TestArc::new(Mutex::new(0));
        let paced_bytes = TestArc::new(Mutex::new(Vec::new()));
        let server = serve_once(200, b"reject me", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
            .policy(AcquirePolicy::default().shared_byte_pacing());
        let resources = UreqResource::default().with_byte_pacer(TestArc::new(TestSyncBytePacer {
            waited: Duration::from_millis(7),
            error: Some(PacingError::Rejected),
            enters: TestArc::clone(&enters),
            bytes: TestArc::clone(&paced_bytes),
        }));
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let error = UreqAcquire::with_resource(resources)
            .acquire_node(chosen)
            .unwrap_err();
        server.join();

        let attempts = match error {
            AcquireError::Pacing {
                kind: PacingError::Rejected,
                attempts,
                ..
            } => attempts,
            other => panic!("expected pacing rejected, got {other:?}"),
        };
        assert_eq!(*enters.lock().unwrap(), 1);
        assert_eq!(
            *paced_bytes.lock().unwrap(),
            vec![b"reject me".len() as u64]
        );
        assert!(!destination.exists());
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].outcome, AttemptOutcome::PacingRejected);
        assert_eq!(attempts[0].pacing_wait, Duration::ZERO);
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_shared_admission_records_wait_on_attempt() {
        let waited = Duration::from_millis(7);
        let server = serve_once(200, b"admitted", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let policy = AcquirePolicy::default().shared_admission();
        let source =
            RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination).policy(policy);
        let resources = UreqResource::default().with_admission(TestArc::new(TestSyncAdmission {
            waited,
            error: None,
            enters: TestArc::new(Mutex::new(0)),
        }));
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let acquired = UreqAcquire::with_resource(resources)
            .acquire_node(chosen)
            .unwrap();
        server.join();

        assert_eq!(acquired.evidence.attempts[0].admission_wait, Some(waited));
        assert_eq!(
            acquired.evidence.attempts[0].outcome,
            AttemptOutcome::Success
        );
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_concrete_rate_admission_downloads() {
        let server = serve_once(200, b"rate admitted", &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
            .policy(AcquirePolicy::default().shared_admission());
        let admission = RateAdmission::new(AttemptRate::new(
            NonZeroU32::new(1_000).unwrap(),
            NonZeroU32::new(1).unwrap(),
        ));
        let resources = UreqResource::default().with_admission(TestArc::new(admission));
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let acquired = UreqAcquire::with_resource(resources)
            .acquire_node(chosen)
            .unwrap();
        server.join();

        assert_eq!(std::fs::read(&destination).unwrap(), b"rate admitted");
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
    #[cfg(feature = "ureq")]
    fn ureq_shared_admission_rejection_fails_before_request() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let source = RemoteSource::new(
            RemoteUrl::parse("http://127.0.0.1:9/artifact.bin").unwrap(),
            &destination,
        )
        .policy(AcquirePolicy::default().shared_admission());
        let resources = UreqResource::default().with_admission(TestArc::new(TestSyncAdmission {
            waited: Duration::ZERO,
            error: Some(AdmissionError::Rejected),
            enters: TestArc::new(Mutex::new(0)),
        }));
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let error = UreqAcquire::with_resource(resources)
            .acquire_node(chosen)
            .unwrap_err();

        assert!(matches!(
            error,
            AcquireError::Admission {
                kind: AdmissionError::Rejected,
                ..
            }
        ));
        assert!(!destination.exists());
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_retry_enters_shared_admission_per_attempt() {
        let enters = TestArc::new(Mutex::new(0));
        let server = serve_sequence(vec![(503, b"busy", &[]), (200, b"ok", &[])]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let policy = AcquirePolicy::default()
            .shared_admission()
            .retry(RetryPolicy::exponential(1, Duration::from_millis(1)));
        let source =
            RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination).policy(policy);
        let resources = UreqResource::default()
            .with_delay(TestArc::new(|_| {}))
            .with_admission(TestArc::new(TestSyncAdmission {
                waited: Duration::from_millis(3),
                error: None,
                enters: TestArc::clone(&enters),
            }));
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let acquired = UreqAcquire::with_resource(resources)
            .acquire_node(chosen)
            .unwrap();
        server.join();

        assert_eq!(*enters.lock().unwrap(), 2);
        assert_eq!(acquired.evidence.attempts.len(), 2);
        assert_eq!(
            acquired
                .evidence
                .attempts
                .iter()
                .map(|attempt| attempt.admission_wait)
                .collect::<Vec<_>>(),
            vec![
                Some(Duration::from_millis(3)),
                Some(Duration::from_millis(3))
            ]
        );
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_resume_206_appends_after_valid_content_range() {
        let server = serve_sequence(vec![(
            206,
            b" world",
            &[("Content-Range", "bytes 5-10/11")],
        )]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let policy = AcquirePolicy::default().resume(ResumePolicy::unvalidated(&partial));
        let source =
            RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination).policy(policy);
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let acquired = UreqAcquire::new().acquire_node(chosen).unwrap();
        server.join();

        assert_eq!(std::fs::read(&destination).unwrap(), b"hello world");
        assert_eq!(acquired.evidence.bytes, 11);
        assert_eq!(
            acquired.evidence.resume.unwrap().outcome,
            ResumeOutcome::PartialAppended
        );
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_resume_200_to_range_restarts_full_with_fresh_stage() {
        let server = serve_sequence(vec![(200, b"fresh", &[])]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"stale").unwrap();
        let policy = AcquirePolicy::default().resume(ResumePolicy::unvalidated(&partial));
        let source =
            RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination).policy(policy);
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let acquired = UreqAcquire::new().acquire_node(chosen).unwrap();
        server.join();

        assert_eq!(std::fs::read(&destination).unwrap(), b"fresh");
        assert_eq!(
            acquired.evidence.resume.unwrap().outcome,
            ResumeOutcome::RangeIgnoredRestarted
        );
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_resume_missing_content_range_rejects_without_persist() {
        let server = serve_sequence(vec![(206, b" world", &[])]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let policy = AcquirePolicy::default().resume(ResumePolicy::unvalidated(&partial));
        let source =
            RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination).policy(policy);
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let error = UreqAcquire::new().acquire_node(chosen).unwrap_err();
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
    #[cfg(feature = "ureq")]
    fn ureq_if_range_resume_sends_range_and_if_range_and_appends_206() {
        let server = serve_sequence(vec![(
            206,
            b" world",
            &[("Content-Range", "bytes 5-10/11"), ("ETag", "\"next\"")],
        )]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let validator = Validator::Etag("\"abc\"".to_string());
        let policy =
            AcquirePolicy::default().resume(ResumePolicy::if_range(&partial, validator.clone()));
        let source =
            RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination).policy(policy);
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let acquired = UreqAcquire::new().acquire_node(chosen).unwrap();
        let request = server.next_request();
        server.join();

        assert!(request_has_header(&request, "range", "bytes=5-"));
        assert!(request_has_header(&request, "if-range", "\"abc\""));
        assert_eq!(std::fs::read(&destination).unwrap(), b"hello world");
        let resume = acquired.evidence.resume.unwrap();
        assert_eq!(resume.outcome, ResumeOutcome::PartialAppended);
        assert_eq!(resume.validator, Some(validator));
        assert_eq!(
            acquired.evidence.validator,
            Some(Validator::Etag("\"next\"".to_string()))
        );
    }

    #[test]
    #[cfg(feature = "ureq")]
    fn ureq_unvalidated_resume_sends_range_without_if_range() {
        let server = serve_sequence(vec![(
            206,
            b" world",
            &[("Content-Range", "bytes 5-10/11")],
        )]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = temp.path().join("artifact.part");
        std::fs::write(&partial, b"hello").unwrap();
        let policy = AcquirePolicy::default().resume(ResumePolicy::unvalidated(&partial));
        let source =
            RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination).policy(policy);
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let acquired = UreqAcquire::new().acquire_node(chosen).unwrap();
        let request = server.next_request();
        server.join();

        assert!(request_has_header(&request, "range", "bytes=5-"));
        assert!(!request_has_header_name(&request, "if-range"));
        assert_eq!(std::fs::read(&destination).unwrap(), b"hello world");
        assert_eq!(acquired.evidence.resume.unwrap().validator, None);
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn reqwest_acquire_downloads_file_to_local_material() {
        block_on_reqwest(async {
            let body = b"async downloaded bytes";
            let server = serve_once(200, body, &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination);
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let acquired = ReqwestAcquire::new()
                .acquire_node_async(chosen)
                .await
                .unwrap();
            server.join();

            assert_eq!(acquired.material.kind, MaterialKind::File);
            assert_eq!(acquired.material.path, destination);
            assert_eq!(std::fs::read(&acquired.material.path).unwrap(), body);
            assert_eq!(acquired.evidence.status, 200);
            assert_eq!(acquired.evidence.bytes, body.len() as u64);
            assert_eq!(acquired.evidence.content_length, Some(body.len() as u64));
        });
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn reqwest_acquire_rejects_non_success_status_without_touching_destination() {
        block_on_reqwest(async {
            let server = serve_once(404, b"not found", &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            std::fs::write(&destination, b"old").unwrap();
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination);
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let error = ReqwestAcquire::new()
                .acquire_node_async(chosen)
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
    #[cfg(feature = "reqwest")]
    fn reqwest_acquire_enforces_max_bytes_before_persist() {
        block_on_reqwest(async {
            let server = serve_once(200, b"too large", &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let policy = AcquirePolicy::default().max_bytes(3);
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
                .policy(policy);
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let error = ReqwestAcquire::new()
                .acquire_node_async(chosen)
                .await
                .unwrap_err();
            server.join();

            assert!(matches!(error, AcquireError::LimitExceeded { max: 3, .. }));
            assert!(!destination.exists());
        });
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn reqwest_max_bytes_rejects_before_byte_pacing() {
        block_on_reqwest(async {
            let enters = TestArc::new(Mutex::new(0));
            let paced_bytes = TestArc::new(Mutex::new(Vec::new()));
            let server = serve_once(200, b"too large", &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
                .policy(AcquirePolicy::default().max_bytes(3).shared_byte_pacing());
            let resources =
                ReqwestResource::default().with_byte_pacer(TestArc::new(TestAsyncBytePacer {
                    waited: Duration::from_millis(5),
                    error: None,
                    enters: TestArc::clone(&enters),
                    bytes: TestArc::clone(&paced_bytes),
                }));
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let error = ReqwestAcquire::with_resource(resources)
                .acquire_node_async(chosen)
                .await
                .unwrap_err();
            server.join();

            assert!(matches!(error, AcquireError::LimitExceeded { max: 3, .. }));
            assert_eq!(*enters.lock().unwrap(), 0);
            assert!(paced_bytes.lock().unwrap().is_empty());
            assert!(!destination.exists());
        });
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn reqwest_retries_retryable_status_and_records_attempts() {
        block_on_reqwest(async {
            let server = serve_sequence(vec![
                (503, b"busy", &[("Retry-After", "2")]),
                (200, b"ok", &[]),
            ]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let policy = AcquirePolicy::default()
                .retry(RetryPolicy::exponential(1, Duration::from_millis(10)));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
                .policy(policy);
            let sleeps = TestArc::new(Mutex::new(Vec::new()));
            let resources = ReqwestResource::default().with_delay({
                let sleeps = TestArc::clone(&sleeps);
                TestArc::new(move |duration| {
                    sleeps.lock().unwrap().push(duration);
                    Box::pin(std::future::ready(()))
                })
            });
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let acquired = ReqwestAcquire::with_resource(resources)
                .acquire_node_async(chosen)
                .await
                .unwrap();
            server.join();

            assert_eq!(std::fs::read(&destination).unwrap(), b"ok");
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
    #[cfg(feature = "reqwest")]
    fn reqwest_shared_byte_pacing_records_wait_on_body_copy() {
        block_on_reqwest(async {
            let waited = Duration::from_millis(13);
            let enters = TestArc::new(Mutex::new(0));
            let paced_bytes = TestArc::new(Mutex::new(Vec::new()));
            let server = serve_once(200, b"async paced", &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
                .policy(AcquirePolicy::default().shared_byte_pacing());
            let resources =
                ReqwestResource::default().with_byte_pacer(TestArc::new(TestAsyncBytePacer {
                    waited,
                    error: None,
                    enters: TestArc::clone(&enters),
                    bytes: TestArc::clone(&paced_bytes),
                }));
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let acquired = ReqwestAcquire::with_resource(resources)
                .acquire_node_async(chosen)
                .await
                .unwrap();
            server.join();

            assert_eq!(std::fs::read(&destination).unwrap(), b"async paced");
            assert_eq!(*enters.lock().unwrap(), 1);
            assert_eq!(
                *paced_bytes.lock().unwrap(),
                vec![b"async paced".len() as u64]
            );
            assert_eq!(acquired.evidence.attempts[0].pacing_wait, waited);
            assert_eq!(
                acquired.evidence.attempts[0].outcome,
                AttemptOutcome::Success
            );
        });
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn reqwest_concrete_byte_rate_pacer_downloads() {
        block_on_reqwest(async {
            let server = serve_once(200, b"async concrete paced", &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
                .policy(AcquirePolicy::default().shared_byte_pacing());
            let pacer = ByteRatePacer::new(ByteRate::new(
                NonZeroU32::new(1_000_000).unwrap(),
                NonZeroU32::new(16_384).unwrap(),
            ));
            let resources = ReqwestResource::default().with_byte_pacer(TestArc::new(pacer));
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let acquired = ReqwestAcquire::with_resource(resources)
                .acquire_node_async(chosen)
                .await
                .unwrap();
            server.join();

            assert_eq!(
                std::fs::read(&destination).unwrap(),
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
    #[cfg(feature = "reqwest")]
    fn reqwest_shared_byte_pacing_unavailable_records_rejection_without_persist() {
        block_on_reqwest(async {
            let server = serve_once(200, b"async unpaced", &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
                .policy(AcquirePolicy::default().shared_byte_pacing());
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let error = ReqwestAcquire::new()
                .acquire_node_async(chosen)
                .await
                .unwrap_err();
            server.join();

            let attempts = match error {
                AcquireError::Pacing {
                    kind: PacingError::Unavailable,
                    attempts,
                    ..
                } => attempts,
                other => panic!("expected pacing unavailable, got {other:?}"),
            };
            assert!(!destination.exists());
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].status, Some(200));
            assert_eq!(attempts[0].bytes, 0);
            assert_eq!(attempts[0].pacing_wait, Duration::ZERO);
            assert_eq!(attempts[0].outcome, AttemptOutcome::PacingRejected);
        });
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn reqwest_byte_pacing_rejection_records_pacing_rejected_without_persist() {
        block_on_reqwest(async {
            let enters = TestArc::new(Mutex::new(0));
            let paced_bytes = TestArc::new(Mutex::new(Vec::new()));
            let server = serve_once(200, b"reject me", &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
                .policy(AcquirePolicy::default().shared_byte_pacing());
            let resources =
                ReqwestResource::default().with_byte_pacer(TestArc::new(TestAsyncBytePacer {
                    waited: Duration::from_millis(7),
                    error: Some(PacingError::Rejected),
                    enters: TestArc::clone(&enters),
                    bytes: TestArc::clone(&paced_bytes),
                }));
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let error = ReqwestAcquire::with_resource(resources)
                .acquire_node_async(chosen)
                .await
                .unwrap_err();
            server.join();

            let attempts = match error {
                AcquireError::Pacing {
                    kind: PacingError::Rejected,
                    attempts,
                    ..
                } => attempts,
                other => panic!("expected pacing rejected, got {other:?}"),
            };
            assert_eq!(*enters.lock().unwrap(), 1);
            assert_eq!(
                *paced_bytes.lock().unwrap(),
                vec![b"reject me".len() as u64]
            );
            assert!(!destination.exists());
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].outcome, AttemptOutcome::PacingRejected);
            assert_eq!(attempts[0].pacing_wait, Duration::ZERO);
        });
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn reqwest_shared_admission_records_wait_on_attempt() {
        block_on_reqwest(async {
            let waited = Duration::from_millis(11);
            let server = serve_once(200, b"async admitted", &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
                .policy(AcquirePolicy::default().shared_admission());
            let resources =
                ReqwestResource::default().with_admission(TestArc::new(TestAsyncAdmission {
                    waited,
                    error: None,
                    enters: TestArc::new(Mutex::new(0)),
                }));
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let acquired = ReqwestAcquire::with_resource(resources)
                .acquire_node_async(chosen)
                .await
                .unwrap();
            server.join();

            assert_eq!(acquired.evidence.attempts[0].admission_wait, Some(waited));
            assert_eq!(
                acquired.evidence.attempts[0].outcome,
                AttemptOutcome::Success
            );
        });
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn reqwest_concrete_rate_admission_downloads() {
        block_on_reqwest(async {
            let server = serve_once(200, b"async rate admitted", &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
                .policy(AcquirePolicy::default().shared_admission());
            let admission = RateAdmission::new(AttemptRate::new(
                NonZeroU32::new(1_000).unwrap(),
                NonZeroU32::new(1).unwrap(),
            ));
            let resources = ReqwestResource::default().with_admission(TestArc::new(admission));
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let acquired = ReqwestAcquire::with_resource(resources)
                .acquire_node_async(chosen)
                .await
                .unwrap();
            server.join();

            assert_eq!(std::fs::read(&destination).unwrap(), b"async rate admitted");
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
    #[cfg(feature = "reqwest")]
    fn reqwest_shared_admission_rejection_fails_before_request() {
        block_on_reqwest(async {
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let source = RemoteSource::new(
                RemoteUrl::parse("http://127.0.0.1:9/artifact.bin").unwrap(),
                &destination,
            )
            .policy(AcquirePolicy::default().shared_admission());
            let resources =
                ReqwestResource::default().with_admission(TestArc::new(TestAsyncAdmission {
                    waited: Duration::ZERO,
                    error: Some(AdmissionError::Rejected),
                    enters: TestArc::new(Mutex::new(0)),
                }));
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let error = ReqwestAcquire::with_resource(resources)
                .acquire_node_async(chosen)
                .await
                .unwrap_err();

            assert!(matches!(
                error,
                AcquireError::Admission {
                    kind: AdmissionError::Rejected,
                    ..
                }
            ));
            assert!(!destination.exists());
        });
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn reqwest_resume_206_appends_after_valid_content_range() {
        block_on_reqwest(async {
            let server = serve_sequence(vec![(
                206,
                b" world",
                &[("Content-Range", "bytes 5-10/11")],
            )]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let partial = temp.path().join("artifact.part");
            std::fs::write(&partial, b"hello").unwrap();
            let policy = AcquirePolicy::default().resume(ResumePolicy::unvalidated(&partial));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
                .policy(policy);
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let acquired = ReqwestAcquire::new()
                .acquire_node_async(chosen)
                .await
                .unwrap();
            server.join();

            assert_eq!(std::fs::read(&destination).unwrap(), b"hello world");
            assert_eq!(acquired.evidence.bytes, 11);
            assert_eq!(
                acquired.evidence.resume.unwrap().outcome,
                ResumeOutcome::PartialAppended
            );
        });
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn reqwest_if_range_resume_sends_range_and_if_range_and_appends_206() {
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
            let validator = Validator::Etag("\"abc\"".to_string());
            let policy = AcquirePolicy::default()
                .resume(ResumePolicy::if_range(&partial, validator.clone()));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
                .policy(policy);
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let acquired = ReqwestAcquire::new()
                .acquire_node_async(chosen)
                .await
                .unwrap();
            let request = server.next_request();
            server.join();

            assert!(request_has_header(&request, "range", "bytes=5-"));
            assert!(request_has_header(&request, "if-range", "\"abc\""));
            assert_eq!(std::fs::read(&destination).unwrap(), b"hello world");
            let resume = acquired.evidence.resume.unwrap();
            assert_eq!(resume.outcome, ResumeOutcome::PartialAppended);
            assert_eq!(resume.validator, Some(validator));
            assert_eq!(
                acquired.evidence.validator,
                Some(Validator::Etag("\"next\"".to_string()))
            );
        });
    }

    #[test]
    #[cfg(feature = "reqwest")]
    fn reqwest_resume_416_restarts_once_without_persisting_partial() {
        block_on_reqwest(async {
            let server = serve_sequence(vec![(416, b"", &[]), (200, b"fresh", &[])]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let partial = temp.path().join("artifact.part");
            std::fs::write(&partial, b"stale partial").unwrap();
            let policy = AcquirePolicy::default().resume(ResumePolicy::unvalidated(&partial));
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination)
                .policy(policy);
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let acquired = ReqwestAcquire::new()
                .acquire_node_async(chosen)
                .await
                .unwrap();
            server.join();

            assert_eq!(std::fs::read(&destination).unwrap(), b"fresh");
            assert_eq!(
                acquired.evidence.resume.unwrap().outcome,
                ResumeOutcome::RangeUnsatisfiableRestarted
            );
        });
    }

    #[test]
    #[cfg(all(feature = "reqwest", feature = "hash", feature = "blake3"))]
    fn reqwest_acquire_flows_into_hash_verify() {
        block_on_reqwest(async {
            let body = b"reqwest verified bytes";
            let expected = blake3::hash(body).to_hex().to_string();
            let server = serve_once(200, body, &[]);
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("artifact.bin");
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination);
            let chosen = crate::Chosen {
                input: Intent::new(
                    Item::new("artifact"),
                    LocalTarget::new(temp.path().join("out")),
                ),
                source,
            };

            let acquired = ReqwestAcquire::new()
                .acquire_node_async(chosen)
                .await
                .unwrap();
            server.join();
            let verified = HashVerify::<Blake3>::new()
                .verify_node(acquired, DigestNeed::new(expected))
                .unwrap();

            assert_eq!(verified.material.path, destination);
        });
    }

    #[test]
    #[cfg(all(feature = "reqwest", feature = "hash", feature = "blake3"))]
    fn reqwest_acquire_flows_into_local_apply_after_verify() {
        block_on_reqwest(async {
            let body = b"reqwest apply bytes";
            let expected = blake3::hash(body).to_hex().to_string();
            let server = serve_once(200, body, &[]);
            let temp = tempfile::tempdir().unwrap();
            let cache_path = temp.path().join("cache.bin");
            let final_path = temp.path().join("final.bin");
            let intent = Intent::new(Item::new("artifact"), LocalTarget::new(&final_path))
                .op::<CreateOrReplace>();
            let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &cache_path);
            let chosen = crate::Chosen {
                input: intent,
                source,
            };

            let acquired = ReqwestAcquire::new()
                .acquire_node_async(chosen)
                .await
                .unwrap();
            server.join();
            let verified = HashVerify::<Blake3>::new()
                .verify_node(acquired, DigestNeed::new(expected))
                .unwrap();
            let prepared = IdentityPrepare
                .prepare_node(verified, crate::Identity)
                .unwrap();
            let applied = LocalApply::<CreateOrReplace>::default()
                .apply_node(prepared)
                .unwrap();

            assert_eq!(std::fs::read(final_path).unwrap(), body);
            assert_eq!(applied.evidence.current.files, 1);
        });
    }

    #[test]
    #[cfg(all(feature = "ureq", feature = "hash", feature = "blake3"))]
    fn net_acquire_flows_into_hash_verify() {
        let body = b"verified bytes";
        let expected = blake3::hash(body).to_hex().to_string();
        let server = serve_once(200, body, &[]);
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &destination);
        let chosen = crate::Chosen {
            input: Intent::new(
                Item::new("artifact"),
                LocalTarget::new(temp.path().join("out")),
            ),
            source,
        };

        let acquired = UreqAcquire::new().acquire_node(chosen).unwrap();
        server.join();
        let verified = HashVerify::<Blake3>::new()
            .verify_node(acquired, DigestNeed::new(expected))
            .unwrap();

        assert_eq!(verified.material.path, destination);
    }

    #[test]
    #[cfg(all(feature = "ureq", feature = "hash", feature = "blake3"))]
    fn net_acquire_flows_into_local_apply_after_verify() {
        let body = b"apply bytes";
        let expected = blake3::hash(body).to_hex().to_string();
        let server = serve_once(200, body, &[]);
        let temp = tempfile::tempdir().unwrap();
        let cache_path = temp.path().join("cache.bin");
        let final_path = temp.path().join("final.bin");
        let intent = Intent::new(Item::new("artifact"), LocalTarget::new(&final_path))
            .op::<CreateOrReplace>();
        let source = RemoteSource::new(RemoteUrl::parse(&server.url).unwrap(), &cache_path);
        let chosen = crate::Chosen {
            input: intent,
            source,
        };

        let acquired = UreqAcquire::new().acquire_node(chosen).unwrap();
        server.join();
        let verified = HashVerify::<Blake3>::new()
            .verify_node(acquired, DigestNeed::new(expected))
            .unwrap();
        let prepared = IdentityPrepare
            .prepare_node(verified, crate::Identity)
            .unwrap();
        let applied = LocalApply::<CreateOrReplace>::default()
            .apply_node(prepared)
            .unwrap();

        assert_eq!(std::fs::read(final_path).unwrap(), body);
        assert_eq!(applied.evidence.current.files, 1);
    }

    struct TestServer {
        url: String,
        handle: thread::JoinHandle<()>,
        requests: mpsc::Receiver<String>,
    }

    impl TestServer {
        fn next_request(&self) -> String {
            self.requests.recv().unwrap()
        }

        fn join(self) {
            self.handle.join().unwrap();
        }
    }

    #[cfg(feature = "reqwest")]
    fn block_on_reqwest<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn serve_once(
        status: u16,
        body: &'static [u8],
        headers: &'static [(&'static str, &'static str)],
    ) -> TestServer {
        serve_sequence(vec![(status, body, headers)])
    }

    fn request_has_header(request: &str, name: &str, value: &str) -> bool {
        request.lines().any(|line| {
            line.split_once(':')
                .is_some_and(|(actual_name, actual_value)| {
                    actual_name.eq_ignore_ascii_case(name) && actual_value.trim() == value
                })
        })
    }

    #[cfg(feature = "ureq")]
    fn request_has_header_name(request: &str, name: &str) -> bool {
        request.lines().any(|line| {
            line.split_once(':')
                .is_some_and(|(actual_name, _)| actual_name.eq_ignore_ascii_case(name))
        })
    }

    type TestResponse = (u16, &'static [u8], &'static [(&'static str, &'static str)]);

    fn serve_sequence(responses: Vec<TestResponse>) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (requests, request_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            for (status, body, headers) in responses {
                let (stream, _) = listener.accept().unwrap();
                handle_request(stream, status, body, headers, &requests);
            }
        });
        TestServer {
            url: format!("http://{addr}/artifact.bin"),
            handle,
            requests: request_rx,
        }
    }

    fn handle_request(
        mut stream: TcpStream,
        status: u16,
        body: &[u8],
        headers: &[(&str, &str)],
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
