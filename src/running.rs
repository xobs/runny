extern crate nix;

use tty::TtyServer;
use self::nix::sys::signal::{SIGTERM, SIGKILL};
use self::nix::sys::signal::kill;
use self::nix::sys::wait::waitpid;

use std::process::Child;
use std::io::{self, Read, Result, Write};
use std::fs::File;
use std::fmt;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::result;
use std::sync::{Arc, Mutex};

// We must not drop "tty" until the process exits,
// however we never actually /use/ tty.
#[allow(dead_code)]
pub struct Running {
    tty: TtyServer,
    child: Child,
    stream: File,
    timeout_thread: Option<JoinHandle<()>>,
    result: Arc<Mutex<Option<i32>>>,
}

pub struct RunningWaiter {
    pid: i32,
    result: Arc<Mutex<Option<i32>>>,
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
                if kill(-id, SIGTERM).is_err() {
                    *exited_thr.lock().unwrap() = true;
                    return;
                }

                thread::park_timeout(Duration::from_secs(5));
                if *exited_thr.lock().unwrap() == true {
                    return;
                }
                kill(-id, SIGKILL).ok();
                *exited_thr.lock().unwrap() = true;
            }))
        } else {
            None
        };
        Running {
            tty: tty,
            child: child,
            stream: file,
            timeout_thread: thr,
            result: Arc::new(Mutex::new(None)),
        }
    }

    pub fn wait(&mut self) -> result::Result<i32, RunningError> {
        // Convert a None ExitStatus into -1, removing
        // the Option<> from the type chain.
        match self.child.wait() {
            Ok(res) => {
                let val = match res.code() {
                    Some(r) => r,
                    None => -1,
                };
                (*self.result.lock().unwrap()) = Some(val);
                Ok(val)
            }
            Err(e) => Err(RunningError::RunningIoError(e)),
        }
    }

    pub fn waitable(&self) -> RunningWaiter {
        RunningWaiter {
            pid: self.child.id() as i32,
            result: self.result.clone(),
        }
    }

    pub fn result(&mut self) -> i32 {
        self.wait().unwrap();

        // We can unwrap here, because wait() should always set Some
        // value, and if not it's a bad bug anyway.
        self.result.lock().unwrap().unwrap()
    }

    pub fn terminate(&mut self, timeout: Option<Duration>) -> result::Result<i32, RunningError> {
        let pid = self.child.id() as i32;
        if let Some(res) = *self.result.lock().unwrap() {
            return Ok(res);
        }

        // If there's a timeout, give the process some time to quit before sending a SIGKILL.
        let result = match timeout {
            None => {
                // Eat the error, since there's nothing we can do if it fails.
                kill(-pid, SIGKILL).ok();
                self.wait()
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
                let res = self.wait();
                thr.thread().unpark();
                res
            }
        };

        // If there was a timeout, a timeout_thread will have been created.
        // Wake it up so that it can terminate.
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

impl Write for Running {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> Result<()> {
        self.stream.flush()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // Terminate immediately
        self.terminate(None).ok();
    }
}

impl RunningWaiter {
    pub fn wait(&self) {
        waitpid(self.pid, None).ok();
    }

    pub fn result(&self) -> i32 {
        loop {
            if let Some(res) = *self.result.lock().unwrap() {
                return res;
            }
            thread::park_timeout(Duration::from_millis(50));
        }
    }
}