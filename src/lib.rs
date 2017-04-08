extern crate tty;
extern crate chan_signal;
extern crate libc;

use chan_signal::Signal;
use std::process::Command;
use tty::{FileDesc, TtyServer};
use std::io;

pub struct Runny {
    tty: TtyServer,
}

pub enum RunnyError {
    RunnyIoError(io::Error),
}

impl From<std::io::Error> for RunnyError {
    fn from(kind: std::io::Error) -> Self {
        RunnyError::RunnyIoError(kind)
    }
}

impl Runny {
    pub fn new() -> Result<Runny, RunnyError> {
        let stdin_fd = tty::FileDesc::new(0 as i32, true);
        let tty = TtyServer::new(Some(&stdin_fd))?;
        Ok(Runny { tty: tty })
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
