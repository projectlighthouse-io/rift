//! sqlx errors to [`Error`].

use types::Error;

/// `ER_QUERY_INTERRUPTED`.
const QUERY_INTERRUPTED: &str = "1317";

pub(crate) fn map(e: sqlx::Error) -> Error {
    match e {
        sqlx::Error::Database(db) => {
            let code = db.code().map(|c| c.into_owned());
            if code.as_deref() == Some(QUERY_INTERRUPTED) || db.message().contains("interrupted") {
                Error::Cancelled
            } else {
                Error::Query {
                    message: db.message().to_owned(),
                    code,
                }
            }
        }
        sqlx::Error::PoolTimedOut
        | sqlx::Error::PoolClosed
        | sqlx::Error::Io(_)
        | sqlx::Error::Tls(_) => Error::connect(e.to_string()),
        sqlx::Error::RowNotFound => Error::not_found("row"),
        other => Error::query(other.to_string()),
    }
}
