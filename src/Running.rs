use chan_signal::Signal;
use tty::{FileDesc, TtyServer};

use std::process::{Command, Child, ExitStatus};
use std::io;
use std::fmt;
use std::fs::File;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};
use std::thread;

pub struct Running {
    tty: TtyServer, // child: Child,
}

impl Running {
    pub fn new(tty: TtyServer) -> Running {
        Running {
            tty: tty,
            //child: child,
        }
    }
    // pub fn wait(&mut self) -> io::Result<ExitStatus> {
    // self.child.wait()
    // }
    //

    pub fn get_interface(&self) -> File {
        // let master_raw = FileDesc::new(self.tty.get_master().as_raw_fd(), true);
        unsafe { File::from_raw_fd(self.tty.get_master().as_raw_fd()) }
    }
}