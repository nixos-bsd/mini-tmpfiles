use std::{ffi::OsString, path::PathBuf, time::Duration};

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum LineAction {
    CreateFile,
    WriteFile,
    CreateAndCleanUpDirectory,
    CreateAndRemoveDirectory,
    CleanUpDirectory,
    CreateFifo,
    CreateSymlink,
    CreateCharDevice,
    CreateBlockDevice,
    Copy,
    Ignore,
    IgnoreNonRecursive,
    Remove,
    RemoveRecursive,
    SetMode,
    SetModeRecursive,
    SetXattr,
    SetXattrRecursive,
    SetAttr,
    SetAttrRecursive,
    SetAcl,
    SetAclRecursive,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct LineType {
    /// Basic action, represented by first character
    pub action: LineAction,
    /// Plus sign modifier, means recreate except for write
    pub recreate: bool,
    /// Exclamation mark modifier, should only be run during boot
    pub boot: bool,
    /// Minus sign modifier, means failure during create will not error
    pub noerror: bool,
    /// Equals sign modifier, remove existing objects if they do not match
    pub force: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum FileOwner {
    Id(u32),
    Name(OsString),
}

#[derive(Debug, PartialEq, Eq, Copy, Clone, Default)]
pub struct CleanupAge {
    /// Minimum age before cleaning up
    pub age: Duration,
    /// Only cleanup directories at the second level and below the root path
    pub second_level: bool,

    /// Consider the atime (last access) as last use for files
    pub consider_atime: bool,
    /// Consider the atime (last access) as last use for directories
    pub consider_atime_dir: bool,

    /// Consider the btime (creation) as last use for files
    pub consider_btime: bool,
    /// Consider the btime (creation) as last use for directories
    pub consider_btime_dir: bool,

    /// Consider the ctime (last status change) as last use for files
    pub consider_ctime: bool,
    /// Consider the ctime (last status change) as last use for directories
    pub consider_ctime_dir: bool,

    /// Consider the mtime (last modification) as last use for files
    pub consider_mtime: bool,
    /// Consider the mtime (last modification) as last use for directories
    pub consider_mtime_dir: bool,
}

impl CleanupAge {
    pub const EMPTY: Self = Self {
        age: Duration::ZERO,
        second_level: false,
        consider_atime: true,
        consider_atime_dir: true,
        consider_btime: true,
        consider_btime_dir: true,
        consider_ctime: true,
        consider_ctime_dir: false,
        consider_mtime: true,
        consider_mtime_dir: true,
    };
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Line {
    pub(crate) line_type: LineType,
    pub(crate) path: PathBuf,
    pub(crate) mode: Option<u32>,
    pub(crate) owner: Option<FileOwner>,
    pub(crate) group: Option<FileOwner>,
    pub(crate) age: CleanupAge,
    pub(crate) argument: Option<OsString>,
}
