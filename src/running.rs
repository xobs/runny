extern crate nix;

use tty::TtyServer;
use self::nix::sys::signal::{SIGTERM, SIGKILL};
use self::nix::sys::signal::kill;

use std::process::{Child, ExitStatus};
use std::io::{self, Read, Result};
use std::fs::File;
use std::fmt;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::result;
use std::sync::{Arc, Mutex};

pub struct Running {
    tty: TtyServer,
    child: Child,
    stream: File,
    timeout_thread: Option<JoinHandle<()>>,
    exited: Arc<Mutex<bool>>,
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

impl fmt::Debug for RunningError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &RunningError::RunningIoError(ref e) => write!(f, "Running I/O error: {:?}", e),
            &RunningError::RunningNixError(ref e) => write!(f, "Running Nix error: {:?}", e),
        }
    }
}

impl Running {
    pub fn new(tty: TtyServer, child: Child, timeout: Option<Duration>) -> Running {
        let file = unsafe { File::from_raw_fd(tty.get_master().as_raw_fd()) };
        let id = child.id() as i32;

        let exited = Arc::new(Mutex::new(false));
        let exited_thr = exited.clone();

        let thr = if let Some(t) = timeout {
            Some(thread::spawn(move || {
                thread::park_timeout(t);
                if *exited_thr.lock().unwrap() == true {
                    return;
                }
                if let Err(e) = kill(-id, SIGTERM) {
                    println!("Got error sending SIGTERM: {:?}", e);
                }

                thread::park_timeout(Duration::from_secs(5));
                if *exited_thr.lock().unwrap() == true {
                    return;
                }
                kill(-id, SIGKILL).ok();
            }))
        } else {
            None
        };
        Running {
            tty: tty,
            child: child,
            stream: file,
            timeout_thread: thr,
            exited: Arc::new(Mutex::new(false)),
        }
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    pub fn terminate(&mut self, timeout: Option<Duration>) -> result::Result<(), RunningError> {
        let pid = self.child.id() as i32;
        if *self.exited.lock().unwrap() == true {
            return Ok(());
        }

        // If there's a timeout, give the process some time to quit before sending a SIGKILL.
        let result = match timeout {
            None => {
                let ret = match kill(-pid, SIGKILL) {
                    Ok(_) => Ok(()),
                    Err(e) => Err(RunningError::RunningNixError(e)),
                };
                self.child.wait().ok();
                ret
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
                self.child.wait().ok();
                thr.thread().unpark();
                Ok(())
            }
        };
        *self.exited.lock().unwrap() = true;

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
        // Terminate immediately
        self.terminate(None).ok();
    }
}