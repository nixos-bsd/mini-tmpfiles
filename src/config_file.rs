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
    pub data: T,
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
    #[allow(unused)]
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
    pub(crate) fn as_ref(&self) -> Spanned<'a, &T> {
        Spanned {
            data: &self.data,
            file: self.file,
            characters: self.characters.clone(),
        }
    }
}

impl<'a, T, U> Spanned<'a, (T, U)> {
    pub fn unzip(self) -> (Spanned<'a, T>, Spanned<'a, U>) {
        (
            Spanned {
                data: self.data.0,
                file: self.file,
                characters: self.characters.clone(),
            },
            Spanned {
                data: self.data.1,
                file: self.file,
                characters: self.characters,
            },
        )
    }
}

impl<'a, T> Spanned<'a, Option<T>> {
    pub fn try_then<U, E>(
        self,
        closure: impl FnOnce(T) -> Result<Option<U>, E>,
    ) -> Result<Spanned<'a, Option<U>>, E> {
        let data = self.data.map(closure).transpose()?.flatten();
        Ok(Spanned {
            data,
            file: self.file,
            characters: self.characters,
        })
    }
    pub fn opt_map<U>(self, closure: impl FnOnce(T) -> U) -> Spanned<'a, Option<U>> {
        let data = self.data.map(closure);
        Spanned {
            data,
            file: self.file,
            characters: self.characters,
        }
    }
    pub fn try_opt_map<U, E>(
        self,
        closure: impl FnOnce(T) -> Result<U, E>,
    ) -> Result<Spanned<'a, Option<U>>, E> {
        let data = self.data.map(closure).transpose()?;
        Ok(Spanned {
            data,
            file: self.file,
            characters: self.characters,
        })
    }
    pub fn as_opt_deref(&self) -> Spanned<'a, Option<&T::Target>>
    where
        T: Deref,
    {
        self.as_ref().map(Option::as_deref)
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
    pub(crate) age: Spanned<'a, Option<CleanupAge>>,
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
