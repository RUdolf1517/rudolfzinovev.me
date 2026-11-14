//! Safe, ergonomic Rust wrapper over system `libsqlite3`.
//! Configured for WAL mode, foreign keys, and busy_timeout for fast VPS concurrency.

use std::ffi::{CStr, CString, c_char, c_int, c_uchar, c_void};
use std::path::Path;
use std::ptr;

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;
const SQLITE_INTEGER: c_int = 1;
const SQLITE_NULL: c_int = 5;

// SQLITE_TRANSIENT tells SQLite to make its own copy of bound strings/blobs immediately.
const SQLITE_TRANSIENT: isize = -1;

#[link(name = "sqlite3")]
unsafe extern "C" {
    fn sqlite3_open(filename: *const c_char, pp_db: *mut *mut c_void) -> c_int;
    fn sqlite3_close_v2(db: *mut c_void) -> c_int;
    fn sqlite3_errmsg(db: *mut c_void) -> *const c_char;
    fn sqlite3_busy_timeout(db: *mut c_void, ms: c_int) -> c_int;
    fn sqlite3_exec(
        db: *mut c_void,
        sql: *const c_char,
        callback: *const c_void,
        arg: *mut c_void,
        errmsg: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_free(ptr: *mut c_void);
    fn sqlite3_prepare_v2(
        db: *mut c_void,
        z_sql: *const c_char,
        n_byte: c_int,
        pp_stmt: *mut *mut c_void,
        pz_tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_step(stmt: *mut c_void) -> c_int;
    fn sqlite3_finalize(stmt: *mut c_void) -> c_int;
    fn sqlite3_bind_int64(stmt: *mut c_void, idx: c_int, val: i64) -> c_int;
    fn sqlite3_bind_text(
        stmt: *mut c_void,
        idx: c_int,
        val: *const c_char,
        n: c_int,
        destructor: isize,
    ) -> c_int;
    fn sqlite3_bind_null(stmt: *mut c_void, idx: c_int) -> c_int;
    fn sqlite3_column_count(stmt: *mut c_void) -> c_int;
    fn sqlite3_column_type(stmt: *mut c_void, i_col: c_int) -> c_int;
    fn sqlite3_column_int64(stmt: *mut c_void, i_col: c_int) -> i64;
    fn sqlite3_column_text(stmt: *mut c_void, i_col: c_int) -> *const c_uchar;
    fn sqlite3_column_bytes(stmt: *mut c_void, i_col: c_int) -> c_int;
    fn sqlite3_last_insert_rowid(db: *mut c_void) -> i64;
    fn sqlite3_changes(db: *mut c_void) -> c_int;
    fn sqlite3_libversion() -> *const c_char;
}

/// Binding surface mirrors the SQLite C API: `OptText(None)` binds `NULL`.
#[derive(Debug, Clone)]
pub enum SqlParam<'a> {
    Int(i64),
    Text(&'a str),
    #[allow(dead_code)]
    OptText(Option<&'a str>),
}

#[derive(Debug, Clone)]
pub enum SqlVal {
    Null,
    Int(i64),
    Text(String),
}

impl SqlVal {
    pub fn as_i64(&self) -> i64 {
        match self {
            SqlVal::Int(v) => *v,
            SqlVal::Text(s) => s.parse().unwrap_or(0),
            SqlVal::Null => 0,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            SqlVal::Text(s) => s.as_str(),
            _ => "",
        }
    }

}

pub struct Connection {
    raw: *mut c_void,
}

// System libsqlite3 (Debian/Ubuntu) is compiled with SQLITE_THREADSAFE=1
// (serialized mode), so one connection may be used from several threads. Every
// statement below is prepared, bound, stepped and finalized inside a single
// call, never shared across threads.
unsafe impl Send for Connection {}
unsafe impl Sync for Connection {}

impl Connection {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| format!("create db dir: {e}"))?;
        }
        let path_str = path
            .to_str()
            .ok_or_else(|| "invalid UTF-8 db path".to_string())?;
        let c_path = CString::new(path_str).map_err(|e| e.to_string())?;

        let mut raw: *mut c_void = ptr::null_mut();
        let rc = unsafe { sqlite3_open(c_path.as_ptr(), &mut raw) };
        if rc != SQLITE_OK || raw.is_null() {
            let msg = if raw.is_null() {
                "sqlite3_open returned null".to_string()
            } else {
                let err = unsafe { CStr::from_ptr(sqlite3_errmsg(raw)).to_string_lossy().into_owned() };
                unsafe { sqlite3_close_v2(raw) };
                err
            };
            return Err(msg);
        }

        unsafe {
            sqlite3_busy_timeout(raw, 5_000);
        }

        let conn = Self { raw };
        conn.exec_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA cache_size = -8000;",
        )?;
        Ok(conn)
    }

    pub fn sqlite_version() -> String {
        unsafe {
            CStr::from_ptr(sqlite3_libversion())
                .to_string_lossy()
                .into_owned()
        }
    }

    fn last_error(&self) -> String {
        unsafe {
            let p = sqlite3_errmsg(self.raw);
            if p.is_null() {
                "unknown sqlite error".to_string()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }

    pub fn exec_batch(&self, sql: &str) -> Result<(), String> {
        let c_sql = CString::new(sql).map_err(|e| e.to_string())?;
        let mut errmsg: *mut c_char = ptr::null_mut();
        let rc = unsafe {
            sqlite3_exec(
                self.raw,
                c_sql.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                &mut errmsg,
            )
        };
        if rc != SQLITE_OK {
            let msg = if !errmsg.is_null() {
                let s = unsafe { CStr::from_ptr(errmsg).to_string_lossy().into_owned() };
                unsafe { sqlite3_free(errmsg.cast()) };
                s
            } else {
                self.last_error()
            };
            return Err(msg);
        }
        Ok(())
    }

    pub fn execute(&self, sql: &str, params: &[SqlParam<'_>]) -> Result<usize, String> {
        let stmt = self.prepare_and_bind(sql, params)?;
        let rc = unsafe { sqlite3_step(stmt) };
        unsafe { sqlite3_finalize(stmt) };
        if rc != SQLITE_DONE && rc != SQLITE_ROW {
            return Err(format!("sqlite step error ({rc}): {}", self.last_error()));
        }
        let changes = unsafe { sqlite3_changes(self.raw) };
        Ok(changes.max(0) as usize)
    }

    pub fn insert(&self, sql: &str, params: &[SqlParam<'_>]) -> Result<i64, String> {
        self.execute(sql, params)?;
        Ok(unsafe { sqlite3_last_insert_rowid(self.raw) })
    }

    pub fn query_rows(&self, sql: &str, params: &[SqlParam<'_>]) -> Result<Vec<Vec<SqlVal>>, String> {
        let stmt = self.prepare_and_bind(sql, params)?;
        let mut rows = Vec::new();
        loop {
            let rc = unsafe { sqlite3_step(stmt) };
            if rc == SQLITE_ROW {
                let cols = unsafe { sqlite3_column_count(stmt) };
                let mut row = Vec::with_capacity(cols.max(0) as usize);
                for c in 0..cols {
                    let col_type = unsafe { sqlite3_column_type(stmt, c) };
                    let val = match col_type {
                        SQLITE_NULL => SqlVal::Null,
                        SQLITE_INTEGER => SqlVal::Int(unsafe { sqlite3_column_int64(stmt, c) }),
                        _ => {
                            let ptr = unsafe { sqlite3_column_text(stmt, c) };
                            let len = unsafe { sqlite3_column_bytes(stmt, c) };
                            if ptr.is_null() || len <= 0 {
                                SqlVal::Text(String::new())
                            } else {
                                let slice =
                                    unsafe { std::slice::from_raw_parts(ptr, len as usize) };
                                SqlVal::Text(String::from_utf8_lossy(slice).into_owned())
                            }
                        }
                    };
                    row.push(val);
                }
                rows.push(row);
            } else if rc == SQLITE_DONE {
                break;
            } else {
                let err = self.last_error();
                unsafe { sqlite3_finalize(stmt) };
                return Err(format!("sqlite query error ({rc}): {err}"));
            }
        }
        unsafe { sqlite3_finalize(stmt) };
        Ok(rows)
    }

    pub fn query_one(
        &self,
        sql: &str,
        params: &[SqlParam<'_>],
    ) -> Result<Option<Vec<SqlVal>>, String> {
        let mut rows = self.query_rows(sql, params)?;
        if rows.is_empty() {
            Ok(None)
        } else {
            Ok(Some(rows.swap_remove(0)))
        }
    }

    fn prepare_and_bind(&self, sql: &str, params: &[SqlParam<'_>]) -> Result<*mut c_void, String> {
        let mut stmt: *mut c_void = ptr::null_mut();
        let rc = unsafe {
            sqlite3_prepare_v2(
                self.raw,
                sql.as_ptr().cast(),
                sql.len() as c_int,
                &mut stmt,
                ptr::null_mut(),
            )
        };
        if rc != SQLITE_OK || stmt.is_null() {
            return Err(format!(
                "sqlite prepare error ({rc}): {}",
                self.last_error()
            ));
        }

        for (i, param) in params.iter().enumerate() {
            let idx = (i + 1) as c_int;
            let brc = unsafe {
                match param {
                    SqlParam::Int(v) => sqlite3_bind_int64(stmt, idx, *v),
                    SqlParam::Text(s) => sqlite3_bind_text(
                        stmt,
                        idx,
                        s.as_ptr().cast(),
                        s.len() as c_int,
                        SQLITE_TRANSIENT,
                    ),
                    SqlParam::OptText(Some(s)) => sqlite3_bind_text(
                        stmt,
                        idx,
                        s.as_ptr().cast(),
                        s.len() as c_int,
                        SQLITE_TRANSIENT,
                    ),
                    SqlParam::OptText(None) => sqlite3_bind_null(stmt, idx),
                }
            };
            if brc != SQLITE_OK {
                let err = self.last_error();
                unsafe { sqlite3_finalize(stmt) };
                return Err(format!("sqlite bind error at {idx}: {err}"));
            }
        }

        Ok(stmt)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                sqlite3_close_v2(self.raw);
            }
        }
    }
}
