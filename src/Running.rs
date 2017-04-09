extern crate nix;

use chan_signal::Signal;
use tty::{FileDesc, TtyServer};
use self::nix::sys::signal::{SIGTERM, SIGKILL};
use self::nix::sys::signal::kill;

use std::process::{Command, Child, ExitStatus};
use std::io::{self, Read, Result};
use std::fmt;
use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::result;

pub struct Running {
    tty: TtyServer,
    child: Child,
    stream: File,
    timeout: Option<Duration>,
    timeout_thread: Option<JoinHandle<()>>,
}

pub enum RunningError {
    RunningIoError(io::Error),
    RunningNixError(self::nix::Error),
}

impl From<io::Error> for RunningError {
    fn from(kind: io::Error) -> Self {
        RunningError::RunningIoError(kind)
    }
}

impl From<self::nix::Error> for RunningError {
    fn from(kind: self::nix::Error) -> Self {
        RunningError::RunningNixError(kind)
    }
}

impl Running {
    pub fn new(tty: TtyServer, child: Child, timeout: Option<Duration>) -> Running {
        let file = unsafe { File::from_raw_fd(tty.get_master().as_raw_fd()) };
        let id = child.id() as i32;

        let thr = if let Some(t) = timeout {
            Some(thread::spawn(move || {
                thread::park_timeout(t);
                if let Err(e) = kill(-id, SIGTERM) {
                    println!("Got error sending SIGTERM: {:?}", e);
                }
                thread::park_timeout(Duration::from_secs(5));
                if let Err(e) = kill(-id, SIGKILL) {
                    println!("Got error sending SIGKILL: {:?}", e);
                }
            }))
        } else {
            None
        };
        Running {
            tty: tty,
            child: child,
            stream: file,
            timeout: timeout,
            timeout_thread: thr,
        }
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    pub fn terminate(&mut self, timeout: Option<Duration>) -> result::Result<(), RunningError> {
        let pid = self.child.id() as i32;
        // If there's a timeout, give the process some time to quit before sending a SIGKILL.
        let result = match timeout {
            None => {
                match kill(-pid, SIGKILL) {
                    Ok(_) => Ok(()),
                    Err(e) => Err(RunningError::RunningNixError(e)),
                }
            }
            Some(t) => {
                let thr = thread::spawn(move || {
                    // Send the terminal to -pid, which also sends it to every
                    // process in the controlling group.
                    kill(-pid, SIGTERM).ok();
                    thread::park_timeout(t);
                    kill(-pid, SIGKILL).ok();
                });

                // Wait for the child to terminate,
                self.child.wait();
                thr.thread().unpark();
                Ok(())
            }
        };

        if let Some(ref thr) = self.timeout_thread {
            thr.thread().unpark();
        };

        result
    }

    pub fn get_interface(&self) -> &File {
        // let master_raw = FileDesc::new(self.tty.get_master().as_raw_fd(), true);
        &self.stream
    }
}

impl Read for Running {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        match self.stream.read(buf) {
            Err(e) => {
                match e.raw_os_error() {
                    Some(5) => Ok(0),
                    _ => Err(e),
                }
            }
            Ok(n) => Ok(n),
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // Terminate immediately.
        self.terminate(None);
    }
}