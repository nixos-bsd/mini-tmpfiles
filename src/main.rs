mod config_file;
mod parser;

use clap::Parser;
use config_file::Line;
use eyre::Context;
use std::{
    collections::BTreeMap,
    error::Error,
    ffi::{OsStr, OsString},
    fs::{self, Permissions},
    io::{self, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

use crate::parser::{parse_line, FileSpan};

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
            match create(line) {
                Ok(()) => {}
                Err(e) => {
                    eprint!("{}: line {}", line.line_type.file.to_string_lossy(), i + 1);
                    for err in e.chain() {
                        eprint!(": {}", err);
                    }
                    eprintln!();
                }
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

fn create_parents(path: &Path, check: bool) -> eyre::Result<()> {
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
                if check {
                    match fs::symlink_metadata(&buf) {
                        Ok(m) => if !m.is_dir() {},
                        Err(e) => match e.kind() {
                            io::ErrorKind::NotFound => {
                                match fs::DirBuilder::new().mode(0o755).create(&buf) {
                                    Ok(_) => {}
                                    Err(e) => match e.kind() {
                                        io::ErrorKind::AlreadyExists => {}
                                        _ => {
                                            Err(e).wrap_err("Failed to create parent directory")?
                                        }
                                    },
                                }
                            }
                            _ => Err(e).wrap_err("Failed to get parent directory metadata")?,
                        },
                    }
                } else {
                    match fs::DirBuilder::new().mode(0o755).create(&buf) {
                        Ok(_) => {}
                        Err(e) => match e.kind() {
                            io::ErrorKind::AlreadyExists => {}
                            _ => Err(e).wrap_err("Failed to create parent directory")?,
                        },
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
            let file = Path::new(OsStr::from_bytes(&line.path.data.0));
            let contents = match line.argument.data.as_ref() {
                Some(contents) => contents.as_bytes(),
                None => b"",
            };
            if contents.contains(&b'%') {
                todo!("Specifiers in file contents not yet implemented")
            } else if !line.path.data.1.is_empty() {
                Err(eyre::eyre!("Specifiers in file path not yet implemented"))?
            }
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
                            if let Some(specmode) = line.mode.data.as_ref() {
                                if specmode.value != meta.mode() {
                                    fs::set_permissions(
                                        file,
                                        Permissions::from_mode(specmode.value),
                                    )
                                    .wrap_err("Failed to set permissions of existing directory")?;
                                }
                            }
                            if let Some(specuser) = line.owner.data.as_ref() {
                                let specuid = specuser.as_uid()?;
                                if specuid != meta.uid() {
                                    std::os::unix::fs::chown(file, Some(specuid), None)?;
                                }
                            }
                            if let Some(specuser) = line.group.data.as_ref() {
                                let specgid = specuser.as_gid()?;
                                if specgid != meta.gid() {
                                    std::os::unix::fs::chown(file, None, Some(specgid))?;
                                }
                            }
                            return Ok(());
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
            create_parents(file, line_type.force)?;
            let mut fp = fs::File::create(file).wrap_err("Creating file")?;
            fp.write(contents).wrap_err("Writing contents")?;
            if let Some(specmode) = line.mode.data.as_ref() {
                fp.set_permissions(Permissions::from_mode(specmode.value))
                    .wrap_err("Setting permissions of new file")?;
            }
            let uid = match line.owner.data.as_ref() {
                Some(o) => Some(o.as_uid()?),
                None => None,
            };
            let gid = match line.group.data.as_ref() {
                Some(o) => Some(o.as_gid()?),
                None => None,
            };
            std::os::unix::fs::fchown(&fp, uid, gid).wrap_err("Setting ownership of new file")?;
        }
        config_file::LineAction::WriteFile => todo!(),
        config_file::LineAction::CreateAndCleanUpDirectory => {
            let dir = Path::new(OsStr::from_bytes(&line.path.data.0));
            if !line.path.data.1.is_empty() {
                Err(eyre::eyre!(
                    "Specifiers in directory path not yet implemented"
                ))?
            }
            match fs::symlink_metadata(dir) {
                Ok(meta) => {
                    if meta.is_dir() {
                        // It's already here! Fix up the attrs and bail.
                        if let Some(specmode) = line.mode.data.as_ref() {
                            if specmode.value != meta.mode() {
                                fs::set_permissions(dir, Permissions::from_mode(specmode.value))
                                    .wrap_err("Failed to set permissions of existing directory")?;
                            }
                        }
                        if let Some(specuser) = line.owner.data.as_ref() {
                            let specuid = specuser.as_uid()?;
                            if specuid != meta.uid() {
                                std::os::unix::fs::chown(dir, Some(specuid), None)?;
                            }
                        }
                        if let Some(specuser) = line.group.data.as_ref() {
                            let specgid = specuser.as_gid()?;
                            if specgid != meta.gid() {
                                std::os::unix::fs::chown(dir, None, Some(specgid))?;
                            }
                        }
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
            if let Some(mode) = &line.mode.data {
                fs::set_permissions(dir, Permissions::from_mode(mode.value))
                    .wrap_err("Setting mode of new directory")?;
            }
            let uid = match line.owner.data.as_ref() {
                Some(o) => Some(o.as_uid()?),
                None => None,
            };
            let gid = match line.group.data.as_ref() {
                Some(o) => Some(o.as_gid()?),
                None => None,
            };
            std::os::unix::fs::chown(dir, uid, gid)
                .wrap_err("Setting ownership of new directory")?;
        }
        config_file::LineAction::CreateAndRemoveDirectory => todo!(),
        config_file::LineAction::CleanUpDirectory => todo!(),
        config_file::LineAction::CreateFifo => todo!(),
        config_file::LineAction::CreateSymlink => {
            if line_type.boot || line_type.noerror || !line_type.recreate {
                todo!()
            }
            let target = line.argument.data.as_ref().unwrap();
            let link = Path::new(OsStr::from_bytes(&line.path.data.0));
            if target.as_bytes().contains(&b'%') {
                todo!("Specifiers in symlink target not yet implemented")
            } else if !line.path.data.1.is_empty() {
                todo!("Specifiers in symlink path not yet implemented")
            }
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

/// Print the output of each configuration file, without reencoding
fn cat_config(config_files: &BTreeMap<OsString, PathBuf>) -> io::Result<()> {
    println!("# WARNING: --cat-config is vulnerable to a TOCTOU attack, do not use for security purposes");

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
