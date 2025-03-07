use core::str;
use std::{
    borrow::Cow,
    ffi::{OsStr, OsString},
    fmt::Display,
    ops::{Deref, Range},
    os::unix::ffi::OsStrExt,
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

impl FileOwner {
    pub(crate) fn as_uid(&self) -> eyre::Result<u32> {
        match self {
            FileOwner::Id(x) => Ok(*x),
            FileOwner::Name(s) => Ok(uzers::get_user_by_name(s)
                .ok_or_else(|| eyre::eyre!("User {s} not found"))?
                .uid()),
        }
    }

    pub(crate) fn as_uname(&self) -> eyre::Result<Cow<str>> {
        match self {
            FileOwner::Id(x) => Ok(Cow::Owned(
                uzers::get_user_by_uid(*x)
                    .ok_or_else(|| eyre::eyre!("User {x} not found"))?
                    .name()
                    .to_str()
                    .ok_or_else(|| eyre::eyre!("User {x} is not valid utf-8"))?
                    .to_owned(),
            )),
            FileOwner::Name(s) => Ok(Cow::Borrowed(s)),
        }
    }

    pub(crate) fn as_gid(&self) -> eyre::Result<u32> {
        match self {
            FileOwner::Id(x) => Ok(*x),
            FileOwner::Name(s) => Ok(uzers::get_group_by_name(s)
                .ok_or_else(|| eyre::eyre!("Group {s} not found"))?
                .gid()),
        }
    }

    pub(crate) fn as_gname(&self) -> eyre::Result<Cow<str>> {
        match self {
            FileOwner::Id(x) => Ok(Cow::Owned(
                uzers::get_group_by_gid(*x)
                    .ok_or_else(|| eyre::eyre!("Group {x} not found"))?
                    .name()
                    .to_str()
                    .ok_or_else(|| eyre::eyre!("Group {x} is not valid utf-8"))?
                    .to_owned(),
            )),
            FileOwner::Name(s) => Ok(Cow::Borrowed(s)),
        }
    }
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
    pub file: &'a Path,
    pub characters: Range<usize>,
}

impl<T> Deref for Spanned<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
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

impl Display for Specifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "%{}",
            match self {
                Specifier::Architecture => "a",
                Specifier::ImageVersion => "A",
                Specifier::BootID => "b",
                Specifier::BuildID => "B",
                Specifier::CacheDir => "C",
                Specifier::UserGroup => "g",
                Specifier::UserGID => "G",
                Specifier::UserHome => "h",
                Specifier::Hostname => "H",
                Specifier::ShortHostname => "l",
                Specifier::LogDir => "L",
                Specifier::MachineID => "m",
                Specifier::ImageID => "M",
                Specifier::OperatingSystemID => "o",
                Specifier::StateDir => "S",
                Specifier::RuntimeDir => "T",
                Specifier::TempDir => "t",
                Specifier::Username => "u",
                Specifier::UserUID => "U",
                Specifier::KernelRelease => "v",
                Specifier::PersistentTempDir => "V",
                Specifier::VersionID => "w",
                Specifier::VariantID => "W",
                Specifier::PercentSign => "%",
            }
        )
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct SpecifierString(Box<[u8]>, Box<[(Specifier, Box<[u8]>)]>);

impl SpecifierString {
    pub fn new(
        initial_component: Box<[u8]>,
        trailing_components: Box<[(Specifier, Box<[u8]>)]>,
    ) -> Self {
        Self(initial_component, trailing_components)
    }
    pub fn new_without_specifiers(string: Box<[u8]>) -> Self {
        Self::new(string, [].into())
    }
    pub fn as_path_no_specifiers(&self) -> Option<&Path> {
        if self.1.is_empty() {
            Some(Path::new(OsStr::from_bytes(&self.0)))
        } else {
            None
        }
    }
    pub fn components_iter(&self) -> impl Iterator<Item = &[u8]> {
        struct Iter<'a> {
            idx: usize,
            str: &'a SpecifierString,
        }
        impl<'a> Iterator for Iter<'a> {
            type Item = &'a [u8];

            fn next(&mut self) -> Option<Self::Item> {
                let value = if self.idx == 0 {
                    self.str.0.as_ref()
                } else {
                    self.str.1.get(self.idx - 1)?.1.as_ref()
                };
                self.idx += 1;
                Some(value)
            }
        }
        Iter { idx: 0, str: self }
    }
    pub fn first_component(&self) -> &[u8] {
        &self.0
    }

    pub fn specifier_iter(&self) -> impl Iterator<Item = &Specifier> {
        self.1.iter().map(|(specifier, _)| specifier)
    }
}
