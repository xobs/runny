use chan_signal::Signal;
use tty::{FileDesc, TtyServer};

use std::process::{Command, Child, ExitStatus};
use std::io::{self, Read, Result};
use std::fmt;
use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};
use std::thread;

pub struct Running {
    tty: TtyServer,
    child: Child,
    stream: File,
}

impl Running {
    pub fn new(tty: TtyServer, child: Child) -> Running {
        let file = unsafe { File::from_raw_fd(tty.get_master().as_raw_fd()) };
        Running {
            tty: tty,
            child: child,
            stream: file,
        }
    }
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
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