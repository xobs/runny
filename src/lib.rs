extern crate tty;
extern crate chan_signal;
extern crate libc;
extern crate shlex;

use chan_signal::Signal;
use tty::{FileDesc, TtyServer};

use std::process::{Command, Stdio};
use std::io;
use std::os::unix::io::{AsRawFd, FromRawFd};

pub struct Runny {
    tty: TtyServer,
    cmd: String,
    args: Vec<String>,
    working_directory: Option<String>,
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
            working_directory: None,
        })
    }

    pub fn set_directory(&mut self, wd: &str) {
        self.working_directory = Some(wd.to_string());
    }

    pub fn start(&mut self) -> Result<(), RunnyError> {
        let (mut slave_fd, stdin, stdout, stderr) = match self.tty.take_slave() {
            // TODO: Use pipes if no TTY
            Some(slave_fd) => {
                let fd = slave_fd.as_raw_fd();
                // tty::set_nonblock(&fd);
                // self.stdio = Some(s);
                // Keep the slave FD open until the spawn
                (Some(slave_fd),
                 unsafe { Stdio::from_raw_fd(fd) },
                 unsafe { Stdio::from_raw_fd(fd) },
                 unsafe { Stdio::from_raw_fd(fd) })
            }
            None => (None, Stdio::inherit(), Stdio::inherit(), Stdio::inherit()),
        };
        let master = self.tty.get_master();

        let mut cmd = Command::new(&self.cmd);
        cmd.stdin(stdin)
            .stdout(stdout)
            .stderr(stderr)
            .env_clear()
            .args(self.args.as_slice());
        if let Some(ref wd) = self.working_directory {
            cmd.current_dir(wd);
        }

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
