//! Watchdog do dongle 2.4G (Linux): reinicia o dongle quando o rádio dele trava.
//!
//! O firmware do dongle (WCH CH58x) não liga o watchdog do chip. Quando o rádio trava, o dongle
//! continua enumerado no USB, mas para de entregar teclas até ser reiniciado (replugar, reiniciar
//! o PC ou `usbreset`). Um dongle travado não se distingue de um teclado parado, então o watchdog
//! reinicia o dongle (`USBDEVFS_RESET`, igual ao `usbreset`) depois de um silêncio que vem após
//! atividade. Reiniciar com o teclado parado é inofensivo: ele some e volta em ~1 s.
//!
//! Política ([`Policy`]):
//! - só arma depois de ver uma tecla vinda do dongle (com o teclado no Bluetooth, não age);
//! - nos primeiros `fast_window` de silêncio, reinicia a cada `silence`;
//! - depois, repete com o intervalo dobrando de `backoff_start` até `backoff_max`;
//! - desarma depois de `give_up` sem atividade, até a próxima tecla.
//!
//! Lê os nós hidraw das interfaces de teclado do dongle, não o evdev: um `EVIOCGRAB` (Steam,
//! gamescope ou InputPlumber no Game Mode) esconderia as teclas no evdev, e o hidraw entrega cada
//! relatório a todos os leitores. A interface de controle (página 0xFF60, e as outras de
//! fabricante) fica de fora: as respostas aos comandos deste programa, como a sincronização com o
//! HyperHDR, chegam por ela e seriam confundidas com atividade.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub silence: Duration,
    pub fast_window: Duration,
    pub backoff_start: Duration,
    pub backoff_max: Duration,
    pub give_up: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            silence: Duration::from_secs(30),
            fast_window: Duration::from_secs(15 * 60),
            backoff_start: Duration::from_secs(60),
            backoff_max: Duration::from_secs(30 * 60),
            give_up: Duration::from_secs(2 * 60 * 60),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Wait,
    /// Reiniciar o dongle; traz há quanto tempo não chega nada.
    Reset(Duration),
    /// Silêncio passou de `give_up`: parar até a próxima tecla.
    Disarm,
}

/// Estado do watchdog, separado do USB para poder ser testado.
#[derive(Debug)]
pub struct State {
    policy: Policy,
    armed: bool,
    last_activity: Instant,
    next_reset: Instant,
    backoff: Duration,
}

impl State {
    pub fn new(policy: Policy, now: Instant) -> Self {
        State { policy, armed: false, last_activity: now, next_reset: now, backoff: policy.backoff_start }
    }

    pub fn armed(&self) -> bool {
        self.armed
    }

    /// Chegou um relatório do dongle. Devolve `true` se isso armou o watchdog.
    pub fn activity(&mut self, now: Instant) -> bool {
        let armed_now = !self.armed;
        self.armed = true;
        self.last_activity = now;
        self.next_reset = now + self.policy.silence;
        self.backoff = self.policy.backoff_start;
        armed_now
    }

    pub fn poll(&mut self, now: Instant) -> Action {
        if !self.armed {
            return Action::Wait;
        }
        let silent = now.saturating_duration_since(self.last_activity);
        if silent > self.policy.give_up {
            self.armed = false;
            return Action::Disarm;
        }
        if now >= self.next_reset {
            return Action::Reset(silent);
        }
        Action::Wait
    }

    /// Agenda o próximo reset depois de um reset feito em `now`.
    pub fn after_reset(&mut self, now: Instant) {
        if now.saturating_duration_since(self.last_activity) < self.policy.fast_window {
            self.next_reset = now + self.policy.silence;
        } else {
            self.next_reset = now + self.backoff;
            self.backoff = (self.backoff * 2).min(self.policy.backoff_max);
        }
    }
}

/// Primeira página de uso (Usage Page) de um descritor de relatório HID.
pub fn first_usage_page(desc: &[u8]) -> Option<u16> {
    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE {
            // item longo: FE, tamanho, tag, dados
            i += 3 + *desc.get(i + 1)? as usize;
            continue;
        }
        let size = match prefix & 0x03 {
            3 => 4,
            n => n as usize,
        };
        let data = desc.get(i + 1..i + 1 + size)?;
        if prefix & 0xFC == 0x04 {
            let mut v = 0u32;
            for (k, b) in data.iter().enumerate() {
                v |= (*b as u32) << (8 * k);
            }
            return Some(v as u16);
        }
        i += 1 + size;
    }
    None
}

/// Interface de fabricante (0xFF00–0xFFFF): controle, não teclas.
pub fn is_vendor_page(page: u16) -> bool {
    page >= 0xFF00
}

#[cfg(target_os = "linux")]
pub use linux::run;

#[cfg(not(target_os = "linux"))]
pub fn run(_policy: Policy, _dry_run: bool) -> anyhow::Result<()> {
    anyhow::bail!("o watchdog do dongle só existe no Linux")
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::hooks::{Event, Hooks};
    use crate::proto;
    use anyhow::{Context, Result};
    use std::fs::{self, File};
    use std::io::{ErrorKind, Read};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf};

    /// `_IO('U', 20)`
    const USBDEVFS_RESET: u64 = 0x5514;

    fn read_trim(p: impl AsRef<Path>) -> Option<String> {
        fs::read_to_string(p).ok().map(|s| s.trim().to_string())
    }

    /// Pasta sysfs do dongle (ex.: /sys/bus/usb/devices/7-1).
    fn find_dongle() -> Option<PathBuf> {
        let want = (format!("{:04x}", proto::VID), format!("{:04x}", proto::PID_DONGLE));
        fs::read_dir("/sys/bus/usb/devices").ok()?.flatten().map(|e| e.path()).find(|d| {
            read_trim(d.join("idVendor")).as_deref() == Some(want.0.as_str())
                && read_trim(d.join("idProduct")).as_deref() == Some(want.1.as_str())
        })
    }

    struct Node {
        file: File,
        path: String,
    }

    /// Abre os hidraw de teclado do dongle (as interfaces de fabricante ficam de fora).
    fn open_nodes(usb: &Path) -> Vec<Node> {
        let Ok(usb) = fs::canonicalize(usb) else { return Vec::new() };
        let Ok(entries) = fs::read_dir("/sys/class/hidraw") else { return Vec::new() };
        let mut nodes: Vec<Node> = entries
            .flatten()
            .filter_map(|e| {
                let dev = fs::canonicalize(e.path().join("device")).ok()?;
                if !dev.starts_with(&usb) {
                    return None;
                }
                let desc = fs::read(dev.join("report_descriptor")).ok()?;
                if first_usage_page(&desc).is_some_and(is_vendor_page) {
                    return None;
                }
                let path = format!("/dev/{}", e.file_name().to_string_lossy());
                match fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&path) {
                    Ok(file) => Some(Node { file, path }),
                    Err(err) => {
                        println!("não abriu {path}: {err}");
                        None
                    }
                }
            })
            .collect();
        nodes.sort_by(|a, b| a.path.cmp(&b.path));
        nodes
    }

    fn reset(usb: &Path, dry_run: bool) -> Result<String> {
        let bus: u32 = read_trim(usb.join("busnum")).context("busnum")?.parse()?;
        let dev: u32 = read_trim(usb.join("devnum")).context("devnum")?.parse()?;
        let node = format!("/dev/bus/usb/{bus:03}/{dev:03}");
        if dry_run {
            return Ok(format!("[dry-run] reiniciaria {node}"));
        }
        let f = fs::OpenOptions::new()
            .write(true)
            .open(&node)
            .with_context(|| format!("abrir {node} (falta a regra udev 70-ak820.rules?)"))?;
        // SAFETY: ioctl sem argumento de ponteiro num descritor válido.
        if unsafe { libc::ioctl(f.as_raw_fd(), USBDEVFS_RESET as _, 0) } < 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| format!("USBDEVFS_RESET em {node}"));
        }
        Ok(format!("dongle reiniciado ({node})"))
    }

    fn secs(d: Duration) -> u64 {
        d.as_secs()
    }

    pub fn run(policy: Policy, dry_run: bool) -> Result<()> {
        println!(
            "watchdog do dongle: silêncio={}s janela={}s backoff={}-{}s desiste={}s{}",
            secs(policy.silence),
            secs(policy.fast_window),
            secs(policy.backoff_start),
            secs(policy.backoff_max),
            secs(policy.give_up),
            if dry_run { " DRY-RUN" } else { "" }
        );
        let hooks = Hooks::load();
        let mut state = State::new(policy, Instant::now());
        let mut usb: Option<PathBuf> = None;
        let mut nodes: Vec<Node> = Vec::new();
        let mut buf = [0u8; 256];
        loop {
            if nodes.is_empty() {
                usb = find_dongle();
                if let Some(u) = &usb {
                    nodes = open_nodes(u);
                    if !nodes.is_empty() {
                        let names: Vec<_> = nodes.iter().map(|n| n.path.as_str()).collect();
                        println!(
                            "monitorando {}: {}",
                            u.file_name().unwrap_or_default().to_string_lossy(),
                            names.join(", ")
                        );
                    }
                }
                if nodes.is_empty() {
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            }

            let mut fds: Vec<libc::pollfd> = nodes
                .iter()
                .map(|n| libc::pollfd { fd: n.file.as_raw_fd(), events: libc::POLLIN, revents: 0 })
                .collect();
            // SAFETY: `fds` é um vetor válido de pollfd com o tamanho informado.
            let r = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 1000) };
            let now = Instant::now();
            if r < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() != ErrorKind::Interrupted {
                    return Err(e).context("poll");
                }
            }
            let mut gone = false;
            for (n, p) in nodes.iter_mut().zip(&fds) {
                if p.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    gone = true;
                    continue;
                }
                if p.revents & libc::POLLIN == 0 {
                    continue;
                }
                match n.file.read(&mut buf) {
                    Ok(0) => gone = true,
                    Ok(len) => {
                        if state.activity(now) {
                            let hex: String = buf[..len.min(16)].iter().map(|b| format!("{b:02x}")).collect();
                            println!("atividade no dongle ({}: {hex}): watchdog armado", n.path);
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                    Err(_) => gone = true,
                }
            }
            if gone {
                // desplugado ou reiniciado: procura de novo
                nodes.clear();
                continue;
            }

            match state.poll(now) {
                Action::Wait => {}
                Action::Disarm => {
                    println!("{}s sem atividade: watchdog desarmado até a próxima tecla", secs(policy.give_up));
                }
                Action::Reset(silent) => {
                    println!("{}s sem eventos do dongle", secs(silent));
                    let u = usb.as_deref().expect("dongle encontrado acima");
                    match reset(u, dry_run) {
                        Ok(msg) => {
                            println!("{msg}");
                            if !dry_run {
                                hooks.fire(Event::Reset, &format!("{}s sem eventos", secs(silent)));
                            }
                        }
                        Err(e) => {
                            println!("falha no reset: {e:#}");
                            hooks.fire(Event::Error, &format!("watchdog: {e:#}"));
                        }
                    }
                    nodes.clear();
                    state.after_reset(now);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simula silêncio total depois de uma tecla e devolve os instantes dos resets.
    fn reset_times(policy: Policy) -> Vec<Duration> {
        let t0 = Instant::now();
        let mut s = State::new(policy, t0);
        assert!(s.activity(t0));
        let mut out = Vec::new();
        let mut t = t0;
        while s.armed() {
            t += Duration::from_secs(1);
            match s.poll(t) {
                Action::Reset(_) => {
                    out.push(t - t0);
                    s.after_reset(t);
                }
                Action::Disarm | Action::Wait => {}
            }
        }
        out
    }

    #[test]
    fn schedule_matches_policy() {
        let r = reset_times(Policy::default());
        let m = |min: u64| Duration::from_secs(min * 60);
        assert_eq!(r[0], Duration::from_secs(30));
        assert_eq!(r[1], Duration::from_secs(60));
        // a cada 30 s até 15 min, depois 16, 18, 22, 30, 46 e então a cada 30 min
        assert_eq!(r.iter().filter(|d| **d <= m(15)).count(), 30);
        let after: Vec<_> = r.iter().filter(|d| **d > m(15)).map(|d| d.as_secs() / 60).collect();
        assert_eq!(after, [16, 18, 22, 30, 46, 76, 106]);
        assert_eq!(r.len(), 37);
    }

    #[test]
    fn activity_rearms_and_resets_backoff() {
        let p = Policy::default();
        let t0 = Instant::now();
        let mut s = State::new(p, t0);
        assert_eq!(s.poll(t0 + Duration::from_secs(999)), Action::Wait, "não age antes da 1ª tecla");
        assert!(s.activity(t0));
        assert!(!s.activity(t0), "já armado");
        assert_eq!(s.poll(t0 + Duration::from_secs(29)), Action::Wait);
        assert!(matches!(s.poll(t0 + Duration::from_secs(30)), Action::Reset(_)));
        s.after_reset(t0 + Duration::from_secs(30));
        let t1 = t0 + Duration::from_secs(40);
        s.activity(t1);
        assert_eq!(s.poll(t1 + Duration::from_secs(29)), Action::Wait, "tecla adia o próximo reset");
        assert_eq!(s.poll(t1 + p.give_up + Duration::from_secs(1)), Action::Disarm);
        assert!(!s.armed());
    }

    #[test]
    fn usage_page_of_descriptor() {
        // teclado: Usage Page (Generic Desktop)
        assert_eq!(first_usage_page(&[0x05, 0x01, 0x09, 0x06, 0xA1, 0x01]), Some(0x0001));
        // fabricante: Usage Page (0xFF60), item de 2 bytes
        assert_eq!(first_usage_page(&[0x06, 0x60, 0xFF, 0x09, 0x61]), Some(0xFF60));
        // itens antes da página são pulados
        assert_eq!(first_usage_page(&[0x85, 0x05, 0x05, 0x0C]), Some(0x000C));
        assert_eq!(first_usage_page(&[0x09]), None);
        assert!(is_vendor_page(0xFF60) && !is_vendor_page(0x000C));
    }
}
