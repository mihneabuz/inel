use crate::helpers::{setup_tracing, temp_file};
use futures::{AsyncBufReadExt, StreamExt};
use std::os::fd::AsRawFd;

mod bufreader;
mod bufwriter;
mod read;
mod write;

#[test]
fn stdio() {
    setup_tracing();

    inel::block_on(async move {
        let stdin = inel::io::stdin();
        let stdout = inel::io::stdout();
        assert_eq!(stdin.as_raw_fd(), libc::STDIN_FILENO);
        assert_eq!(stdout.as_raw_fd(), libc::STDOUT_FILENO);
    });
}
