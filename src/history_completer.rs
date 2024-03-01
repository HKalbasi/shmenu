use std::{fs::OpenOptions, io::Write, sync::OnceLock};

static HISTORY: OnceLock<String> = OnceLock::new();

fn init_history() -> String {
    let home = home::home_dir().unwrap();
    std::fs::read_to_string(home.join(".bash_history")).unwrap_or_default()
}

pub fn get_history_candidate(prompt: &str) -> String {
    if prompt.is_empty() {
        return "".to_owned();
    }
    let history = HISTORY.get_or_init(init_history);
    for candidate in history.lines().rev() {
        if candidate.starts_with(prompt) {
            return candidate.to_owned();
        }
    }
    prompt.to_owned()
}

pub fn get_history_item(index: i32) -> String {
    let Ok(index) = usize::try_from(index) else {
        return "".to_owned();
    };
    let history = HISTORY.get_or_init(init_history);
    history
        .lines()
        .rev()
        .nth(index)
        .unwrap_or_default()
        .to_owned()
}

pub fn add_history_record(prompt: &str) -> Result<(), std::io::Error> {
    let home = home::home_dir().unwrap();
    let mut file = OpenOptions::new()
        .append(true)
        .open(home.join(".bash_history"))?;
    writeln!(file, "{prompt}")?;
    Ok(())
}
