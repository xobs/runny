extern crate shlex;
extern crate nix;

#[cfg(unix)]
use std::process::Child;
use std::process::{Command, Stdio};
use std::io;
use std::env;
use std::fmt;
use std::fs::File;
use std::time::Duration;
use std::collections::HashMap;

#[cfg(unix)]
use std::os::unix::io::FromRawFd;
#[cfg(windows)]
use std::os::windows::io::{FromRawHandle, IntoRawHandle};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[cfg(unix)]
use nix::pty::openpty;
#[cfg(unix)]
use nix::unistd::{dup, pipe2};
#[cfg(unix)]
use nix::sys::termios;
#[cfg(unix)]
use nix::fcntl::O_CLOEXEC;

pub mod running;

pub struct Runny {
    cmd: String,
    working_directory: Option<String>,
    timeout: Option<Duration>,
    path: Vec<String>,
}

pub enum RunnyError {
    RunnyIoError(io::Error),
    NoCommandSpecified,
    #[cfg(unix)]
    NixError(nix::Error),
}

impl fmt::Debug for RunnyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &RunnyError::RunnyIoError(ref e) => write!(f, "I/O error: {:?}", e),
            &RunnyError::NoCommandSpecified => write!(f, "No command was specified"),
            #[cfg(unix)]
            &RunnyError::NixError(ref e) => write!(f, "Nix library error: {:?}", e),
        }
    }
}

impl From<std::io::Error> for RunnyError {
    fn from(kind: std::io::Error) -> Self {
        RunnyError::RunnyIoError(kind)
    }
}

#[cfg(unix)]
impl From<nix::Error> for RunnyError {
    fn from(kind: nix::Error) -> Self {
        RunnyError::NixError(kind)
    }
}

impl Runny {
    pub fn new(cmd: &str) -> Runny {
        Runny {
            cmd: cmd.to_string(),
            working_directory: None,
            timeout: None,
            path: vec![],
        }
    }

    pub fn directory(&mut self, wd: &Option<String>) -> &mut Runny {
        self.working_directory = wd.clone();
        self
    }

    pub fn path(&mut self, path: Vec<String>) -> &mut Runny {
        self.path = path;
        self
    }

    pub fn timeout(&mut self, timeout: Duration) -> &mut Runny {
        self.timeout = Some(timeout);
        self
    }

    /// Spawn a new process connected to the slave TTY
    #[cfg(unix)]
    fn spawn(&self,
             mut cmd: Command,
             slave_fd: i32,
             handles: &mut HashMap<String, File>)
             -> Result<Child, RunnyError> {

        // When reading from the pty, sometimes we get -EIO or -EBADF,
        // which can be ignored.  But Rust really doesn't like this.
        // So send the pty through a pipe, and ignore those errors.
        //
        let (stderr_rx, stderr_tx) = pipe2(O_CLOEXEC)?;

        let stderr = unsafe { File::from_raw_fd(stderr_rx) };
        handles.insert("stderr".to_owned(), stderr);

        let stdin = unsafe { Stdio::from_raw_fd(dup(slave_fd)?) };
        let stdout = unsafe { Stdio::from_raw_fd(slave_fd) };
        let stderr = unsafe { Stdio::from_raw_fd(stderr_tx) };

        let child = cmd.stdin(stdin)
                       .stdout(stdout)
                        // Must close the slave FD to not wait indefinitely the end of the proxy
                       .stderr(stderr)
                        // Don't check the error of setsid because it fails if we're the
                        // process leader already. We just forked so it shouldn't return
                        // error, but ignore it anyway.
                       .before_exec(|| { nix::unistd::setsid().ok(); Ok(()) })
                       .spawn()?;
        Ok(child)
    }

    #[cfg(unix)]
    fn open_session(&self,
                    cmd: Command,
                    mut handles: HashMap<String, File>)
                    -> Result<running::Running, RunnyError> {
        let pty = openpty(None, None)?;

        // Disable character echo.
        let mut termios_master = termios::tcgetattr(pty.master)?;
        termios_master.input_flags &=
            !(termios::IGNBRK | termios::BRKINT | termios::PARMRK | termios::ISTRIP |
              termios::INLCR | termios::IGNCR | termios::ICRNL | termios::IXON);
        termios_master.output_flags &= !termios::OPOST;
        termios_master.local_flags &=
            !(termios::ECHO | termios::ECHONL | termios::ICANON | termios::ISIG | termios::IEXTEN);
        termios_master.control_flags &= !(termios::CSIZE | termios::PARENB);
        termios_master.control_flags |= termios::CS8;
        termios_master.control_chars[termios::SpecialCharacterIndices::VMIN as usize] = 1;
        termios_master.control_chars[termios::SpecialCharacterIndices::VTIME as usize] = 0;
        termios::tcsetattr(pty.master, termios::SetArg::TCSANOW, &termios_master)?;

        let child = self.spawn(cmd, pty.slave, &mut handles)?;

        let stdin = unsafe { File::from_raw_fd(dup(pty.master)?) };
        let stdout = unsafe { File::from_raw_fd(dup(pty.master)?) };
        Ok(running::Running::new(child, stdin, stdout, self.timeout, handles))
    }

    #[cfg(windows)]
    fn open_session(&self,
                    mut cmd: Command,
                    mut handles: HashMap<String, File>)
                    -> Result<running::Running, RunnyError> {
        let mut child =
            cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;

        if self.path.len() > 0 {
            cmd.env("PATH", env::join_paths(&self.path).unwrap());
        }

        // Transmute the Handles into Files.
        let stdin = unsafe { File::from_raw_handle(child.stdin.take().unwrap().into_raw_handle()) };
        let stdout =
            unsafe { File::from_raw_handle(child.stdout.take().unwrap().into_raw_handle()) };
        let stderr =
            unsafe { File::from_raw_handle(child.stderr.take().unwrap().into_raw_handle()) };

        handles.insert("stderr".to_string(), stderr);

        Ok(running::Running::new(child, stdin, stdout, self.timeout, handles))
    }

    pub fn start(&self) -> Result<running::Running, RunnyError> {

        let mut args = Self::make_command(self.cmd.as_str()).unwrap();
        let cmd = args.remove(0);
        let handles = HashMap::new();

        let mut cmd = Command::new(&cmd);
        cmd.args(args.as_slice());
        //        cmd.env_clear();
        if let Some(ref wd) = self.working_directory {
            cmd.current_dir(wd);
        }

        self.open_session(cmd, handles)
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
    use std::io::Read;

    #[cfg(unix)]
    use std::io::{BufRead, Write};
    use std::time::Instant;

    #[cfg(windows)]
    extern crate winapi;
    #[cfg(windows)]
    extern crate user32;

    #[cfg(unix)]
    #[test]
    fn launch_echo() {
        let mut running = Runny::new("/bin/echo -n 'Launch test echo works'").start().unwrap();
        let mut simple_str = String::new();

        running.read_to_string(&mut simple_str).unwrap();
        assert_eq!(simple_str, "Launch test echo works");
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
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

    #[cfg(unix)]
    #[test]
    fn timeout_works() {
        let timeout_secs = 3;
        let start_time = Instant::now();

        let mut running = Runny::new("/bin/bash -c 'echo -n Hi there; sleep 1000; echo -n Bye there'")
            .timeout(Duration::from_secs(timeout_secs))
            .start()
            .unwrap();

        let mut s = String::new();
        running.read_to_string(&mut s).unwrap();
        let end_time = Instant::now();

        // Give one extra second for timeout, to account for plumbing.
        assert!(end_time.duration_since(start_time) < Duration::from_secs(timeout_secs + 1));
        assert!(end_time.duration_since(start_time) > Duration::from_secs(timeout_secs - 1));
        assert_eq!(s, "Hi there");
    }

    #[cfg(unix)]
    #[test]
    fn read_write() {
        let mut running = Runny::new("/bin/bash -c 'echo Input:; read foo; echo Got string: \
                                  -$foo-; sleep 1; echo End'")
            .start()
            .unwrap();
        let mut input = running.take_input();
        let mut output = running.take_output();
        writeln!(input, "bar").unwrap();

        let mut result = String::new();
        output.read_to_string(&mut result).unwrap();

        running.terminate(None).unwrap();
        assert_eq!(result, "Input:\nGot string: -bar-\nEnd\n");
    }

    #[cfg(unix)]
    #[test]
    fn read_write_err() {
        let mut running = Runny::new("/bin/bash -c 'echo Input:; read foo; echo Got string: \
                                  -$foo-; echo -n Error string 1>&2; sleep 1; echo Cool'")
            .timeout(Duration::from_secs(5))
            .start()
            .unwrap();
        let mut input = running.take_input();
        let mut output = running.take_output();
        let mut error = running.take_error();

        writeln!(input, "bar").unwrap();

        let mut result = String::new();
        let mut err_result = String::new();

        output.read_to_string(&mut result).unwrap();
        error.read_to_string(&mut err_result).unwrap();

        running.terminate(None).unwrap();
        println!("stdout: {}", result);
        println!("stderr: {}", err_result);
        assert_eq!(result, "Input:\nGot string: -bar-\nCool\n");
        assert_eq!(err_result, "Error string");
    }

    #[cfg(unix)]
    #[test]
    fn exit_codes() {
        assert_eq!(Runny::new("/bin/true").start().unwrap().result(), 0);
        assert_ne!(Runny::new("/bin/false").start().unwrap().result(), 0);
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
    #[test]
    fn running_waiter_result() {
        let run = Runny::new("/bin/bash -c 'exit 1'").start().unwrap();
        let waiter = run.waiter();
        waiter.wait();
        assert_eq!(waiter.result(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn read_stderr() {
        let mut run = Runny::new("/bin/bash -c 'echo -n error-test 1>&2'").start().unwrap();
        let mut s = String::new();
        let mut stderr = run.take_error();
        stderr.read_to_string(&mut s).unwrap();
        assert_eq!(s, "error-test");
    }

    #[cfg(unix)]
    #[test]
    fn many_commands_true() {
        let runny = Runny::new("/bin/true");
        for _ in 1..100 {
            assert_eq!(runny.start().unwrap().result(), 0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn many_commands_false() {
        let runny = Runny::new("/bin/false");
        for _ in 1..100 {
            assert_ne!(runny.start().unwrap().result(), 0);
        }
    }

    #[test]
    fn invalid_command() {
        let runny = Runny::new("/bin/does/not/exist");
        let running = runny.start();
        assert!(running.is_err());
    }

    #[cfg(windows)]
    #[test]
    fn win_notepad() {
        let timeout_secs = 3;

        let mut cmd = Runny::new("C:\\Windows\\notepad.exe");
        cmd.timeout(Duration::from_secs(timeout_secs));

        let start_time = Instant::now();
        let run = cmd.start().unwrap();
        let waiter = run.waiter();
        waiter.wait();
        let end_time = Instant::now();

        // Give one extra second for timeout, to account for plumbing.
        assert!(end_time.duration_since(start_time) < Duration::from_secs(timeout_secs + 1));
        assert!(end_time.duration_since(start_time) > Duration::from_secs(timeout_secs - 1));
    }

    #[cfg(windows)]
    #[test]
    fn win_notepad_term_delay() {
        use std::thread;

        let timeout_secs = 3;

        let cmd = Runny::new("C:\\Windows\\notepad.exe");

        let run = cmd.start().unwrap();
        thread::sleep(Duration::from_secs(3));

        fn send_key_a(pid: i32) -> self::winapi::minwindef::BOOL {
            use self::winapi::{HWND, LPARAM, WPARAM, DWORD};
            use std::ptr;
            use std::ffi::CString;
            let process_id = pid as self::winapi::LPWORD;

            extern "system" fn enum_windows_callback(hwnd: HWND,
                                                     target_pid: LPARAM)
                                                     -> self::winapi::minwindef::BOOL {
                let mut found_process_id = 0;
                let target_pid = target_pid as DWORD;

                unsafe { self::user32::GetWindowThreadProcessId(hwnd, &mut found_process_id) };

                if found_process_id == target_pid {
                    let class_name = CString::new("EDIT").expect("Couldn't convert class name");
                    let edit_hwnd = unsafe {
                        self::user32::FindWindowExA(hwnd,
                                                    ptr::null_mut(),
                                                    class_name.as_ptr(),
                                                    ptr::null_mut())
                    };
                    unsafe {
                        self::user32::PostMessageW(edit_hwnd,
                                                   self::winapi::WM_CHAR,
                                                   'A' as WPARAM,
                                                   0)
                    };
                }

                // Continue enumerating windows
                1
            }

            // let enum_func_ptr = &mut enum_func as F;
            unsafe { self::user32::EnumWindows(Some(enum_windows_callback), process_id as LPARAM) }
        }

        send_key_a(run.pid());

        let start_time = Instant::now();
        run.terminate(Some(Duration::from_secs(timeout_secs))).unwrap();
        let end_time = Instant::now();

        // Give one extra second for timeout, to account for plumbing.
        assert!(end_time.duration_since(start_time) < Duration::from_secs(timeout_secs + 1));
        assert!(end_time.duration_since(start_time) > Duration::from_secs(timeout_secs - 1));
    }

    #[cfg(windows)]
    #[test]
    fn win_output() {
        let mut running = Runny::new("cmd /c echo Launch test echo works")
            .timeout(Duration::from_secs(2))
            .start()
            .unwrap();
        let mut simple_str = String::new();

        running.read_to_string(&mut simple_str).unwrap();
        assert_eq!(simple_str, "Launch test echo works\r\n");
    }

    #[cfg(windows)]
    #[test]
    fn win_take_output() {
        let mut running = Runny::new("cmd /c echo Launch test echo works")
            .timeout(Duration::from_secs(2))
            .start()
            .unwrap();
        let mut simple_str = String::new();

        let mut output = running.take_output();
        output.read_to_string(&mut simple_str).unwrap();
        assert_eq!(simple_str, "Launch test echo works\r\n");
    }

    #[cfg(windows)]
    #[test]
    fn win_msgbox() {
        let running = Runny::new("powershell \
                                  [Reflection.Assembly]::LoadWithPartialName(\"\"\"System.\
                                  Windows.Forms\"\"\");[Windows.Forms.MessageBox]::\
                                  show(\"\"\"Hello World\"\"\", \"\"\"My PopUp Message Box\"\"\")")
            .timeout(Duration::from_secs(2))
            .start()
            .unwrap();
        use std::thread;
        thread::sleep(Duration::from_secs(2));
        running.terminate(Some(Duration::from_secs(2))).unwrap();
    }
}