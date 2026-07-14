use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub const LOCK_FILE_NAME: &str = "daemon.lock";

pub struct DaemonLock {
    _file: File,
}

impl DaemonLock {
    pub fn acquire(runtime_dir: &Path) -> io::Result<Self> {
        Self::acquire_named(runtime_dir, LOCK_FILE_NAME, "dseuhid")
    }

    pub fn acquire_named(
        runtime_dir: &Path,
        file_name: &str,
        process_name: &str,
    ) -> io::Result<Self> {
        std::fs::create_dir_all(runtime_dir)?;
        let path = runtime_dir.join(file_name);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o644)
            .open(&path)?;

        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                let mut owner = String::new();
                let _ = file.seek(SeekFrom::Start(0));
                let _ = file.read_to_string(&mut owner);
                let owner = owner.trim();
                let detail = if owner.is_empty() {
                    format!("another {process_name} instance holds the daemon lock")
                } else {
                    format!("another {process_name} instance holds the daemon lock (PID {owner})")
                };
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, detail));
            }
            return Err(error);
        }

        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "{}", std::process::id())?;
        Ok(Self { _file: file })
    }
}
