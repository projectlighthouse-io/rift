//! Raw `libfdb_c` bindings through `libloading`. The crate's only unsafe code.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;
use std::sync::OnceLock;

use libloading::Library;

use crate::{API_VERSION, FdbError, KeySelector, KeyValue};

type Err = c_int;
type Bool = c_int;
type Ptr = *mut c_void;

/// `FDB_TR_OPTION_TIMEOUT`, milliseconds as a little-endian `i64`.
const TR_OPTION_TIMEOUT: c_int = 500;
/// `FDB_STREAMING_MODE_WANT_ALL` / `FDB_STREAMING_MODE_EXACT`.
const MODE_WANT_ALL: c_int = -2;
const MODE_EXACT: c_int = 0;

/// `FDBKeyValue`, declared under `#pragma pack(4)` in `fdb_c.h`.
#[repr(C, packed(4))]
struct RawKeyValue {
    key: *const u8,
    key_length: c_int,
    value: *const u8,
    value_length: c_int,
}

/// Function pointers copied out of the library, which stays loaded for the process lifetime.
struct Api {
    _lib: Library,
    get_error: unsafe extern "C" fn(Err) -> *const c_char,
    create_database: unsafe extern "C" fn(*const c_char, *mut Ptr) -> Err,
    database_destroy: unsafe extern "C" fn(Ptr),
    create_transaction: unsafe extern "C" fn(Ptr, *mut Ptr) -> Err,
    transaction_destroy: unsafe extern "C" fn(Ptr),
    transaction_set_option: unsafe extern "C" fn(Ptr, c_int, *const u8, c_int) -> Err,
    transaction_get: unsafe extern "C" fn(Ptr, *const u8, c_int, Bool) -> Ptr,
    #[allow(clippy::type_complexity)]
    transaction_get_range: unsafe extern "C" fn(
        Ptr,
        *const u8,
        c_int,
        Bool,
        c_int,
        *const u8,
        c_int,
        Bool,
        c_int,
        c_int,
        c_int,
        c_int,
        c_int,
        Bool,
        Bool,
    ) -> Ptr,
    transaction_set: unsafe extern "C" fn(Ptr, *const u8, c_int, *const u8, c_int),
    transaction_clear: unsafe extern "C" fn(Ptr, *const u8, c_int),
    transaction_commit: unsafe extern "C" fn(Ptr) -> Ptr,
    transaction_on_error: unsafe extern "C" fn(Ptr, Err) -> Ptr,
    transaction_get_read_version: unsafe extern "C" fn(Ptr) -> Ptr,
    future_block_until_ready: unsafe extern "C" fn(Ptr) -> Err,
    future_get_error: unsafe extern "C" fn(Ptr) -> Err,
    future_get_int64: unsafe extern "C" fn(Ptr, *mut i64) -> Err,
    future_get_value: unsafe extern "C" fn(Ptr, *mut Bool, *mut *const u8, *mut c_int) -> Err,
    future_get_keyvalue_array:
        unsafe extern "C" fn(Ptr, *mut *const RawKeyValue, *mut c_int, *mut Bool) -> Err,
    future_destroy: unsafe extern "C" fn(Ptr),
}

fn candidates() -> Vec<String> {
    let mut paths: Vec<String> = std::env::var("RIFT_FDB_LIB").into_iter().collect();
    #[cfg(target_os = "macos")]
    paths.extend([
        "/usr/local/lib/libfdb_c.dylib".into(),
        "libfdb_c.dylib".into(),
    ]);
    #[cfg(target_os = "linux")]
    paths.extend(["libfdb_c.so".into(), "/usr/lib/libfdb_c.so".into()]);
    #[cfg(target_os = "windows")]
    paths.extend([
        "fdb_c.dll".into(),
        r"C:\Program Files\foundationdb\bin\fdb_c.dll".into(),
    ]);
    paths
}

fn missing() -> FdbError {
    FdbError {
        code: 0,
        message: "The FoundationDB client library (libfdb_c) is not installed. Install the \
                  FoundationDB client package from foundationdb.org, or set RIFT_FDB_LIB to \
                  its path."
            .into(),
    }
}

fn load() -> Result<Api, FdbError> {
    // SAFETY: loading libfdb_c runs its initialisers, which have no preconditions.
    let lib = candidates()
        .into_iter()
        .find_map(|path| unsafe { Library::new(path) }.ok())
        .ok_or_else(missing)?;
    macro_rules! sym {
        ($name:literal) => {
            // SAFETY: the symbol has this signature in fdb_c.h for API version 710; the
            // pointer stays valid because `lib` is kept alive in `Api` for the process.
            *unsafe { lib.get(concat!($name, "\0").as_bytes()) }.map_err(|e| FdbError {
                code: 0,
                message: format!("libfdb_c is missing {}: {e}", $name),
            })?
        };
    }
    let select: unsafe extern "C" fn(c_int, c_int) -> Err = sym!("fdb_select_api_version_impl");
    let setup_network: unsafe extern "C" fn() -> Err = sym!("fdb_setup_network");
    let run_network: unsafe extern "C" fn() -> Err = sym!("fdb_run_network");
    let api = Api {
        get_error: sym!("fdb_get_error"),
        create_database: sym!("fdb_create_database"),
        database_destroy: sym!("fdb_database_destroy"),
        create_transaction: sym!("fdb_database_create_transaction"),
        transaction_destroy: sym!("fdb_transaction_destroy"),
        transaction_set_option: sym!("fdb_transaction_set_option"),
        transaction_get: sym!("fdb_transaction_get"),
        transaction_get_range: sym!("fdb_transaction_get_range"),
        transaction_set: sym!("fdb_transaction_set"),
        transaction_clear: sym!("fdb_transaction_clear"),
        transaction_commit: sym!("fdb_transaction_commit"),
        transaction_on_error: sym!("fdb_transaction_on_error"),
        transaction_get_read_version: sym!("fdb_transaction_get_read_version"),
        future_block_until_ready: sym!("fdb_future_block_until_ready"),
        future_get_error: sym!("fdb_future_get_error"),
        future_get_int64: sym!("fdb_future_get_int64"),
        future_get_value: sym!("fdb_future_get_value"),
        future_get_keyvalue_array: sym!("fdb_future_get_keyvalue_array"),
        future_destroy: sym!("fdb_future_destroy"),
        _lib: lib,
    };
    // SAFETY: called once per process (guarded by the `OnceLock` in `api`).
    check(&api, unsafe { select(API_VERSION, API_VERSION) })?;
    // SAFETY: as above; the network must be set up once, after selecting the API version.
    check(&api, unsafe { setup_network() })?;
    std::thread::Builder::new()
        .name("fdb-network".into())
        // SAFETY: runs the network loop set up above; it never returns while the process lives.
        .spawn(move || unsafe {
            run_network();
        })
        .map_err(|e| FdbError {
            code: 0,
            message: format!("starting the FoundationDB network: {e}"),
        })?;
    Ok(api)
}

fn api() -> Result<&'static Api, FdbError> {
    static API: OnceLock<Result<Api, FdbError>> = OnceLock::new();
    API.get_or_init(load).as_ref().map_err(Clone::clone)
}

pub(crate) fn api_loaded() -> Result<(), FdbError> {
    api().map(|_| ())
}

fn check(api: &Api, code: Err) -> Result<(), FdbError> {
    if code == 0 {
        return Ok(());
    }
    // SAFETY: fdb_get_error returns a static, NUL-terminated string for any code.
    let message = unsafe { CStr::from_ptr((api.get_error)(code)) }
        .to_string_lossy()
        .into_owned();
    Err(FdbError { code, message })
}

fn len(bytes: &[u8]) -> Result<c_int, FdbError> {
    c_int::try_from(bytes.len()).map_err(|_| FdbError {
        code: 0,
        message: "key or value too large".into(),
    })
}

/// A ready future; destroyed on drop.
struct Future<'a> {
    api: &'a Api,
    ptr: Ptr,
}

impl<'a> Future<'a> {
    /// Blocks until `ptr` is ready and checks its error.
    fn wait(api: &'a Api, ptr: Ptr) -> Result<Self, FdbError> {
        let future = Future { api, ptr };
        // SAFETY: `ptr` is a live future returned by the library.
        check(api, unsafe { (api.future_block_until_ready)(ptr) })?;
        // SAFETY: as above, and it is ready.
        check(api, unsafe { (api.future_get_error)(ptr) })?;
        Ok(future)
    }
}

impl Drop for Future<'_> {
    fn drop(&mut self) {
        // SAFETY: each future is destroyed exactly once, here.
        unsafe { (self.api.future_destroy)(self.ptr) }
    }
}

pub(crate) fn open(cluster_file: Option<&str>) -> Result<Database, FdbError> {
    let api = api()?;
    let path = cluster_file
        .map(|p| {
            CString::new(p).map_err(|_| FdbError {
                code: 0,
                message: "cluster file path contains NUL".into(),
            })
        })
        .transpose()?;
    let mut db: Ptr = ptr::null_mut();
    // SAFETY: `path` outlives the call; `db` is a valid out-pointer.
    check(api, unsafe {
        (api.create_database)(path.as_ref().map_or(ptr::null(), |p| p.as_ptr()), &mut db)
    })?;
    Ok(Database { api, ptr: db })
}

/// An open database handle. Thread-safe per the FoundationDB client docs.
pub struct Database {
    api: &'static Api,
    ptr: Ptr,
}

// SAFETY: FDBDatabase handles may be used from any thread concurrently.
unsafe impl Send for Database {}
// SAFETY: as above.
unsafe impl Sync for Database {}

impl Drop for Database {
    fn drop(&mut self) {
        // SAFETY: destroyed exactly once.
        unsafe { (self.api.database_destroy)(self.ptr) }
    }
}

impl Database {
    /// A new transaction with a timeout.
    ///
    /// # Errors
    /// When the client cannot create one.
    pub fn transaction(&self, timeout_ms: i64) -> Result<Transaction, FdbError> {
        let mut tr: Ptr = ptr::null_mut();
        // SAFETY: `self.ptr` is live; `tr` is a valid out-pointer.
        check(self.api, unsafe {
            (self.api.create_transaction)(self.ptr, &mut tr)
        })?;
        let tr = Transaction {
            api: self.api,
            ptr: tr,
        };
        let value = timeout_ms.to_le_bytes();
        // SAFETY: `value` outlives the call; TIMEOUT takes an 8-byte little-endian int.
        check(self.api, unsafe {
            (self.api.transaction_set_option)(tr.ptr, TR_OPTION_TIMEOUT, value.as_ptr(), 8)
        })?;
        Ok(tr)
    }

    /// Runs `f` in a transaction, commits, and retries on retryable errors.
    ///
    /// # Errors
    /// The first non-retryable error from `f` or the commit.
    pub fn run<T>(
        &self,
        timeout_ms: i64,
        mut f: impl FnMut(&Transaction) -> Result<T, FdbError>,
    ) -> Result<T, FdbError> {
        let tr = self.transaction(timeout_ms)?;
        loop {
            let result = f(&tr).and_then(|value| tr.commit().map(|()| value));
            match result {
                Ok(value) => return Ok(value),
                Err(e) if e.code != 0 => tr.on_error(e.code)?,
                Err(e) => return Err(e),
            }
        }
    }
}

/// A transaction. Use from one thread at a time.
pub struct Transaction {
    api: &'static Api,
    ptr: Ptr,
}

// SAFETY: FDBTransaction may move between threads as long as calls are not concurrent, which
// `&self` methods on a non-`Sync` type guarantee.
unsafe impl Send for Transaction {}

impl Drop for Transaction {
    fn drop(&mut self) {
        // SAFETY: destroyed exactly once.
        unsafe { (self.api.transaction_destroy)(self.ptr) }
    }
}

impl Transaction {
    /// Reads one value.
    ///
    /// # Errors
    /// Cluster errors.
    pub fn get(&self, key: &[u8], snapshot: bool) -> Result<Option<Vec<u8>>, FdbError> {
        // SAFETY: `key` outlives the call (the library copies it).
        let f = Future::wait(self.api, unsafe {
            (self.api.transaction_get)(self.ptr, key.as_ptr(), len(key)?, Bool::from(snapshot))
        })?;
        let (mut present, mut value, mut length): (Bool, *const u8, c_int) = (0, ptr::null(), 0);
        // SAFETY: `f` is ready; out-pointers are valid; the value lives until `f` drops.
        check(self.api, unsafe {
            (self.api.future_get_value)(f.ptr, &mut present, &mut value, &mut length)
        })?;
        if present == 0 {
            return Ok(None);
        }
        // SAFETY: `value` points at `length` bytes owned by `f`, copied before `f` drops.
        Ok(Some(
            unsafe { std::slice::from_raw_parts(value, length.max(0) as usize) }.to_vec(),
        ))
    }

    /// Reads up to `limit` pairs (0 for no limit) between two selectors; also returns whether
    /// more remain.
    ///
    /// # Errors
    /// Cluster errors.
    pub fn get_range(
        &self,
        begin: &KeySelector,
        end: &KeySelector,
        limit: i32,
        reverse: bool,
        snapshot: bool,
    ) -> Result<(Vec<KeyValue>, bool), FdbError> {
        let mode = if limit > 0 { MODE_EXACT } else { MODE_WANT_ALL };
        // SAFETY: selector keys outlive the call. `x_or_equal = after` with offset 1 encodes
        // firstGreaterThan / firstGreaterOrEqual.
        let f = Future::wait(self.api, unsafe {
            (self.api.transaction_get_range)(
                self.ptr,
                begin.key.as_ptr(),
                len(&begin.key)?,
                Bool::from(begin.after),
                1,
                end.key.as_ptr(),
                len(&end.key)?,
                Bool::from(end.after),
                1,
                limit,
                0,
                mode,
                1,
                Bool::from(snapshot),
                Bool::from(reverse),
            )
        })?;
        let (mut items, mut count, mut more): (*const RawKeyValue, c_int, Bool) =
            (ptr::null(), 0, 0);
        // SAFETY: `f` is ready; out-pointers are valid.
        check(self.api, unsafe {
            (self.api.future_get_keyvalue_array)(f.ptr, &mut items, &mut count, &mut more)
        })?;
        let mut out = Vec::with_capacity(count.max(0) as usize);
        for i in 0..count.max(0) as usize {
            // SAFETY: `items` has `count` entries owned by `f`; fields are read unaligned
            // because the struct is packed, and the bytes are copied before `f` drops.
            unsafe {
                let kv = ptr::read_unaligned(items.add(i));
                out.push(KeyValue {
                    key: std::slice::from_raw_parts(kv.key, kv.key_length.max(0) as usize).to_vec(),
                    value: std::slice::from_raw_parts(kv.value, kv.value_length.max(0) as usize)
                        .to_vec(),
                });
            }
        }
        Ok((out, more != 0))
    }

    /// Writes a value (applied on commit).
    ///
    /// # Errors
    /// When the key or value is too large.
    pub fn set(&self, key: &[u8], value: &[u8]) -> Result<(), FdbError> {
        let (k, v) = (len(key)?, len(value)?);
        // SAFETY: the library copies both buffers.
        unsafe { (self.api.transaction_set)(self.ptr, key.as_ptr(), k, value.as_ptr(), v) };
        Ok(())
    }

    /// Clears a key (applied on commit).
    ///
    /// # Errors
    /// When the key is too large.
    pub fn clear(&self, key: &[u8]) -> Result<(), FdbError> {
        let k = len(key)?;
        // SAFETY: the library copies the key.
        unsafe { (self.api.transaction_clear)(self.ptr, key.as_ptr(), k) };
        Ok(())
    }

    /// The read version this transaction reads at.
    ///
    /// # Errors
    /// Cluster errors.
    pub fn read_version(&self) -> Result<i64, FdbError> {
        // SAFETY: `self.ptr` is live.
        let f = Future::wait(self.api, unsafe {
            (self.api.transaction_get_read_version)(self.ptr)
        })?;
        let mut version = 0;
        // SAFETY: `f` is ready; out-pointer is valid.
        check(self.api, unsafe {
            (self.api.future_get_int64)(f.ptr, &mut version)
        })?;
        Ok(version)
    }

    fn commit(&self) -> Result<(), FdbError> {
        // SAFETY: `self.ptr` is live.
        Future::wait(self.api, unsafe { (self.api.transaction_commit)(self.ptr) }).map(|_| ())
    }

    /// Waits as the client asks after `code`, resetting the transaction for a retry; fails
    /// when `code` is not retryable.
    fn on_error(&self, code: Err) -> Result<(), FdbError> {
        // SAFETY: `self.ptr` is live.
        Future::wait(self.api, unsafe {
            (self.api.transaction_on_error)(self.ptr, code)
        })
        .map(|_| ())
    }
}
