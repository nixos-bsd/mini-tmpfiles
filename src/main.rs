mod config_file;
mod parser;

use clap::Parser;
use std::{
    collections::BTreeMap,
    error::Error,
    ffi::OsString,
    fs,
    io::{self, Write},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
};

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
        cat_config(&config_files)?;
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
