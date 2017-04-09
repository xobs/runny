extern crate tty;
extern crate chan_signal;
extern crate libc;
extern crate shlex;

use chan_signal::Signal;
use tty::{FileDesc, TtyServer};

use std::process::{Command, Child, ExitStatus};
use std::io;
use std::fmt;
use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};
use std::thread;

pub mod Running;
// use self::Running::Running;

pub struct Runny {
    cmd: String,
    args: Vec<String>,
    working_directory: Option<String>,
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

    pub fn start(&self) -> Result<Running::Running, RunnyError> {

        // Create a new session, tied to stdin (FD number 0)
        let stdin_fd = tty::FileDesc::new(0 as i32, false);
        let mut tty = TtyServer::new(Some(&stdin_fd))?;

        let mut cmd = Command::new(&self.cmd);
        cmd.env_clear()
            .args(self.args.as_slice());
        if let Some(ref wd) = self.working_directory {
            cmd.current_dir(wd);
        }

        // let signal = chan_signal::notify(&[Signal::WINCH]);
        // let proxy = match tty.new_client(stdin_fd, Some(signal)) {
        // Ok(p) => p,
        // Err(e) => panic!("Error TTY client: {}", e),
        // };
        //

        // Spawn a child.  Since we're doing this with a TtyServer,
        // it will have its own session, and will terminate
        let mut child = tty.spawn(cmd)?;
        // thread::spawn(move || proxy.wait());

        Ok(Running::Running::new(tty, child))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, BufRead};

    #[test]
    fn launch_ls() {
        let cmd = Runny::new("/bin/ls -l /etc").unwrap();
        // let cmd = Runny::new("tty").unwrap();
        let mut running = cmd.start().unwrap();
        let mut simple_str = String::new();

        running.read_to_string(&mut simple_str).unwrap();
        println!("Read string: {}", simple_str);
    }

    #[test]
    fn launch_ls_buffered() {
        let cmd = Runny::new("/bin/ls -l /etc").unwrap();
        // let cmd = Runny::new("tty").unwrap();
        let running = cmd.start().unwrap();

        for line in io::BufReader::new(running).lines() {
            match line {
                Ok(l) => println!("Read line: [{}]", l),
                Err(e) => {
                    println!("Read ERROR: {:?}", e);
                    break;
                }
            }
        }
    }
}
