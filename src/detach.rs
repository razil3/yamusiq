//! Running helper processes (ffmpeg) out of sight of the terminal.
//!
//! macOS Terminal names a window after the foreground process *and its
//! descendants* ("yamusiq ▸ ffmpeg"), whatever their controlling terminal.
//! So in the player ffmpeg is started as an orphan: a short-lived middle
//! process forks it and exits, ffmpeg is re-parented to launchd, and we keep
//! only its pid (plus the stdout pipe) to read from it and stop it.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdout, Command};

pub enum Proc {
    /// Ordinary child (command-line runs, so Ctrl-C reaches it).
    Child(Child),
    /// Orphaned process we only know by pid; launchd reaps it.
    Orphan(i32),
}

impl Proc {
    /// Spawns `cmd`; with `detached`, as an orphan in its own session with
    /// no controlling terminal. Returns the process and its stdout, if piped.
    pub fn spawn(cmd: &mut Command, detached: bool) -> io::Result<(Proc, Option<ChildStdout>)> {
        if !detached {
            let mut child = cmd.spawn()?;
            let out = child.stdout.take();
            return Ok((Proc::Child(child), out));
        }
        let mut fds = [0i32; 2];
        // SAFETY: plain pipe(2)/fcntl(2) on a local array.
        unsafe {
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                return Err(io::Error::last_os_error());
            }
            libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(fds[1], libc::F_SETFD, libc::FD_CLOEXEC);
        }
        let (r, w) = (fds[0], fds[1]);
        // SAFETY: runs in the forked child before exec and only calls
        // async-signal-safe functions (setsid, fork, write, _exit).
        unsafe {
            cmd.pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                let pid = libc::fork();
                if pid < 0 {
                    return Err(io::Error::last_os_error());
                }
                if pid > 0 {
                    // Middle process: report the grandchild's pid and vanish.
                    let b = pid.to_ne_bytes();
                    libc::write(w, b.as_ptr().cast(), b.len());
                    libc::_exit(0);
                }
                Ok(()) // grandchild goes on to exec ffmpeg
            });
        }
        let spawned = cmd.spawn();
        // SAFETY: closing our copy of the write end.
        unsafe { libc::close(w) };
        let mut middle = match spawned {
            Ok(c) => c,
            Err(e) => {
                unsafe { libc::close(r) };
                return Err(e);
            }
        };
        let out = middle.stdout.take();
        let _ = middle.wait(); // reap the middle process (exits at once)
        let mut b = [0u8; 4];
        // SAFETY: reading 4 bytes into a local buffer, then closing.
        let n = unsafe {
            let n = libc::read(r, b.as_mut_ptr().cast(), b.len());
            libc::close(r);
            n
        };
        if n != 4 {
            return Err(io::Error::other("не удалось запустить ffmpeg в фоне"));
        }
        Ok((Proc::Orphan(i32::from_ne_bytes(b)), out))
    }

    #[cfg(test)]
    pub fn id(&self) -> u32 {
        match self {
            Proc::Child(c) => c.id(),
            Proc::Orphan(pid) => *pid as u32,
        }
    }

    /// Stops the process (if still running) and reaps it when it is ours.
    pub fn kill(&mut self) {
        match self {
            Proc::Child(c) => {
                let _ = c.kill();
                let _ = c.wait();
            }
            // SAFETY: signalling a pid we started. It is reaped by launchd, so
            // the pid can't be reused while it is still a zombie of ours.
            Proc::Orphan(pid) => unsafe {
                libc::kill(*pid, libc::SIGKILL);
            },
        }
    }

    /// Reaps a finished child; orphans need nothing.
    pub fn reap(&mut self) {
        if let Proc::Child(c) = self {
            let _ = c.wait();
        }
    }
}
