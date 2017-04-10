extern crate nix;

use tty::TtyServer;
use self::nix::sys::signal::{SIGTERM, SIGKILL};
use self::nix::sys::signal::kill;

use std::process::Child;
use std::io::{self, Read, Result, Write};
use std::fs::File;
use std::fmt;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::result;
use std::sync::{Arc, Mutex, Condvar};

pub struct RunningWaiter {
    result: Arc<(Mutex<Option<i32>>, Condvar)>,
    term_thr: Arc<Mutex<JoinHandle<()>>>,
    term_delay: Arc<Mutex<Option<Duration>>>,
}

pub struct RunningOutput {
    stream: File,
}

pub struct RunningInput {
    stream: File,
}

// We must not drop "tty" until the process exits,
// however we never actually /use/ tty.
#[allow(dead_code)]
pub struct Running {
    tty: TtyServer,
    child_pid: i32,
    input: Option<RunningInput>,
    output: Option<RunningOutput>,
    term_thr: Arc<Mutex<JoinHandle<()>>>,
    term_delay: Arc<Mutex<Option<Duration>>>,
    wait_thr: JoinHandle<()>,
    result: Arc<(Mutex<Option<i32>>, Condvar)>,
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

impl fmt::Debug for Running {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Running {}: {:?}", self.child_pid, self.result)
    }
}

impl Running {
    pub fn new(tty: TtyServer, mut child: Child, timeout: Option<Duration>) -> Running {
        let input = unsafe { File::from_raw_fd(tty.get_master().as_raw_fd()) };
        let output = unsafe { File::from_raw_fd(tty.get_master().as_raw_fd()) };

        let child_pid = child.id() as i32;
        let child_result = Arc::new((Mutex::new(None), Condvar::new()));
        let child_result_thr = child_result.clone();
        let term_delay: Arc<Mutex<Option<Duration>>> = Arc::new(Mutex::new(None));
        let term_delay_thr = term_delay.clone();

        let term_thr = Arc::new(Mutex::new(thread::spawn(move || {

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
        })));

        // This thread just does a wait() on the child, and stores the result
        // in a variable.
        let wait_thr = thread::spawn(move || {
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
            let &(ref lock, ref cvar) = &*child_result_thr;
            let mut child_result = lock.lock().unwrap();
            *child_result = result;
            cvar.notify_all();
        });

        Running {
            tty: tty,
            child_pid: child_pid,
            term_delay: term_delay,
            input: Some(RunningInput { stream: input }),
            output: Some(RunningOutput { stream: output }),
            term_thr: term_thr,
            wait_thr: wait_thr,
            result: child_result,
        }
    }

    pub fn take_output(&mut self) -> RunningOutput {
        let value = self.output.take();
        value.unwrap()
    }

    pub fn output(&self) -> &Option<RunningOutput> {
        // Unwrap, because it's bad if we get output after it's taken
        &self.output
    }

    pub fn take_input(&mut self) -> RunningInput {
        let stream = self.input.take();
        stream.unwrap()
    }

    pub fn input(&self) -> &Option<RunningInput> {
        &self.input
    }

    pub fn wait(&mut self) -> result::Result<i32, RunningError> {
        // Convert a None ExitStatus into -1, removing
        // the Option<> from the type chain.
        println!("Waiting on child with PID {}", self.child_pid);
        let &(ref lock, ref cvar) = &*self.result;
        let mut ret = lock.lock().unwrap();
        while ret.is_none() {
            ret = cvar.wait(ret).unwrap();
        }
        Ok(ret.unwrap())
    }

    pub fn waiter(&self) -> RunningWaiter {
        RunningWaiter {
            result: self.result.clone(),
            term_thr: self.term_thr.clone(),
            term_delay: self.term_delay.clone(),
        }
    }

    pub fn result(&mut self) -> i32 {
        self.wait().unwrap();

        // We can unwrap here, because wait() should always set Some
        // value, and if not it's a bad bug anyway.
        self.result.0.lock().unwrap().unwrap()
    }

    pub fn terminate(&mut self, timeout: Option<Duration>) -> result::Result<i32, RunningError> {

        // If there's already a result, then the process has exited already.
        if let Some(res) = *self.result.0.lock().unwrap() {
            return Ok(res);
        }

        // Set up the delay, then wake up the termination thread.
        *self.term_delay.lock().unwrap() = timeout;
        self.term_thr.lock().unwrap().thread().unpark();

        // Hand execution off to self.wait(), which shouldn't block now that the process is
        // being terminated.
        self.wait()
    }
}


impl Read for Running {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let mut output = match self.output {
            Some(ref mut s) => s,
            None => return Err(io::Error::from_raw_os_error(9 /* EBADF */)),
        };

        match output.read(buf) {
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
        let mut input = match self.input {
            Some(ref mut s) => s,
            None => return Err(io::Error::from_raw_os_error(9 /* EBADF */)),
        };
        input.write(buf)
    }

    fn flush(&mut self) -> Result<()> {
        let mut input = match self.input {
            Some(ref mut s) => s,
            None => return Err(io::Error::from_raw_os_error(9 /* EBADF */)),
        };
        input.flush()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // Terminate immediately
        self.terminate(None).ok();
    }
}

impl Read for RunningOutput {
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

impl Write for RunningInput {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> Result<()> {
        self.stream.flush()
    }
}

impl RunningWaiter {
    pub fn wait(&self) {
        // waitpid(self.pid, None).ok();
        self.result();
    }

    pub fn result(&self) -> i32 {
        let &(ref lock, ref cvar) = &*self.result;
        let mut ret = lock.lock().unwrap();
        while ret.is_none() {
            ret = cvar.wait(ret).unwrap();
        }
        ret.unwrap()
    }

    pub fn terminate(&self, timeout: &Option<Duration>) {
        *self.term_delay.lock().unwrap() = *timeout;
        self.term_thr.lock().unwrap().thread().unpark();
    }
}