Runny: The Process Runner
=========================

Runny is a Rust crate that allows for easily running processes in their own sessions.  You can read() and write() the resulting Running process, as well as terminate it (and all of its child processes).

On Unix, the child process will be run in its own pty, so it will be unbuffered.

On Windows, TerminateProcess() is used, which immediately stops a process from running, much like SIGKILL.  Runny does not yet send WM_QUIT to the various windows of a program, although this may be available in a future version.

Synopsis
--------

Add this to your Cargo.toml:

    runny = "*"

Then in your code, create a Runny object and start the subprocess:

    let cmd = Runny::new("/bin/bash -c 'echo Hi there, here are some numbers:; seq 1 5;'");
    cmd.set_timeout(Duration::from_secs(5));
    let mut running = cmd.start().unwrap();
    let mut result = String::new();
    running.read_to_string(&mut result).unwrap();
    let exit_code = running.result();
    println!("Result of output: {}.  String: {}", exit_code, result);
