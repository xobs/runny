extern crate tty;
extern crate chan_signal;
extern crate libc;
extern crate shlex;

use chan_signal::Signal;
use tty::{FileDesc, TtyServer};

use std::process::{Command, Stdio, Child, ExitStatus};
use std::io::{self, Read};
use std::fmt;
use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};

pub struct Runny {
    cmd: String,
    args: Vec<String>,
    working_directory: Option<String>,
}

pub struct Running {
    tty: TtyServer,
    child: Child,
    master: std::fs::File,
}

pub enum RunnyError {
    RunnyIoError(io::Error),
    NoCommandSpecified,
}

impl fmt::Debug for RunnyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &RunnyError::RunnyIoError(ref e) => write!(f, "I/O error: {:?}", e),
            &RunnyError::NoCommandSpecified => write!(f, "No command was specified"),
        }
    }
}

impl From<std::io::Error> for RunnyError {
    fn from(kind: std::io::Error) -> Self {
        RunnyError::RunnyIoError(kind)
    }
}

impl Runny {
    pub fn new(cmd: &str) -> Result<Runny, RunnyError> {
        let mut args = Self::make_command(cmd)?;
        let cmd = args.remove(0);

        Ok(Runny {
            cmd: cmd,
            args: args,
            working_directory: None,
        })
    }

    pub fn set_directory(&mut self, wd: &str) {
        self.working_directory = Some(wd.to_string());
    }

    pub fn start(&self) -> Result<Running, RunnyError> {

        // Create a new session, tied to stdin (FD number 0)
        let stdin_fd = tty::FileDesc::new(0 as i32, true);
        let mut tty = TtyServer::new(Some(&stdin_fd))?;

        let mut cmd = Command::new(&self.cmd);
        cmd.env_clear()
            .args(self.args.as_slice());
        if let Some(ref wd) = self.working_directory {
            cmd.current_dir(wd);
        }

        // Spawn a child.  Since we're doing this with a TtyServer,
        // it will have its own session, and will terminate
        let child = tty.spawn(cmd)?;

        let master_raw = FileDesc::new(tty.get_master().as_raw_fd(), true);
        let master = unsafe { File::from_raw_fd(master_raw.into_raw_fd()) };

        Ok(Running {
            tty: tty,
            child: child,
            master: master,
        })
    }

    fn make_command(cmd: &str) -> Result<Vec<String>, RunnyError> {
        let cmd = cmd.to_string().replace("\\", "\\\\");
        let cmd = cmd.as_str();
        match shlex::split(cmd) {
            None => Err(RunnyError::NoCommandSpecified),
            Some(s) => Ok(s),
        }
    }
}

impl Running {
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_ls() {
        let cmd = Runny::new("/bin/ls -l /etc").unwrap();
        // let cmd = Runny::new("tty").unwrap();
        let mut running = cmd.start().unwrap();
        let mut simple_str = String::new();

        running.master.read_to_string(&mut simple_str);
        println!("Read string: {}", simple_str);
    }
}
