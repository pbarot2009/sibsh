use std::env;
use std::fs;
use std::path::PathBuf;

pub struct Prompt;

impl Prompt {
    /// Renders the default prompt: `user@host:path ❯ ` with a red `[status]`
    /// marker after a failed command.
    pub fn render(last_status: i32) -> String {
        let (user, hostname, display_path) = Self::parts();
        let status_indicator = Self::status_marker(last_status);

        // ANSI Color Codes
        let green = "\x1b[1;32m";
        let blue = "\x1b[1;34m";
        let reset = "\x1b[0m";
        let yellow = "\x1b[1;33m";

        format!(
            "{green}{user}@{hostname}{reset}:{blue}{display_path}{reset} {status_indicator}{yellow}❯{reset} "
        )
    }

    /// Renders a prompt from a config template. Supported placeholders:
    /// `{user}`, `{host}`, `{cwd}`, `{status}`. Falls back to the default
    /// prompt when no template is set.
    pub fn render_with(template: Option<&str>, last_status: i32) -> String {
        let Some(template) = template else {
            return Self::render(last_status);
        };

        let (user, hostname, display_path) = Self::parts();
        template
            .replace("{user}", &user)
            .replace("{host}", &hostname)
            .replace("{cwd}", &display_path)
            .replace("{status}", &Self::status_marker(last_status))
            + " "
    }

    fn parts() -> (String, String, String) {
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

        (user, hostname, display_path)
    }

    fn status_marker(last_status: i32) -> String {
        if last_status == 0 {
            String::new()
        } else {
            format!("\x1b[1;31m[{last_status}]\x1b[0m ")
        }
    }
}
