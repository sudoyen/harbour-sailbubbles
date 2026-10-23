use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use tokio::task::spawn_blocking;

/// Asks the person signing in, on a terminal or through the app.
pub trait Prompter {
    /// Relay address and code; `previous` is the address signed in with before.
    async fn relay(&mut self, previous: Option<&str>) -> Result<(String, String)>;
    async fn account(&mut self) -> Result<(String, String)>;
    /// A code Apple sent to the account's trusted devices.
    async fn device_code(&mut self) -> Result<String>;
    /// Picks one of the trusted numbers, given as (id, last two digits).
    async fn phone(&mut self, phones: &[(u32, String)]) -> Result<u32>;
    async fn sms_code(&mut self) -> Result<String>;
    /// What is going on while nothing is asked.
    async fn working(&mut self, what: &str);
}

pub struct Terminal;

// The tty prompts block, so they run off the async workers.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> io::Result<T> + Send + 'static) -> Result<T> {
    Ok(spawn_blocking(f).await??)
}

impl Prompter for Terminal {
    async fn relay(&mut self, _previous: Option<&str>) -> Result<(String, String)> {
        let host = blocking(|| ask("Relay address (http://<your Mac's IP>:8000)")).await?;
        Ok((host, blocking(|| ask_secret("Relay code")).await?))
    }

    async fn account(&mut self) -> Result<(String, String)> {
        let apple_id = blocking(|| ask("Apple ID")).await?;
        Ok((apple_id, blocking(|| ask_secret("Password")).await?))
    }

    async fn device_code(&mut self) -> Result<String> {
        eprintln!("A code was sent to your trusted Apple devices.");
        blocking(ask_2fa_code).await
    }

    async fn phone(&mut self, phones: &[(u32, String)]) -> Result<u32> {
        for (i, (_, last)) in phones.iter().enumerate() {
            eprintln!("{}: number ending in {last}", i + 1);
        }
        let pick: usize = blocking(|| ask("Send the code to number")).await?.parse().context("not a number")?;
        Ok(phones.get(pick.wrapping_sub(1)).context("no such number")?.0)
    }

    async fn sms_code(&mut self) -> Result<String> {
        blocking(ask_2fa_code).await
    }

    async fn working(&mut self, _what: &str) {}
}

// Prompts use /dev/tty rather than stdin/stdout so answers never come from a
// pipe or script, and nothing typed lands in redirected output or logs.
pub fn ask(question: &str) -> io::Result<String> {
    let mut tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    write!(tty, "{question}: ")?;
    tty.flush()?;
    read_answer(&mut BufReader::new(tty))
}

pub fn ask_secret(question: &str) -> io::Result<String> {
    let echo = EchoOff::new()?;
    let answer = ask(question);
    drop(echo);
    let mut tty = OpenOptions::new().write(true).open("/dev/tty")?;
    writeln!(tty)?;
    answer
}

pub fn ask_2fa_code() -> io::Result<String> {
    loop {
        let answer = ask_secret("Verification code (6 digits)")?;
        if is_2fa_code(&answer) {
            return Ok(answer);
        }
    }
}

fn read_answer(r: &mut impl BufRead) -> io::Result<String> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    Ok(line.trim().to_string())
}

pub fn is_2fa_code(s: &str) -> bool {
    s.len() == 6 && s.bytes().all(|b| b.is_ascii_digit())
}

// Echo is restored on drop, but not if the process is killed mid-prompt;
// `stty echo` fixes the terminal then.
struct EchoOff;

impl EchoOff {
    fn new() -> io::Result<Self> {
        stty("-echo")?;
        Ok(EchoOff)
    }
}

impl Drop for EchoOff {
    fn drop(&mut self) {
        let _ = stty("echo");
    }
}

fn stty(arg: &str) -> io::Result<()> {
    let ok = Command::new("stty")
        .arg(arg)
        .stdin(File::open("/dev/tty")?)
        .stdout(Stdio::null())
        .status()?
        .success();
    if ok { Ok(()) } else { Err(io::Error::other(format!("stty {arg} failed"))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_is_trimmed() {
        assert_eq!(read_answer(&mut "  me@example.com \r\n".as_bytes()).unwrap(), "me@example.com");
    }

    #[test]
    fn eof_is_an_error() {
        assert_eq!(read_answer(&mut "".as_bytes()).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn only_six_digits_are_a_code() {
        assert!(is_2fa_code("012345"));
        for bad in ["", "12345", "1234567", "12 345", "12345a"] {
            assert!(!is_2fa_code(bad), "{bad}");
        }
    }
}
