//! The HTTP client: one POST per statement, streamed line by line.

use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
use types::{Error, Value};

use crate::values;

/// Format every query is answered in when it has no `FORMAT` clause.
const FORMAT: &str = "JSONCompactEachRowWithNamesAndTypes";

/// Connection settings shared by every request.
#[derive(Clone)]
pub(crate) struct Client {
    pub(crate) http: reqwest::Client,
    pub(crate) url: String,
    pub(crate) user: String,
    pub(crate) password: String,
    pub(crate) database: Option<String>,
    pub(crate) read_only: bool,
}

/// A started query: column names and types, then the remaining rows.
pub(crate) struct Response {
    pub(crate) names: Vec<String>,
    pub(crate) types: Vec<String>,
    pub(crate) rows: BoxStream<'static, Result<Vec<Value>, Error>>,
    pub(crate) written_rows: u64,
}

fn http_error(e: reqwest::Error) -> Error {
    if e.is_connect() || e.is_timeout() {
        Error::connect(e.to_string())
    } else {
        Error::query(e.to_string())
    }
}

/// `Code: 62. DB::Exception: Syntax error … (version …)` → message and code.
fn server_error(body: &str) -> Error {
    let code = body
        .strip_prefix("Code: ")
        .and_then(|r| r.split('.').next())
        .map(str::to_owned);
    if code.as_deref() == Some("394") {
        return Error::Cancelled;
    }
    let message = body.split_once("DB::Exception: ").map_or(body, |(_, m)| m);
    let message = message
        .split(" (version")
        .next()
        .unwrap_or(message)
        .trim()
        .to_owned();
    Error::Query { message, code }
}

/// Splits a chunk stream into lines.
fn lines<S, B>(body: S) -> BoxStream<'static, Result<String, Error>>
where
    S: Stream<Item = reqwest::Result<B>> + Send + 'static,
    B: AsRef<[u8]>,
{
    futures::stream::unfold(
        (Box::pin(body), Vec::<u8>::new(), false),
        |(mut body, mut buf, done)| async move {
            loop {
                if let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    let text = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                    return Some((Ok(text), (body, buf, done)));
                }
                if done {
                    return None;
                }
                match body.next().await {
                    Some(Ok(chunk)) => buf.extend_from_slice(chunk.as_ref()),
                    Some(Err(e)) => return Some((Err(http_error(e)), (body, buf, true))),
                    None => {
                        if buf.is_empty() {
                            return None;
                        }
                        let text = String::from_utf8_lossy(&std::mem::take(&mut buf)).into_owned();
                        return Some((Ok(text), (body, buf, true)));
                    }
                }
            }
        },
    )
    .boxed()
}

fn json_list(line: &str) -> Result<Vec<serde_json::Value>, Error> {
    serde_json::from_str(line).map_err(|e| Error::query(format!("bad response line: {e}")))
}

impl Client {
    fn request(&self, sql: &str, query_id: Option<&str>) -> reqwest::RequestBuilder {
        let mut params: Vec<(&str, String)> = vec![
            ("default_format", FORMAT.to_owned()),
            ("output_format_json_quote_64bit_integers", "0".into()),
            ("output_format_json_quote_decimals", "1".into()),
            ("send_progress_in_http_headers", "0".into()),
        ];
        if let Some(db) = &self.database {
            params.push(("database", db.clone()));
        }
        if let Some(id) = query_id {
            params.push(("query_id", id.to_owned()));
        }
        if self.read_only {
            params.push(("readonly", "2".into()));
        }
        self.http
            .post(&self.url)
            .query(&params)
            .header("X-ClickHouse-User", &self.user)
            .header("X-ClickHouse-Key", &self.password)
            .body(sql.to_owned())
    }

    /// Runs a statement: reads the names and types lines, then streams decoded rows.
    pub(crate) async fn run(&self, sql: &str, query_id: Option<&str>) -> Result<Response, Error> {
        let response = self
            .request(sql, query_id)
            .send()
            .await
            .map_err(http_error)?;
        let written_rows = response
            .headers()
            .get("X-ClickHouse-Summary")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|j| j.get("written_rows")?.as_str()?.parse().ok())
            .unwrap_or(0);
        if !response.status().is_success() {
            let body = response.text().await.map_err(http_error)?;
            return Err(server_error(&body));
        }
        let mut lines = lines(response.bytes_stream());
        let Some(first) = lines.next().await else {
            return Ok(Response {
                names: Vec::new(),
                types: Vec::new(),
                rows: futures::stream::empty().boxed(),
                written_rows,
            });
        };
        let first = first?;
        if first.starts_with("Code: ") {
            return Err(server_error(&first));
        }
        let names: Vec<String> = json_list(&first)?
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_owned())
            .collect();
        let second = lines
            .next()
            .await
            .ok_or_else(|| Error::query("missing types line"))??;
        let types: Vec<String> = json_list(&second)?
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_owned())
            .collect();
        let row_types = types.clone();
        let rows = lines
            .map(move |line| {
                let line = line?;
                // Errors raised mid-stream arrive as a plain text line.
                if line.starts_with("Code: ") {
                    return Err(server_error(&line));
                }
                let cells = json_list(&line)?;
                Ok(cells
                    .iter()
                    .zip(&row_types)
                    .map(|(cell, t)| values::value(t, cell))
                    .collect())
            })
            .boxed();
        Ok(Response {
            names,
            types,
            rows,
            written_rows,
        })
    }

    /// Runs a statement and collects every row.
    pub(crate) async fn values(&self, sql: &str) -> Result<Vec<Vec<Value>>, Error> {
        let response = self.run(sql, None).await?;
        response
            .rows
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect()
    }

    /// Best-effort `KILL QUERY` for a query id.
    pub(crate) async fn kill(&self, query_id: &str) {
        let sql = format!(
            "KILL QUERY WHERE query_id = '{}' ASYNC",
            query_id.replace('\'', "")
        );
        let _ = self.request(&sql, None).send().await;
    }
}

#[cfg(test)]
mod tests {
    use types::Error;

    #[test]
    fn parses_server_errors() {
        let e = super::server_error(
            "Code: 62. DB::Exception: Syntax error: failed at position 1. (SYNTAX_ERROR) (version 24.8.14.39 (official build))",
        );
        assert_eq!(
            e,
            Error::Query {
                message: "Syntax error: failed at position 1. (SYNTAX_ERROR)".into(),
                code: Some("62".into())
            }
        );
        assert_eq!(
            super::server_error("Code: 394. DB::Exception: Query was cancelled."),
            Error::Cancelled
        );
    }
}
