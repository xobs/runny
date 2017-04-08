extern crate tty;
extern crate chan_signal;
extern crate libc;
extern crate shlex;

use chan_signal::Signal;
use std::process::Command;
use tty::{FileDesc, TtyServer};
use std::io;

pub struct Runny {
    tty: TtyServer,
    cmd: String,
    args: Vec<String>,
}

pub enum RunnyError {
    RunnyIoError(io::Error),
    NoCommandSpecified,
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

        // Create a new session, tied to stdin (FD number 0)
        let stdin_fd = tty::FileDesc::new(0 as i32, true);
        let tty = TtyServer::new(Some(&stdin_fd))?;
        Ok(Runny {
            tty: tty,
            cmd: cmd,
            args: args,
        })
    }

    pub fn start(&mut self) -> Result<(), RunnyError> {
        let slave = self.tty.take_slave();
        let master = self.tty.get_master();
        Ok(())
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

    #[test]
    fn test_tty() {
        // Get notifications for terminal resizing before any and all other threads!
        let signal = chan_signal::notify(&[Signal::WINCH]);

        let stdin = FileDesc::new(libc::STDIN_FILENO, false);
        let mut server = match TtyServer::new(Some(&stdin)) {
            Ok(s) => s,
            Err(e) => panic!("Error TTY server: {}", e),
        };
        println!("Got PTY {}", server.as_ref().display());
        let proxy = match server.new_client(stdin, Some(signal)) {
            Ok(p) => p,
            Err(e) => panic!("Error TTY client: {}", e),
        };

        let mut cmd = Command::new("/usr/bin/setsid");
        cmd.arg("-c").arg("/bin/sh");
        let process = match server.spawn(cmd) {
            Ok(p) => p,
            Err(e) => panic!("Failed to execute process: {}", e),
        };
        println!("spawned {}", process.id());
        proxy.wait();
        println!("quit");
    }
}
