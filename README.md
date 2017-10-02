Runny: The Process Runner
=========================

Runny is a Rust crate that allows for easily running processes in their own sessions.  You can read() and write() the resulting Running process, as well as terminate it (and all of its child processes).

On Unix, the child process will be run in its own pty, so it will be unbuffered.

On Windows, the system first sends a WM\_QUIT message, which behaves a lot like SIGTERM.  If the process does not quit, TerminateProcess() is used as an analog to SIGKILL.

Synopsis
--------

Add this to your Cargo.toml:

    runny = "*"

Then in your code, create a Runny object and start the subprocess:

    let running = Runny::new("/bin/bash -c 'echo Hi there, here are some numbers:; seq 1 5;'").start().unwrap();
    let exit_code = running.result();
    println!("Result of command: {}", exit_code);

You can also read from the output and write to the input.  Note that stdout and stderr are merged into "output":

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

The Runny command supports setting variables such as Timeout.  If a timeout is set, then the program will exit when the timeout expires, if it hasn't already quit:

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
