use std::{
    ffi::OsString,
    ops::{Deref, Range},
    path::Path,
    time::Duration,
};

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
    /// Tilde modifier, base64_decode the argument
    pub(crate) base64_decode: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum FileOwner {
    Id(u32),
    Name(String),
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
pub struct Spanned<'a, T> {
    data: T,
    file: &'a Path,
    characters: Range<usize>,
}

impl<'a, T> Spanned<'a, T> {
    pub fn new(data: T, file: &'a Path, characters: Range<usize>) -> Self {
        Self {
            data,
            file,
            characters,
        }
    }
    pub fn map<U>(self, closure: impl FnOnce(T) -> U) -> Spanned<'a, U> {
        Spanned {
            data: closure(self.data),
            file: self.file,
            characters: self.characters,
        }
    }
    pub fn try_map<U, E>(
        self,
        closure: impl FnOnce(T) -> Result<U, E>,
    ) -> Result<Spanned<'a, U>, E> {
        Ok(Spanned {
            data: closure(self.data)?,
            file: self.file,
            characters: self.characters,
        })
    }

    pub(crate) fn as_deref(&self) -> Spanned<'a, &T::Target>
    where
        T: Deref,
    {
        Spanned {
            data: self.data.deref(),
            file: self.file,
            characters: self.characters.clone(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Mode {
    pub(crate) value: u32,
    pub(crate) mode_behavior: ModeBehavior,
}

#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub enum ModeBehavior {
    #[default]
    Default,
    Masked,       // If prefixed with a tilde, mask value with existing mode
    KeepExisting, // If prefixed with a colon, keep existing mode if file exists
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Line<'a> {
    pub(crate) line_type: Spanned<'a, LineType>,
    pub(crate) path: Spanned<'a, SpecifierString>,
    pub(crate) mode: Spanned<'a, Option<Mode>>,
    pub(crate) owner: Spanned<'a, Option<FileOwner>>,
    pub(crate) group: Spanned<'a, Option<FileOwner>>,
    pub(crate) age: Spanned<'a, CleanupAge>,
    pub(crate) argument: Spanned<'a, Option<OsString>>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Specifier {
    Architecture,      //%a
    ImageVersion,      //%A
    BootID,            //%b
    BuildID,           //%B
    CacheDir,          //%C
    UserGroup,         //%g
    UserGID,           //%G
    UserHome,          //%h
    Hostname,          //%H
    ShortHostname,     //%l
    LogDir,            //%L
    MachineID,         //%m
    ImageID,           //%M
    OperatingSystemID, //%o
    StateDir,          //%S
    RuntimeDir,        //%t
    TempDir,           //%T
    Username,          //%u
    UserUID,           //%U
    KernelRelease,     //%v
    PersistentTempDir, //%V
    VersionID,         //%w
    VariantID,         //%W
    PercentSign,       //%%
}

impl Specifier {
    pub fn parse(ch: u8) -> Option<Self> {
        use Specifier::*;
        Some(match char::from(ch) {
            'a' => Architecture,
            'A' => ImageVersion,
            'b' => BootID,
            'B' => BuildID,
            'C' => CacheDir,
            'g' => UserGroup,
            'G' => UserGID,
            'h' => UserHome,
            'H' => Hostname,
            'l' => ShortHostname,
            'L' => LogDir,
            'm' => MachineID,
            'M' => ImageID,
            'o' => OperatingSystemID,
            'S' => StateDir,
            't' => TempDir,
            'T' => RuntimeDir,
            'u' => Username,
            'U' => UserUID,
            'v' => KernelRelease,
            'V' => PersistentTempDir,
            'w' => VersionID,
            'W' => VariantID,
            '%' => PercentSign,
            _ => return None,
        })
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct SpecifierString(pub Vec<u8>, pub Box<[(Specifier, Vec<u8>)]>);
