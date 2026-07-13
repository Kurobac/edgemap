use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::process::CommandExt;
use std::process::Command;

use nix::sys::signal::{pthread_sigmask, SigSet, SigmaskHow, Signal};
use nix::sys::signalfd::{SfdFlags, SignalFd};

pub struct ShutdownSignal {
    fd: SignalFd,
    old_mask: SigSet,
}

impl ShutdownSignal {
    pub fn new() -> io::Result<Self> {
        let mut mask = SigSet::empty();
        mask.add(Signal::SIGINT);
        mask.add(Signal::SIGTERM);

        let mut old_mask = SigSet::empty();
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&mask), Some(&mut old_mask))?;
        let fd = match SignalFd::with_flags(
            &mask,
            SfdFlags::SFD_CLOEXEC | SfdFlags::SFD_NONBLOCK,
        ) {
            Ok(fd) => fd,
            Err(error) => {
                let _ = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&old_mask), None);
                return Err(error.into());
            }
        };
        Ok(Self { fd, old_mask })
    }

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    pub fn consume(&self) -> io::Result<bool> {
        let mut consumed = false;
        while self.fd.read_signal()?.is_some() {
            consumed = true;
        }
        Ok(consumed)
    }

}

impl Drop for ShutdownSignal {
    fn drop(&mut self) {
        let _ = pthread_sigmask(SigmaskHow::SIG_SETMASK, Some(&self.old_mask), None);
    }
}

pub fn unblock_shutdown_signals_in_child(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            let mut mask = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
            if libc::sigemptyset(mask.as_mut_ptr()) != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut mask = mask.assume_init();
            if libc::sigaddset(&mut mask, libc::SIGINT) != 0
                || libc::sigaddset(&mut mask, libc::SIGTERM) != 0
            {
                return Err(io::Error::last_os_error());
            }
            if libc::sigprocmask(libc::SIG_UNBLOCK, &mask, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_directed_shutdown_is_reported() {
        let shutdown = ShutdownSignal::new().unwrap();
        assert!(!shutdown.consume().unwrap());

        let result = unsafe { libc::pthread_kill(libc::pthread_self(), libc::SIGTERM) };
        assert_eq!(result, 0);
        assert!(shutdown.consume().unwrap());
    }
}
