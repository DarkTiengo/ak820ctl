//! Ganchos: comandos do usuário rodados quando algo acontece com o teclado.
//!
//! Arquivo `hooks.json` em `~/.config/ak820ctl/` (Linux) ou `%APPDATA%\ak820ctl\` (Windows):
//!
//! ```json
//! { "on_connect": "notify-send 'AK820' \"$AK820_DETAIL\"",
//!   "on_disconnect": "/home/eu/bin/meu-script.sh",
//!   "on_error": "",
//!   "on_reset": "" }
//! ```
//!
//! Cada comando roda em segundo plano pelo shell do sistema (`sh -c` / `cmd /C`), com as
//! variáveis `AK820_EVENT` (connect, disconnect, error, reset) e `AK820_DETAIL` (texto do evento).
//! Comando vazio ou ausente = nada acontece.

use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Connect,
    Disconnect,
    Error,
    /// O watchdog reiniciou o dongle.
    Reset,
}

impl Event {
    fn key(self) -> &'static str {
        match self {
            Event::Connect => "on_connect",
            Event::Disconnect => "on_disconnect",
            Event::Error => "on_error",
            Event::Reset => "on_reset",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Event::Connect => "connect",
            Event::Disconnect => "disconnect",
            Event::Error => "error",
            Event::Reset => "reset",
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Hooks {
    cmds: Vec<(Event, String)>,
}

pub fn config_path() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("ak820ctl").join("hooks.json"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|d| d.join("ak820ctl").join("hooks.json"))
    }
}

impl Hooks {
    /// Lê o arquivo de ganchos; sem arquivo (ou com erro de leitura) não há ganchos.
    pub fn load() -> Self {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    pub fn parse(json: &str) -> Self {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Self::default() };
        let cmds = [Event::Connect, Event::Disconnect, Event::Error, Event::Reset]
            .into_iter()
            .filter_map(|e| {
                let c = v.get(e.key())?.as_str()?.trim();
                (!c.is_empty()).then(|| (e, c.to_string()))
            })
            .collect();
        Hooks { cmds }
    }

    pub fn command(&self, e: Event) -> Option<&str> {
        self.cmds.iter().find(|(k, _)| *k == e).map(|(_, c)| c.as_str())
    }

    /// Roda o comando do evento em segundo plano, sem esperar terminar.
    pub fn fire(&self, e: Event, detail: &str) {
        let Some(cmd) = self.command(e) else { return };
        let mut c = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", cmd]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", cmd]);
            c
        };
        let _ = c
            .env("AK820_EVENT", e.name())
            .env("AK820_DETAIL", detail)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hooks() {
        let h = Hooks::parse(r#"{"on_connect":"echo oi","on_disconnect":"  ","on_error":5,"on_reset":"x"}"#);
        assert_eq!(h.command(Event::Connect), Some("echo oi"));
        assert_eq!(h.command(Event::Disconnect), None);
        assert_eq!(h.command(Event::Error), None);
        assert_eq!(h.command(Event::Reset), Some("x"));
        assert!(Hooks::parse("não é json").command(Event::Connect).is_none());
    }
}
