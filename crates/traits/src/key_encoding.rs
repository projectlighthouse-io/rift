//! A structured key encoding such as the FoundationDB tuple layer.

use types::Error;

/// Turns raw key bytes into readable text and back.
///
/// Implemented by: FoundationDB (tuple layer).
pub trait KeyEncoding: Send + Sync {
    /// Name shown in the "Decode as" picker, e.g. `Tuple`.
    fn name(&self) -> &'static str;
    /// Readable form of `key`, or `None` if it is not in this encoding.
    fn decode(&self, key: &[u8]) -> Option<String>;
    /// Raw bytes for the readable form.
    ///
    /// # Errors
    /// [`Error::Invalid`]-style errors describing what could not be parsed.
    fn encode(&self, text: &str) -> Result<Vec<u8>, Error>;
}
