use std::error::Error;
use std::fmt;

/// Phase of an integration test where a failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AeronPhase {
    Probe,
    GraphStart,
    Send,
    Recv,
    Shutdown,
    Assert,
    Wire,
}

impl fmt::Display for AeronPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AeronPhase::Probe => write!(f, "Probe"),
            AeronPhase::GraphStart => write!(f, "GraphStart"),
            AeronPhase::Send => write!(f, "Send"),
            AeronPhase::Recv => write!(f, "Recv"),
            AeronPhase::Shutdown => write!(f, "Shutdown"),
            AeronPhase::Assert => write!(f, "Assert"),
            AeronPhase::Wire => write!(f, "Wire"),
        }
    }
}

/// Structured integration-test failure with Aeron context.
#[derive(Debug, Clone)]
pub struct AeronTestError {
    pub phase: AeronPhase,
    pub scenario: String,
    pub stream_id: Option<i32>,
    pub channel_uri: Option<String>,
    pub detail: String,
    pub ingress_avail: Option<usize>,
    pub expected_count: Option<usize>,
    pub egress_occupied: Option<usize>,
}

impl AeronTestError {
    pub fn new(phase: AeronPhase, scenario: &str, detail: impl Into<String>) -> Self {
        Self {
            phase,
            scenario: scenario.to_string(),
            stream_id: None,
            channel_uri: None,
            detail: detail.into(),
            ingress_avail: None,
            expected_count: None,
            egress_occupied: None,
        }
    }

    pub fn stream_id(mut self, id: i32) -> Self {
        self.stream_id = Some(id);
        self
    }

    pub fn channel_uri(mut self, uri: impl Into<String>) -> Self {
        self.channel_uri = Some(uri.into());
        self
    }

    pub fn recv_counts(mut self, expected: usize, avail: usize) -> Self {
        self.expected_count = Some(expected);
        self.ingress_avail = Some(avail);
        self
    }

    pub fn egress_occupied(mut self, occupied: usize) -> Self {
        self.egress_occupied = Some(occupied);
        self
    }
}

impl fmt::Display for AeronTestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] phase={}", self.scenario, self.phase)?;
        if let Some(id) = self.stream_id {
            write!(f, " stream_id={id}")?;
        }
        if let Some(uri) = &self.channel_uri {
            write!(f, " channel={uri}")?;
        }
        if let (Some(exp), Some(avail)) = (self.expected_count, self.ingress_avail) {
            write!(f, " expected_messages={exp} ingress_avail={avail}")?;
        }
        if let Some(egress) = self.egress_occupied {
            write!(f, " egress_occupied={egress}")?;
        }
        write!(f, ": {}", self.detail)?;
        write!(
            f,
            "\n  Troubleshooting: same OS user as aeronmd; check /dev/shm/aeron-default; \
             restart aeronmd; run with RUST_LOG=info; see core/tests/README.md"
        )
    }
}

impl Error for AeronTestError {}

pub type AeronResult<T> = Result<T, AeronTestError>;
