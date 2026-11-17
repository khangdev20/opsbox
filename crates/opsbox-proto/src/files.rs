//! Types for the Files page (docs/BACKEND_SPEC.md 7.10).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Dir,
    File,
    /// A symbolic link that does not resolve.
    BrokenLink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    /// Absolute path after resolving symbolic links in the directory part.
    pub path: String,
    pub kind: FileKind,
    /// `ls`-style, e.g. `-rw-r--r--` or `drwxr-xr-x`.
    pub mode: String,
    /// Unix seconds.
    pub mtime: i64,
    /// Bytes; `None` for directories.
    pub size: Option<u64>,
    pub is_link: bool,
    /// A hint the page shows next to the name (for example a secrets file anyone can read).
    pub warn: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirListing {
    pub path: String,
    pub entries: Vec<FileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileContent {
    pub path: String,
    pub content: String,
    pub size: u64,
    pub mode: String,
    pub mtime: i64,
}

/// A file copied into the transfer directory for streaming to the browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staged {
    /// Where the copy is.
    pub staging: String,
    /// The name the person should see.
    pub name: String,
    pub size: u64,
}
