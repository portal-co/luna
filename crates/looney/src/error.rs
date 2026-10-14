//! Error type shared by both compiler frontends.

use std::fmt;

/// A compilation failure. `line`/`col` are 1-based source positions when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub message: String,
    pub line: Option<u32>,
    pub col: Option<u32>,
}

impl CompileError {
    pub fn new(message: impl Into<String>) -> Self {
        CompileError {
            message: message.into(),
            line: None,
            col: None,
        }
    }

    pub fn at(line: u32, col: u32, message: impl Into<String>) -> Self {
        CompileError {
            message: message.into(),
            line: Some(line),
            col: Some(col),
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.col) {
            (Some(line), Some(col)) => write!(f, "{line}:{col}: {}", self.message),
            _ => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for CompileError {}
