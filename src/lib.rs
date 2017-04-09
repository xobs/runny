extern crate tty;
extern crate chan_signal;
extern crate libc;
extern crate shlex;
extern crate termios;

use termios::{Termios, tcsetattr};
use tty::TtyServer;

use std::process::Command;
use std::io;
use std::fmt;
use std::time::Duration;
use std::os::unix::io::AsRawFd;

pub mod running;

pub struct Runny {
    cmd: String,
    args: Vec<String>,
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
    pub fn new(cmd: &str) -> Result<Runny, RunnyError> {
        let mut args = Self::make_command(cmd)?;
        let cmd = args.remove(0);

        Ok(Runny {
            cmd: cmd,
            args: args,
            working_directory: None,
            timeout: None,
        })
    }

    pub fn set_directory(&mut self, wd: &str) {
        self.working_directory = Some(wd.to_string());
    }

    pub fn set_timeout(&mut self, timeout: &Duration) {
        self.timeout = Some(timeout.clone());
    }

    pub fn start(&self) -> Result<running::Running, RunnyError> {

        // Create a new session, tied to stdin (FD number 0)
        // let stdin_fd = tty::FileDesc::new(0 as i32, false);
        // let mut tty = TtyServer::new(Some(&stdin_fd))?;
        let mut tty = TtyServer::new::<tty::FileDesc>(None)?;

        let mut cmd = Command::new(&self.cmd);
        cmd.env_clear()
            .args(self.args.as_slice());
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
        let child = tty.spawn(cmd)?;
        // thread::spawn(move || proxy.wait());

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
        let cmd = Runny::new("/bin/echo -n 'Launch test echo works'").unwrap();
        let mut running = cmd.start().unwrap();
        let mut simple_str = String::new();

        running.read_to_string(&mut simple_str).unwrap();
        assert_eq!(simple_str, "Launch test echo works");
    }

    #[test]
    fn test_multi_lines() {
        let cmd = Runny::new("/usr/bin/seq 1 5").unwrap();
        let running = cmd.start().unwrap();

        let mut vec = vec![];
        for line in io::BufReader::new(running).lines() {
            vec.push(line.unwrap());
        }
        assert_eq!(vec.len(), 5);
        let vec_parsed: Vec<i32> = vec.iter().map(|x| x.parse().unwrap()).collect();
        assert_eq!(vec_parsed, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn terminate_works() {
        let timeout_secs = 5;

        let cmd = Runny::new("/bin/bash -c 'sleep 1000'").unwrap();
        let mut running = cmd.start().unwrap();

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
        let mut cmd = Runny::new("/bin/bash -c 'echo -n Hi there; sleep 1000; echo -n Bye there'")
            .unwrap();
        cmd.set_timeout(&Duration::from_secs(timeout_secs));

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
        let mut cmd = Runny::new("/bin/bash -c 'echo Input:; read foo; echo Got string: \
                                  -$foo-; sleep 1; echo Cool'")
            .unwrap();
        cmd.set_timeout(&Duration::from_secs(5));
        let mut running = cmd.start().unwrap();

        running.write("bar\n".as_bytes()).unwrap();

        let mut result = String::new();
        running.read_to_string(&mut result).unwrap();
        println!("String: [{:?}]", result);

        running.terminate(None).unwrap();
        assert_eq!(result, "Input:\nGot string: -bar-\nCool\n");
    }
}
