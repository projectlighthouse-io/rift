//! rusqlite errors to [`Error`].

use rusqlite::ErrorCode;
use types::Error;

pub(crate) fn map(e: rusqlite::Error) -> Error {
    match &e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == ErrorCode::OperationInterrupted => {
            Error::Cancelled
        }
        rusqlite::Error::SqliteFailure(f, msg) => Error::Query {
            message: msg.clone().unwrap_or_else(|| e.to_string()),
            code: Some(f.extended_code.to_string()),
        },
        _ => Error::query(e.to_string()),
    }
}
