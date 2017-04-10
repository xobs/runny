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
    child_pid: i32,
    stream: File,
    thr: JoinHandle<()>,
    term_delay: Arc<Mutex<Option<Duration>>>,
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
    pub fn new(tty: TtyServer, mut child: Child, timeout: Option<Duration>) -> Running {
        let file = unsafe { File::from_raw_fd(tty.get_master().as_raw_fd()) };
        let child_pid = child.id() as i32;
        let child_result = Arc::new(Mutex::new(None));
        let child_result_thr = child_result.clone();
        let term_delay: Arc<Mutex<Option<Duration>>> = Arc::new(Mutex::new(None));
        let term_delay_thr = term_delay.clone();

        let thr = thread::spawn(move || {

            // Allow the child process to run for a given amount of time,
            // or until we're woken up by a termination process.
            if let Some(t) = timeout {
                thread::park_timeout(t);
            } else {
                thread::park();
            }

            // We've been woken up, so it's time to terminate the child process.
            // Use a negative value to terminate all children in the process group.
            kill(-child_pid, SIGTERM).ok();

            if let Some(t) = *term_delay_thr.lock().unwrap() {
                thread::park_timeout(t);
            }

            // Send a SIGKILL to all children, to ensure they're gone.
            kill(-child_pid, SIGKILL).ok();
        });

        // This thread just does a wait() on the child, and stores the result
        // in a variable.
        let wait_thread = thread::spawn(move || {
            // Finally, get the return code of the process.
            // println!("Waiting on child...");
            let result = match child.wait() {
                Err(e) => {
                    println!("Got an error: {:?}", e);
                    Some(-1)
                }
                Ok(o) => {
                    match o.code() {
                        Some(c) => Some(c),
                        None => Some(-2),
                    }
                }
            };
            // println!("Assigning result to child_result");
            *child_result_thr.lock().unwrap() = result;
            // println!("Done, returning");
        });

        Running {
            tty: tty,
            child_pid: child_pid,
            term_delay: term_delay,
            stream: file,
            thr: thr,
            result: child_result,
        }
    }

    pub fn wait(&mut self) -> result::Result<i32, RunningError> {
        // Convert a None ExitStatus into -1, removing
        // the Option<> from the type chain.
        println!("Waiting on child with PID {}", self.child_pid);
        loop {
            if let Some(ret) = *self.result.lock().unwrap() {
                return Ok(ret);
            }
            // waitpid(self.child_pid, None).ok();
            thread::park_timeout(Duration::from_millis(50));
        }
    }

    pub fn waitable(&self) -> RunningWaiter {
        RunningWaiter {
            pid: self.child_pid,
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

        // If there's already a result, then the process has exited already.
        if let Some(res) = *self.result.lock().unwrap() {
            return Ok(res);
        }

        // Set up the delay, then wake up the termination thread.
        *self.term_delay.lock().unwrap() = timeout;
        self.thr.thread().unpark();

        // Hand execution off to self.wait(), which shouldn't block now that the process is
        // being terminated.
        self.wait()
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
        // waitpid(self.pid, None).ok();
        self.result();
    }

    pub fn result(&self) -> i32 {
        loop {
            println!("Locking self.result...");
            if let Some(res) = *self.result.lock().unwrap() {
                println!("Result was {}", res);
                return res;
            }
            println!("Result was None.  Sleeping.  Zzz...");
            thread::park_timeout(Duration::from_millis(500));
        }
    }
}