#![allow(dead_code)]
mod config_file;
mod parser;

use clap::Parser;
use config_file::{FileOwner, Line, Mode};
use eyre::Context;
use std::{
    collections::BTreeMap,
    error::Error,
    ffi::OsString,
    fs::{self, File, Permissions},
    io::{self, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

use crate::parser::{FileSpan, parse_line};

#[derive(Parser, Debug)]
#[command(version, about = "Standalone replacement for systemd-tmpfiles", long_about = None)]
struct Args {
    /// Create files and directories specified
    #[arg(long)]
    create: bool,
    /// Clean files with a max age parameter
    #[arg(long)]
    clean: bool,
    /// Remove directories and files, unless they are locked
    #[arg(long)]
    remove: bool,
    /// Also execute lines meant only to be run on boot
    #[arg(long)]
    boot: bool,
    /// Print the contents of files to apply
    #[arg(long)]
    cat_config: bool,

    /// Files or directories to apply
    #[arg(default_value = "/etc/tmpfiles.d")]
    config_sources: Vec<PathBuf>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let config_files = find_config_files(&args.config_sources)?;

    if args.cat_config {
        if args.remove || args.clean || args.create {
            todo!("--cat-config cannot be used with create, remove, or clean")
        }
        cat_config(&config_files)?;
        return Ok(());
    }

    let config = parsed_config(&config_files)?;

    if args.remove {
        todo!("Removal is not yet implemented")
    }
    if args.clean {
        todo!("Cleaning is not yet implemented")
    }
    if args.create {
        for (i, line) in config.iter().enumerate() {
            if let Err(e) = create(line) {
                eprint!("{}: line {}", line.line_type.file.to_string_lossy(), i + 1);
                for err in e.chain() {
                    eprint!(": {}", err);
                }
                eprintln!();
            }
        }
    }

    Ok(())
}

fn parsed_config(config_files: &BTreeMap<OsString, PathBuf>) -> eyre::Result<Vec<Line>> {
    let mut config = Vec::new();
    for file_path in config_files.values() {
        let file = fs::read(file_path)?;
        let span = FileSpan::from_slice(&file, file_path);
        for line in span.lines() {
            if line.bytes().starts_with(b"#") || line.bytes().is_empty() {
                continue;
            } else {
                let line = parse_line(line.clone()).unwrap_or_else(|e| {
                    todo!(
                        "Error parsing line: {e:#?} ({})",
                        line.bytes().escape_ascii()
                    )
                });
                config.push(line);
            }
        }
    }
    Ok(config)
}

fn create_parents(path: &Path, force: bool) -> eyre::Result<()> {
    let Some(path) = path.parent() else {
        return Ok(());
    };
    let mut buf = PathBuf::from("/");
    for comp in path.components() {
        match comp {
            std::path::Component::Prefix(_) => todo!(),
            std::path::Component::RootDir => {}
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => Err(eyre::eyre!("Path may not contain .."))?,
            std::path::Component::Normal(piece) => {
                buf.push(piece);
                match fs::symlink_metadata(&buf) {
                    Ok(m) if m.is_dir() => continue,
                    Ok(_) if force => fs::remove_file(&buf)
                        .map_err(|e| eyre::eyre!(e))
                        .wrap_err("Attempting to replace non-directory parent")?,
                    Ok(_) => return Err(eyre::eyre!("A parent directory is not a directory")),
                    Err(e) if e.kind() != io::ErrorKind::NotFound => {
                        Err(e).wrap_err("Failed to get parent directory metadata")?
                    }
                    Err(_) => {} // Not found. Create it below
                }
                if let Err(e) = fs::DirBuilder::new().mode(0o755).create(&buf) {
                    if e.kind() != io::ErrorKind::AlreadyExists {
                        Err(e).wrap_err("Failed to create parent directory")?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn create(line: &Line) -> eyre::Result<()> {
    let line_type = line.line_type.data;
    match line_type.action {
        config_file::LineAction::CreateFile => {
            let contents = line.argument.as_deref().unwrap_or_default().as_bytes();
            if contents.contains(&b'%') {
                todo!("Specifiers in file contents not yet implemented")
            }
            let Some(file) = line.path.as_path_no_specifiers() else {
                Err(eyre::eyre!("Specifiers in file path not yet implemented"))?
            };
            match fs::symlink_metadata(file) {
                Ok(meta) => {
                    if meta.is_dir() {
                        if line_type.force {
                            fs::remove_dir(file)
                                .wrap_err("Failed to remove directory in place of file")?;
                        } else {
                            Err(eyre::eyre!("There is already a directory here"))?;
                        }
                    } else if meta.is_symlink() {
                        if line_type.force {
                            Err(eyre::eyre!(
                                "Currently won't clobber symlinks to create files"
                            ))?;
                        } else {
                            Err(eyre::eyre!("There is already a symlink here"))?;
                        }
                    } else if meta.is_file() {
                        if !line_type.recreate {
                            // It's already here! Fix up the attrs and bail.
                            fixup_attrs(
                                file,
                                &meta,
                                &line.mode,
                                &line.owner,
                                &line.group,
                                "existing file",
                            )?;
                            return Ok(());
                        }
                    } else {
                        Err(eyre::eyre!(
                            "Won't clobber things other than files, directories, or symlinks"
                        ))?
                    }
                }
                Err(e) => match e.kind() {
                    io::ErrorKind::NotADirectory | io::ErrorKind::NotFound => {}
                    _ => Err(e).wrap_err("Failed to query file metadata")?,
                },
            }
            create_parents(file, line_type.force)?;
            let mut fp = fs::File::create(file).wrap_err("Creating file")?;
            fp.write(contents).wrap_err("Writing contents")?;
            set_attrs(
                &fp,
                line.mode.data.as_ref(),
                line.owner.data.as_ref(),
                line.group.data.as_ref(),
                "new file",
            )?;
        }
        config_file::LineAction::WriteFile => todo!(),
        config_file::LineAction::CreateAndCleanUpDirectory => {
            let Some(dir) = line.path.as_path_no_specifiers() else {
                Err(eyre::eyre!(
                    "Specifiers in directory path not yet implemented"
                ))?
            };
            match fs::symlink_metadata(dir) {
                Ok(meta) => {
                    if meta.is_dir() {
                        // It's already here! Fix up the attrs and bail.
                        fixup_attrs(
                            dir,
                            &meta,
                            &line.mode,
                            &line.owner,
                            &line.group,
                            "existing directory",
                        )?;
                        return Ok(());
                    } else if meta.is_file() || meta.is_symlink() {
                        if line_type.force {
                            fs::remove_file(dir)
                                .wrap_err("Failed to remove file in place of directory")?;
                        } else {
                            Err(eyre::eyre!("There is already a file here"))?;
                        }
                    } else {
                        Err(eyre::eyre!(
                            "Won't clobber things other than files, directories, or symlinks"
                        ))?
                    }
                }
                Err(e) => match e.kind() {
                    io::ErrorKind::NotFound => {}
                    _ => Err(e).wrap_err("Failed to query file metadata")?,
                },
            }
            create_parents(dir, line_type.force)?;
            fs::DirBuilder::new().create(dir)?;
            set_attrs(
                dir,
                line.mode.data.as_ref(),
                line.owner.data.as_ref(),
                line.group.data.as_ref(),
                "Setting mode of new directory",
            )?;
        }
        config_file::LineAction::CreateAndRemoveDirectory => todo!(),
        config_file::LineAction::CleanUpDirectory => todo!(),
        config_file::LineAction::CreateFifo => todo!(),
        config_file::LineAction::CreateSymlink => {
            if line_type.boot || line_type.noerror || !line_type.recreate {
                todo!()
            }
            let target = line.argument.data.as_ref().unwrap();
            if target.as_bytes().contains(&b'%') {
                todo!("Specifiers in symlink target not yet implemented")
            }
            let Some(link) = line.path.data.as_path_no_specifiers() else {
                todo!("Specifiers in symlink path not yet implemented")
            };
            let target = Path::new(target);
            match fs::symlink_metadata(link) {
                Ok(meta) => {
                    if meta.is_dir() {
                        // fs::remove_dir_all(target);
                        todo!("Currently won't clobber directories to create symlinks")
                    } else if meta.is_file() {
                        fs::remove_file(link)?;
                    } else if meta.is_symlink() {
                        let existing_target = fs::read_link(link)?;
                        if existing_target != target {
                            fs::remove_file(link)?;
                        } else {
                            return Ok(());
                        }
                    } else {
                        Err(eyre::eyre!(
                            "Won't clobber things other than files, directories, or symlinks"
                        ))?;
                    }
                }
                Err(e) => match e.kind() {
                    io::ErrorKind::NotFound => {}
                    _ => Err(e).wrap_err("Failed querying directory metadata")?,
                },
            }
            create_parents(link, line_type.force)?;
            std::os::unix::fs::symlink(target, link)?;
        }
        config_file::LineAction::CreateCharDevice => todo!(),
        config_file::LineAction::CreateBlockDevice => todo!(),
        config_file::LineAction::Copy => todo!(),
        config_file::LineAction::Ignore => todo!(),
        config_file::LineAction::IgnoreNonRecursive => todo!(),
        config_file::LineAction::Remove => todo!(),
        config_file::LineAction::RemoveRecursive => todo!(),
        config_file::LineAction::SetMode => todo!(),
        config_file::LineAction::SetModeRecursive => todo!(),
        config_file::LineAction::SetXattr => todo!(),
        config_file::LineAction::SetXattrRecursive => todo!(),
        config_file::LineAction::SetAttr => todo!(),
        config_file::LineAction::SetAttrRecursive => todo!(),
        config_file::LineAction::SetAcl => todo!(),
        config_file::LineAction::SetAclRecursive => todo!(),
    }
    Ok(())
}

// Abstraction over `File` and `Path` to use handle both in shared code
trait FileRef {
    fn set_permissions(&self, permissions: Permissions) -> io::Result<()>;
    fn set_ownership(&self, uid: Option<u32>, gid: Option<u32>) -> io::Result<()>;
}

impl FileRef for Path {
    fn set_permissions(&self, permissions: Permissions) -> io::Result<()> {
        fs::set_permissions(self, permissions)
    }

    fn set_ownership(&self, uid: Option<u32>, gid: Option<u32>) -> io::Result<()> {
        std::os::unix::fs::chown(self, uid, gid)
    }
}

impl FileRef for File {
    fn set_permissions(&self, permissions: Permissions) -> io::Result<()> {
        self.set_permissions(permissions)
    }

    fn set_ownership(&self, uid: Option<u32>, gid: Option<u32>) -> io::Result<()> {
        std::os::unix::fs::fchown(self, uid, gid)
    }
}

fn fixup_attrs(
    path: &(impl FileRef + ?Sized),
    meta: &fs::Metadata,
    mode: &Option<Mode>,
    owner: &Option<FileOwner>,
    group: &Option<FileOwner>,
    name_in_error: &str,
) -> Result<(), eyre::Error> {
    let mode = mode
        .as_ref()
        .filter(|specmode| specmode.value != meta.mode());
    let owner = owner
        .as_ref()
        .map(FileOwner::as_uid)
        .transpose()?
        .filter(|&o| o != meta.uid())
        .map(FileOwner::Id);
    let group = group
        .as_ref()
        .map(FileOwner::as_gid)
        .transpose()?
        .filter(|&g| g != meta.gid())
        .map(FileOwner::Id);
    set_attrs(path, mode, owner.as_ref(), group.as_ref(), name_in_error)
}

fn set_attrs(
    path: &(impl FileRef + ?Sized),
    mode: Option<&Mode>,
    owner: Option<&FileOwner>,
    group: Option<&FileOwner>,
    name_in_error: &str,
) -> Result<(), eyre::Error> {
    if let Some(specmode) = mode {
        path.set_permissions(Permissions::from_mode(specmode.value))
            .wrap_err_with(|| format!("Setting mode of {name_in_error}"))?;
    }
    let uid = owner.map(FileOwner::as_uid).transpose()?;
    let gid = group.map(FileOwner::as_gid).transpose()?;
    if uid.is_some() || gid.is_some() {
        path.set_ownership(uid, gid)
            .wrap_err_with(|| format!("Setting ownership of {name_in_error}"))?;
    }
    Ok(())
}

/// Print the output of each configuration file, without reencoding
fn cat_config(config_files: &BTreeMap<OsString, PathBuf>) -> io::Result<()> {
    println!(
        "# WARNING: --cat-config is vulnerable to a TOCTOU attack, do not use for security purposes"
    );

    // We need to write raw bytes. This is somewhat unsafe due to delete escape codes but I don't
    // want to unescape then escape to fix it.
    let mut stdout = io::stdout().lock();

    for (_, path) in config_files.iter() {
        stdout.write_all(b"# ")?;
        stdout.write_all(path.as_os_str().as_encoded_bytes())?;
        stdout.write_all(b"\n")?;
        stdout.write_all(&fs::read(path)?)?
    }
    stdout.write_all(b"\n")?;

    Ok(())
}

fn find_config_files(config_sources: &[PathBuf]) -> io::Result<BTreeMap<OsString, PathBuf>> {
    // We have to apply in lexographic order, so use a BTreeMap to stay sorted
    let mut config_files = BTreeMap::new();

    for config_source in config_sources {
        if config_source.is_file() {
            // We already know it exists and is a file, the kernel would have told us if it ended
            // in `..`, so just unwrap
            config_files.insert(
                config_source.file_name().unwrap().to_os_string(),
                config_source.clone(),
            );
            continue;
        }

        for maybe_entry in fs::read_dir(config_source)? {
            let entry = maybe_entry?;
            let path = entry.path();
            if path
                .extension()
                .map(|ext| ext.as_bytes() != b"conf")
                .unwrap_or(true)
            {
                continue;
            }

            if entry.file_type()?.is_file() || entry.file_type()?.is_symlink() && path.is_file() {
                config_files.insert(entry.file_name(), path);
            }
        }
    }

    Ok(config_files)
}
