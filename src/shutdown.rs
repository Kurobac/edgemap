use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::{Duration, Instant};

use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
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

    pub fn wait_timeout(&self, timeout: Duration) -> io::Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as u32;
            let mut fds = [PollFd::new(self.as_fd(), PollFlags::POLLIN)];
            match poll(
                &mut fds,
                PollTimeout::try_from(timeout_ms).unwrap_or(PollTimeout::MAX),
            ) {
                Ok(0) => return Ok(false),
                Ok(_) => {
                    let events = fds[0].revents().unwrap_or(PollFlags::empty());
                    if events.intersects(
                        PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL,
                    ) {
                        return Err(io::Error::other("shutdown signalfd poll failure"));
                    }
                    if events.contains(PollFlags::POLLIN) {
                        return self.consume();
                    }
                }
                Err(nix::errno::Errno::EINTR) => {
                    if Instant::now() >= deadline {
                        return Ok(false);
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
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
    fn timeout_and_thread_directed_shutdown_are_reported() {
        let shutdown = ShutdownSignal::new().unwrap();
        assert!(!shutdown.wait_timeout(Duration::ZERO).unwrap());

        let result = unsafe { libc::pthread_kill(libc::pthread_self(), libc::SIGTERM) };
        assert_eq!(result, 0);
        assert!(shutdown.wait_timeout(Duration::from_secs(1)).unwrap());
    }
}
