use std::env;
use std::fs;
use std::path::PathBuf;

pub struct Prompt;

impl Prompt {
    pub fn render(last_status: i32) -> String {
        let user = env::var("USER")
            .or_else(|_| env::var("USERNAME"))
            .unwrap_or_else(|_| "user".to_string());

        let hostname = fs::read_to_string("/etc/hostname")
            .map(|s| s.trim().to_string())
            .or_else(|_| env::var("HOSTNAME"))
            .unwrap_or_else(|_| "localhost".to_string());

        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let home = env::var("HOME").unwrap_or_default();

        let cwd_str = cwd.to_string_lossy();
        let display_path = if !home.is_empty() && cwd_str.starts_with(&home) {
            cwd_str.replacen(&home, "~", 1)
        } else {
            cwd_str.into_owned()
        };

        // ANSI Color Codes
        let green = "\x1b[1;32m";
        let blue = "\x1b[1;34m";
        let red = "\x1b[1;31m";
        let yellow = "\x1b[1;33m";
        let reset = "\x1b[0m";

        let status_indicator = if last_status != 0 {
            format!("{red}[{last_status}]{reset} ")
        } else {
            String::new()
        };

        format!(
            "{green}{user}@{hostname}{reset}:{blue}{display_path}{reset} {status_indicator}{yellow}❯{reset} "
        )
    }
}
