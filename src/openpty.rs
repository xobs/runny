extern crate nix;

use nix::libc;

use std::io;
use std::fs::File;
use std::os::unix::io::FromRawFd;

static PTMX_PATH: &'static str = "/dev/ptmx\0";

pub struct Pty {
    pub master: File,
    pub slave: File,
}

pub fn openpty() -> io::Result<Pty> {

        let master_fd = unsafe {libc::open(PTMX_PATH.as_ptr() as *const i8, libc::O_RDWR) };
        if master_fd == -1 {
            return Err(io::Error::last_os_error());
        }
        if -1 == unsafe { libc::grantpt(master_fd) } {
                unsafe { libc::close(master_fd) };
            return Err(io::Error::last_os_error());
        }

        if -1 == unsafe { libc::unlockpt(master_fd) } {
            unsafe { libc::close(master_fd) };
            return Err(io::Error::last_os_error());
        }

        let mut pts_name = [0; 128];
        if -1 == unsafe { libc::ptsname_r(master_fd, pts_name.as_mut_ptr(), 127) } {
            unsafe { libc::close(master_fd) };
            return Err(io::Error::last_os_error());
        }

        let slave_fd = unsafe { libc::open(pts_name.as_mut_ptr(), libc::O_RDWR) };
        if slave_fd == -1 {
                        unsafe { libc::close(master_fd) };
            return Err(io::Error::last_os_error());
        }

        Ok( Pty {
            master: unsafe { File::from_raw_fd(master_fd) },
            slave: unsafe { File::from_raw_fd(slave_fd) },
        })
}