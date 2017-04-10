extern crate tty;
extern crate shlex;
extern crate termios;
extern crate nix;

use termios::{Termios, tcsetattr};
use tty::TtyServer;
use nix::unistd::setsid;

use std::process::{Child, Command, Stdio};
use std::io;
use std::fmt;
use std::time::Duration;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::process::CommandExt;

pub mod running;

pub struct Runny {
    cmd: String,
    working_directory: Option<String>,
    timeout: Option<Duration>,
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
    pub fn new(cmd: &str) -> Runny {
        Runny {
            cmd: cmd.to_string(),
            working_directory: None,
            timeout: None,
        }
    }

    pub fn directory(&mut self, wd: &Option<String>) -> &mut Runny {
        self.working_directory = wd.clone();
        self
    }

    pub fn timeout(&mut self, timeout: &Duration) -> &mut Runny {
        self.timeout = Some(timeout.clone());
        self
    }

    /// Spawn a new process connected to the slave TTY
    fn spawn(&self, tty: &mut TtyServer, mut cmd: Command) -> io::Result<Child> {
        match tty.take_slave() {
            Some(slave) => {
                // Force new session
                // TODO: tcsetpgrp
                cmd.stdin(unsafe { Stdio::from_raw_fd(slave.as_raw_fd()) }).
                    stdout(unsafe { Stdio::from_raw_fd(slave.as_raw_fd()) }).
                    // Must close the slave FD to not wait indefinitely the end of the proxy
                    stderr(unsafe { Stdio::from_raw_fd(slave.into_raw_fd()) }).
                    // Don't check the error of setsid because it fails if we're the
                    // process leader already. We just forked so it shouldn't return
                    // error, but ignore it anyway.
                    before_exec(|| { setsid().ok(); Ok(()) }).
                    spawn()
            }
            None => Err(io::Error::new(io::ErrorKind::BrokenPipe, "No TTY slave")),
        }
    }

    pub fn start(&self) -> Result<running::Running, RunnyError> {

        let mut args = Self::make_command(self.cmd.as_str())?;
        let cmd = args.remove(0);

        // Create a new session, tied to stdin (FD number 0)
        // let stdin_fd = tty::FileDesc::new(0 as i32, false);
        // let mut tty = TtyServer::new(Some(&stdin_fd))?;
        let mut tty = TtyServer::new::<tty::FileDesc>(None)?;

        let mut cmd = Command::new(&cmd);
        cmd.env_clear().args(args.as_slice());
        if let Some(ref wd) = self.working_directory {
            cmd.current_dir(wd);
        }

        // Disable character echo.
        let mut termios_master = Termios::from_fd(tty.get_master().as_raw_fd())?;
        termios_master.c_iflag &=
            !(termios::IGNBRK | termios::BRKINT | termios::PARMRK | termios::ISTRIP |
              termios::INLCR | termios::IGNCR | termios::ICRNL | termios::IXON);
        termios_master.c_oflag &= !termios::OPOST;
        termios_master.c_lflag &=
            !(termios::ECHO | termios::ECHONL | termios::ICANON | termios::ISIG | termios::IEXTEN);
        termios_master.c_cflag &= !(termios::CSIZE | termios::PARENB);
        termios_master.c_cflag |= termios::CS8;
        termios_master.c_cc[termios::VMIN] = 1;
        termios_master.c_cc[termios::VTIME] = 0;
        // XXX: cfmakeraw
        tcsetattr(tty.get_master().as_raw_fd(),
                  termios::TCSANOW,
                  &termios_master)?;

        // Spawn a child.  Since we're doing this with a TtyServer,
        // it will have its own session, and will terminate
        let child = self.spawn(&mut tty, cmd)?;

        Ok(running::Running::new(tty, child, self.timeout))
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
    use std::io::{Read, BufRead, Write};
    use std::time::Instant;

    #[test]
    fn launch_echo() {
        // let cmd = Runny::new("/bin/echo -n 'Launch test echo works'").unwrap();
        // let mut running = cmd.start().unwrap();
        let mut running = Runny::new("/bin/echo -n 'Launch test echo works'").start().unwrap();
        let mut simple_str = String::new();

        running.read_to_string(&mut simple_str).unwrap();
        assert_eq!(simple_str, "Launch test echo works");
    }

    #[test]
    fn test_multi_lines() {
        let mut running = Runny::new("/usr/bin/seq 1 5").start().unwrap();

        let mut vec = vec![];
        for line in io::BufReader::new(running.take_output()).lines() {
            vec.push(line.unwrap());
        }
        assert_eq!(vec.len(), 5);
        let vec_parsed: Vec<i32> = vec.iter().map(|x| x.parse().unwrap()).collect();
        assert_eq!(vec_parsed, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn terminate_works() {
        let timeout_secs = 5;

        let mut running = Runny::new("/bin/bash -c 'sleep 1000'").start().unwrap();

        let mut s = String::new();
        let start_time = Instant::now();
        running.terminate(Some(Duration::from_secs(timeout_secs))).unwrap();
        let end_time = Instant::now();
        running.read_to_string(&mut s).unwrap();

        assert_eq!(s, "");
        // Give one extra second for timeout, to account for plumbing.
        assert!(end_time.duration_since(start_time) < Duration::from_secs(timeout_secs + 1));
    }

    #[test]
    fn timeout_works() {
        let timeout_secs = 1;
        let mut cmd = Runny::new("/bin/bash -c 'echo -n Hi there; sleep 1000; echo -n Bye there'");
        cmd.timeout(&Duration::from_secs(timeout_secs));

        let start_time = Instant::now();
        let mut running = cmd.start().unwrap();
        let mut s = String::new();
        running.read_to_string(&mut s).unwrap();
        let end_time = Instant::now();

        assert_eq!(s, "Hi there");

        // Give one extra second for timeout, to account for plumbing.
        assert!(end_time.duration_since(start_time) < Duration::from_secs(timeout_secs + 1));
    }

    #[test]
    fn read_write() {
        let mut running = Runny::new("/bin/bash -c 'echo Input:; read foo; echo Got string: \
                                  -$foo-; sleep 1; echo Cool'")
            .timeout(&Duration::from_secs(5))
            .start()
            .unwrap();
        let mut input = running.take_input();
        let mut output = running.take_output();
        writeln!(input, "bar").unwrap();

        let mut result = String::new();
        output.read_to_string(&mut result).unwrap();
        println!("String: [{:?}]", result);

        running.terminate(None).unwrap();
        assert_eq!(result, "Input:\nGot string: -bar-\nCool\n");
    }

    #[test]
    fn exit_codes() {
        assert_eq!(Runny::new("/bin/true").start().unwrap().result(), 0);
        assert_ne!(Runny::new("/bin/false").start().unwrap().result(), 0);
    }

    #[test]
    fn running_waiter_wait() {
        let cmd = Runny::new("/bin/bash -c 'sleep 2'");

        let start_time = Instant::now();
        let run = cmd.start().unwrap();
        let waiter = run.waiter();
        waiter.wait();
        let end_time = Instant::now();

        // Give one extra second for timeout, to account for plumbing.
        assert!(end_time.duration_since(start_time) < Duration::from_secs(3));
        assert!(end_time.duration_since(start_time) > Duration::from_secs(1));
    }

    #[test]
    fn running_waiter_result() {
        let run = Runny::new("/bin/bash -c 'exit 1'").start().unwrap();
        let waiter = run.waiter();
        waiter.wait();
        assert_eq!(waiter.result(), 1);
    }

}
