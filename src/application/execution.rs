//! Failure phase is recorded at the boundary, not inferred from error prose.
use serde::Serialize;
#[derive(Debug)]
pub struct BeforeTurn(pub anyhow::Error);
impl std::fmt::Display for BeforeTurn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for BeforeTurn {}
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    BeforeTurn,
    OutcomeUncertain,
}
impl FailureKind {
    pub fn classify(error: &anyhow::Error) -> Self {
        if error.downcast_ref::<BeforeTurn>().is_some() {
            Self::BeforeTurn
        } else {
            Self::OutcomeUncertain
        }
    }
    pub fn state(&self) -> &'static str {
        match self {
            Self::BeforeTurn => "failed",
            Self::OutcomeUncertain => "interrupted",
        }
    }
}
