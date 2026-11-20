//! The shared error type.

/// Errors returned by drivers and infrastructure.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
#[must_use]
pub enum Error {
    /// Could not connect or authenticate.
    #[error("connection failed: {message}")]
    Connect {
        /// Human-readable cause.
        message: String,
    },
    /// The database rejected a statement.
    #[error("{message}")]
    Query {
        /// Database error message.
        message: String,
        /// Vendor error code, e.g. SQLSTATE.
        code: Option<String>,
    },
    /// The connection does not support this operation.
    #[error("not supported: {what}")]
    Unsupported {
        /// The operation.
        what: String,
    },
    /// The operation was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// The operation timed out.
    #[error("timed out")]
    Timeout,
    /// An object or key does not exist.
    #[error("not found: {what}")]
    NotFound {
        /// What was looked up.
        what: String,
    },
    /// Blocked by safe mode or a read-only object.
    #[error("read-only: {reason}")]
    ReadOnly {
        /// Why it is read-only.
        reason: String,
    },
    /// Bad input from the user or caller.
    #[error("invalid input: {message}")]
    Invalid {
        /// What was wrong.
        message: String,
    },
    /// A required native library or external tool is missing.
    #[error("{message}")]
    MissingDependency {
        /// Includes how to install it.
        message: String,
    },
    /// Local I/O or storage failure.
    #[error("i/o error: {message}")]
    Io {
        /// Cause.
        message: String,
    },
}

impl Error {
    /// Shorthand for [`Error::Query`] without a code.
    pub fn query(message: impl Into<String>) -> Self {
        Error::Query {
            message: message.into(),
            code: None,
        }
    }

    /// Shorthand for [`Error::Connect`].
    pub fn connect(message: impl Into<String>) -> Self {
        Error::Connect {
            message: message.into(),
        }
    }

    /// Shorthand for [`Error::Invalid`].
    pub fn invalid(message: impl Into<String>) -> Self {
        Error::Invalid {
            message: message.into(),
        }
    }

    /// Shorthand for [`Error::Unsupported`].
    pub fn unsupported(what: impl Into<String>) -> Self {
        Error::Unsupported { what: what.into() }
    }

    /// Shorthand for [`Error::NotFound`].
    pub fn not_found(what: impl Into<String>) -> Self {
        Error::NotFound { what: what.into() }
    }

    /// Shorthand for [`Error::Io`].
    pub fn io(message: impl Into<String>) -> Self {
        Error::Io {
            message: message.into(),
        }
    }
}
