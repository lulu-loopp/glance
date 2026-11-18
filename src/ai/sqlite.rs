//! A SQLite database read through the copy of SQLite Windows itself carries (`winsqlite3.dll`, in System32 since
//! Windows 10), loaded as it is first wanted: what other tools keep in one is read without Glance carrying SQLite.

use std::ffi::{c_char, c_int, c_void, CString};
use std::path::Path;
use std::sync::OnceLock;
use windows::core::{s, w, PCSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};

const OK: c_int = 0;
const ROW: c_int = 100;
const INTEGER: c_int = 1;
const NULL: c_int = 5;
const OPEN_READONLY: c_int = 0x1;
#[cfg(test)]
const OPEN_MADE: c_int = 0x2 | 0x4;
/// How long a read waits for a writer that holds the database (ms).
const BUSY: c_int = 2_000;

type Open = unsafe extern "system" fn(*const c_char, *mut *mut c_void, c_int, *const c_char) -> c_int;
type Handle = unsafe extern "system" fn(*mut c_void) -> c_int;
type Busy = unsafe extern "system" fn(*mut c_void, c_int) -> c_int;
type Prepare = unsafe extern "system" fn(*mut c_void, *const c_char, c_int, *mut *mut c_void, *mut *const c_char) -> c_int;
type Column = unsafe extern "system" fn(*mut c_void, c_int) -> c_int;
type ColumnInt = unsafe extern "system" fn(*mut c_void, c_int) -> i64;
type ColumnText = unsafe extern "system" fn(*mut c_void, c_int) -> *const u8;

/// SQLite's functions used here.
struct Library {
    open: Open,
    close: Handle,
    busy: Busy,
    prepare: Prepare,
    step: Handle,
    finalize: Handle,
    column_count: Handle,
    column_type: Column,
    column_int: ColumnInt,
    column_text: ColumnText,
    column_bytes: Column,
}

/// A function `module` exports, as `F`.
unsafe fn export<F>(module: HMODULE, name: PCSTR) -> Option<F> {
    let address = unsafe { GetProcAddress(module, name) }?;
    Some(unsafe { std::mem::transmute_copy(&address) })
}

/// SQLite as Windows carries it, where it does.
fn library() -> Option<&'static Library> {
    static LIBRARY: OnceLock<Option<Library>> = OnceLock::new();
    LIBRARY
        .get_or_init(|| {
            let module = unsafe { LoadLibraryExW(w!("winsqlite3.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()?;
            unsafe {
                Some(Library {
                    open: export(module, s!("sqlite3_open_v2"))?,
                    close: export(module, s!("sqlite3_close_v2"))?,
                    busy: export(module, s!("sqlite3_busy_timeout"))?,
                    prepare: export(module, s!("sqlite3_prepare_v2"))?,
                    step: export(module, s!("sqlite3_step"))?,
                    finalize: export(module, s!("sqlite3_finalize"))?,
                    column_count: export(module, s!("sqlite3_column_count"))?,
                    column_type: export(module, s!("sqlite3_column_type"))?,
                    column_int: export(module, s!("sqlite3_column_int64"))?,
                    column_text: export(module, s!("sqlite3_column_text"))?,
                    column_bytes: export(module, s!("sqlite3_column_bytes"))?,
                })
            }
        })
        .as_ref()
}

/// A database open.
pub(super) struct Database {
    library: &'static Library,
    handle: *mut c_void,
}

/// A value of a row's column.
#[derive(Clone, PartialEq, Debug)]
pub(super) enum Value {
    Null,
    Int(i64),
    Text(String),
}

impl Value {
    pub(super) fn int(&self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(*value),
            Value::Text(text) => text.trim().parse().ok(),
            Value::Null => None,
        }
    }

    /// Its text, where it is any but empty.
    pub(super) fn text(&self) -> Option<String> {
        match self {
            Value::Text(text) if !text.is_empty() => Some(text.clone()),
            Value::Int(value) => Some(value.to_string()),
            _ => None,
        }
    }
}

impl Database {
    /// The database at `path`, opened to be read (none: no SQLite, or not a database to be opened).
    pub(super) fn read(path: &Path) -> Option<Database> {
        Database::open(path, OPEN_READONLY)
    }

    /// The database at `path`, made where it is not, to be written.
    #[cfg(test)]
    pub(super) fn made(path: &Path) -> Option<Database> {
        Database::open(path, OPEN_MADE)
    }

    fn open(path: &Path, flags: c_int) -> Option<Database> {
        let library = library()?;
        let name = CString::new(path.to_str()?).ok()?;
        let mut handle = std::ptr::null_mut();
        let opened = unsafe { (library.open)(name.as_ptr(), &mut handle, flags, std::ptr::null()) };
        // (Closed as it goes, opened or not: SQLite gives a handle to close either way.)
        let database = Database { library, handle };
        if opened != OK {
            return None;
        }
        unsafe { (library.busy)(handle, BUSY) };
        Some(database)
    }

    /// Each row `sql` gives, its columns' values (none: it could not be asked, as of a table or a column the
    /// database does not have).
    pub(super) fn rows(&self, sql: &str) -> Option<Vec<Vec<Value>>> {
        let library = self.library;
        let sql = CString::new(sql).ok()?;
        let mut statement = std::ptr::null_mut();
        let prepared = unsafe { (library.prepare)(self.handle, sql.as_ptr(), -1, &mut statement, std::ptr::null_mut()) };
        if prepared != OK || statement.is_null() {
            return None;
        }
        let width = unsafe { (library.column_count)(statement) };
        let mut rows = Vec::new();
        while unsafe { (library.step)(statement) } == ROW {
            rows.push((0..width).map(|at| unsafe { self.value(statement, at) }).collect());
        }
        unsafe { (library.finalize)(statement) };
        Some(rows)
    }

    /// The value of column `at` of the row `statement` stands on.
    unsafe fn value(&self, statement: *mut c_void, at: c_int) -> Value {
        let library = self.library;
        match unsafe { (library.column_type)(statement, at) } {
            NULL => Value::Null,
            INTEGER => Value::Int(unsafe { (library.column_int)(statement, at) }),
            _ => {
                let text = unsafe { (library.column_text)(statement, at) };
                if text.is_null() {
                    return Value::Null;
                }
                let len = unsafe { (library.column_bytes)(statement, at) }.max(0) as usize;
                Value::Text(String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(text, len) }).into_owned())
            }
        }
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.library.close)(self.handle) };
        }
    }
}
