use std::io::{Read, Write};
use std::process::{Command, Stdio};

pub fn get_candidate(prompt: &str) -> Option<String> {
    if prompt.is_empty() {
        return None;
    }
    let mut child = Command::new("bash")
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let mut stdout = child.stderr.take()?;
    write!(stdin, "false && {prompt}\t\t\n").ok()?;
    drop(stdin);
    let mut output = Vec::new();
    stdout.read_to_end(&mut output).ok()?;
    let output = String::from_utf8_lossy(&output);
    child.kill().ok()?;
    Some(
        output
            .split_once("false &&")?
            .1
            .split_once("\n")?
            .0
            .trim()
            .to_owned(),
    )
}
