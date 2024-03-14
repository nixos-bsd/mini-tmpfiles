mod config_file;
mod parser;

use clap::Parser;
use config_file::Line;
use std::{
    collections::BTreeMap,
    error::Error,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write},
    os::unix::ffi::OsStrExt,
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
        create(&config)?;
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

fn create(config: &[Line]) -> eyre::Result<()> {
    for line in config {
        let line_type = line.line_type.data;
        match line_type.action {
            config_file::LineAction::CreateFile => todo!(),
            config_file::LineAction::WriteFile => todo!(),
            config_file::LineAction::CreateAndCleanUpDirectory => todo!(),
            config_file::LineAction::CreateAndRemoveDirectory => todo!(),
            config_file::LineAction::CleanUpDirectory => todo!(),
            config_file::LineAction::CreateFifo => todo!(),
            config_file::LineAction::CreateSymlink => {
                if line_type.boot || line_type.force || line_type.noerror || !line_type.recreate {
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
                                continue;
                            }
                        } else {
                            todo!("Won't clobber things other than files, directories, or symlinks")
                        }
                    }
                    Err(e) => match e.kind() {
                        io::ErrorKind::NotFound => {}
                        _ => todo!(),
                    },
                }
                std::os::unix::fs::symlink(Path::new(target), link)?;
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
