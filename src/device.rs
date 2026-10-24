//! Descoberta do teclado e envio pelo cabo ou pelo dongle.

use crate::proto::{self, Light};
use anyhow::{anyhow, bail, Context, Result};
use hidapi::{DeviceInfo, HidApi, HidDevice};
use std::{
    thread::sleep,
    time::{Duration, Instant},
};

/// `<cmd_delaytime value="35"/>` do rgb-keyboard.xml do programa oficial.
const DELAY: Duration = Duration::from_millis(35);
const RADIO_TIMEOUT_MS: u64 = 300;
const RADIO_TRIES: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Via {
    Auto,
    Wired,
    Dongle,
}

pub enum Link {
    Wired { ctl: HidDevice, lcd: Option<HidDevice> },
    Dongle { dev: HidDevice },
}

pub struct Keyboard {
    pub link: Link,
    pub desc: String,
}

fn is_wired(d: &DeviceInfo) -> bool {
    d.vendor_id() == proto::VID && proto::PIDS_WIRED.contains(&d.product_id())
}

fn is_dongle(d: &DeviceInfo) -> bool {
    d.vendor_id() == proto::VID && d.product_id() == proto::PID_DONGLE
}

/// Interface pela página de uso; no Linux (hidraw) a página pode vir 0, então usa o número da interface.
fn pick<'a>(api: &'a HidApi, want: fn(&DeviceInfo) -> bool, page: u16, iface: i32) -> Option<&'a DeviceInfo> {
    let mut it = api.device_list().filter(|d| want(d));
    let all: Vec<_> = it.by_ref().collect();
    all.iter().copied().find(|d| d.usage_page() == page)
        .or_else(|| all.iter().copied().find(|d| d.usage_page() == 0 && d.interface_number() == iface))
}

pub fn list(api: &HidApi) -> Vec<String> {
    api.device_list()
        .filter(|d| is_wired(d) || is_dongle(d))
        .map(|d| {
            format!(
                "{:04x}:{:04x} iface {} page 0x{:04X} usage 0x{:02X} rel {:x}.{:02x}  {}",
                d.vendor_id(), d.product_id(), d.interface_number(), d.usage_page(), d.usage(),
                d.release_number() >> 8, d.release_number() & 0xFF,
                d.product_string().unwrap_or("")
            )
        })
        .collect()
}

impl Keyboard {
    pub fn open(api: &HidApi, via: Via) -> Result<Self> {
        let wired = || -> Option<Result<Self>> {
            let ctl = pick(api, is_wired, 0xFF13, 3)?;
            Some((|| {
                let dev = ctl.open_device(api).context("abrir a interface 0xFF13 do teclado")?;
                let lcd = pick(api, is_wired, 0xFF68, 2).and_then(|d| d.open_device(api).ok());
                Ok(Keyboard {
                    desc: format!("cabo USB ({:04x}:{:04x})", ctl.vendor_id(), ctl.product_id()),
                    link: Link::Wired { ctl: dev, lcd },
                })
            })())
        };
        let dongle = || -> Option<Result<Self>> {
            let d = pick(api, is_dongle, 0xFF60, 3)?;
            Some(d.open_device(api).context("abrir a interface 0xFF60 do dongle").map(|dev| Keyboard {
                desc: "dongle 2.4G (0c45:fdfd)".into(),
                link: Link::Dongle { dev },
            }))
        };
        let found = match via {
            Via::Wired => wired(),
            Via::Dongle => dongle(),
            Via::Auto => wired().or_else(dongle),
        };
        found.unwrap_or_else(|| {
            Err(anyhow!(
                "teclado não encontrado ({:?}). Cabo ligado com a chave em USB, ou dongle conectado? \
                 No Linux, instale a regra udev (70-ak820.rules).",
                via
            ))
        })
    }

    pub fn is_wired(&self) -> bool {
        matches!(self.link, Link::Wired { .. })
    }

    // ---- cabo -------------------------------------------------------------------------------

    fn feature(&self, data: &[u8], readback: bool, delay: Duration) -> Result<()> {
        let Link::Wired { ctl, .. } = &self.link else { bail!("só pelo cabo") };
        for chunk in data.chunks(64) {
            sleep(delay);
            let mut buf = [0u8; 65];
            buf[1..1 + chunk.len()].copy_from_slice(chunk);
            ctl.send_feature_report(&buf).context("SET_REPORT")?;
            if readback {
                sleep(delay);
                let mut rb = [0u8; 65];
                ctl.get_feature_report(&mut rb).context("GET_REPORT")?;
            }
        }
        Ok(())
    }

    fn cmd(&self, op: u8, count: u16, extra: &[u8], readback: bool, delay: Duration) -> Result<()> {
        self.feature(&proto::wired_cmd(op, count, extra), readback, delay)
    }

    /// Sessão do programa oficial: 04 18, comando, dados, 04 02 (salvar), 04 F0.
    fn session(&self, op: u8, count: u16, data: &[u8]) -> Result<()> {
        self.cmd(0x18, 0, &[], true, DELAY)?;
        self.cmd(op, count, &[], true, DELAY)?;
        self.feature(data, false, DELAY)?;
        self.cmd(0x02, 0, &[], true, DELAY)?;
        self.cmd(0xF0, 0, &[], false, DELAY)
    }

    fn need_wired(&self, what: &str) -> Result<()> {
        if !self.is_wired() {
            bail!("{what} ainda só funciona pelo cabo (o comando pelo rádio ainda não foi capturado)");
        }
        Ok(())
    }

    // ---- rádio ------------------------------------------------------------------------------

    /// Envia um quadro de 32 bytes e espera o eco do teclado; repete como o programa oficial.
    ///
    /// Toda resposta do dongle chega a todas as conexões abertas (por exemplo, as consultas
    /// `20 01` do programa oficial), então a fila é esvaziada antes e respostas de outros
    /// comandos são ignoradas.
    pub fn radio(&self, frame: &[u8; 32]) -> Result<[u8; 32]> {
        let Link::Dongle { dev } = &self.link else { bail!("só pelo dongle") };
        let mut out = [0u8; 33];
        out[1..].copy_from_slice(frame);
        let mut buf = [0u8; 33];
        while dev.read_timeout(&mut buf, 0)? > 0 {}
        for _ in 0..RADIO_TRIES {
            dev.write(&out).context("enviar ao dongle")?;
            let deadline = Instant::now() + Duration::from_millis(RADIO_TIMEOUT_MS);
            loop {
                let left = deadline.saturating_duration_since(Instant::now()).as_millis() as i32;
                if left <= 0 {
                    break;
                }
                let n = dev.read_timeout(&mut buf, left)?;
                if n == 0 {
                    break;
                }
                // O Windows inclui o report ID 0 na leitura; o Linux (hidraw) não.
                let rx = if n == 33 { &buf[1..33] } else { &buf[..32] };
                if rx[0] == frame[0] && rx[2] == frame[2] {
                    let mut r = [0u8; 32];
                    r.copy_from_slice(rx);
                    return Ok(r);
                }
            }
        }
        bail!("sem resposta do teclado pelo dongle (teclado ligado em 2.4G e perto do dongle?)")
    }

    // ---- operações --------------------------------------------------------------------------

    pub fn set_light(&self, l: &Light) -> Result<()> {
        match &self.link {
            Link::Wired { .. } => self.session(0x13, 1, &proto::wired_light_packet(l)),
            Link::Dongle { .. } => self.radio(&proto::radio_frame(proto::RADIO_LIGHT, 0, &l.payload())).map(|_| ()),
        }
    }

    pub fn info(&self) -> Result<proto::Info> {
        if self.is_wired() {
            bail!("info ainda só funciona pelo dongle");
        }
        Ok(proto::parse_info(&self.radio(&proto::radio_frame(proto::RADIO_INFO, 0, &[]))?))
    }

    /// Troca a função de uma tecla (salva no teclado, vale em todos os modos).
    pub fn remap(&self, key_index: u8, action: &proto::Action) -> Result<()> {
        if self.is_wired() {
            bail!("trocar função ainda só funciona pelo dongle (o comando pelo cabo não foi capturado)");
        }
        self.radio(&proto::radio_frame(proto::RADIO_KEYMAP, key_index, &action.bytes())).map(|_| ())
    }

    pub fn ping(&self) -> Result<String> {
        match &self.link {
            Link::Wired { .. } => {
                self.cmd(0xF5, 9, &[], true, DELAY)?;
                self.cmd(0x02, 0, &[], true, DELAY)?;
                Ok("teclado respondeu pelo cabo".into())
            }
            Link::Dongle { .. } => {
                let t = std::time::Instant::now();
                self.radio(&proto::radio_status_frame())?;
                Ok(format!("teclado respondeu pelo dongle em {} ms", t.elapsed().as_millis()))
            }
        }
    }

    pub fn sync_time(&self) -> Result<()> {
        let now = chrono::Local::now().naive_local();
        match &self.link {
            Link::Wired { .. } => self.session(0x28, 1, &proto::time_packet(now)),
            Link::Dongle { .. } => {
                self.radio(&proto::radio_frame(proto::RADIO_TIME, 0, &proto::radio_time_payload(now))).map(|_| ())
            }
        }
    }

    pub fn set_keys(&self, colors: &[(u8, [u8; 3])], brightness: u8) -> Result<()> {
        self.need_wired("Cor por tecla")?;
        let mut p = [0u8; 64];
        p[0] = 0x80;
        p[9] = brightness;
        p[14] = 0xAA;
        p[15] = 0x55;
        self.session(0x13, 1, &p)?;
        self.session(0x23, 9, &proto::keys_packet(colors))
    }

    /// Um quadro de cor por tecla em tempo real pelo cabo (não é salvo na memória).
    pub fn live(&self, colors: &[(u8, [u8; 3])]) -> Result<()> {
        self.need_wired("Cor em tempo real")?;
        self.cmd(0x20, 8, &[], true, Duration::from_millis(5))?;
        self.feature(&proto::live_packet(colors), false, Duration::from_millis(2))?;
        self.cmd(0x02, 0, &[], true, Duration::from_millis(5))
    }

    pub fn upload_screen(&self, blob: &[u8], mut progress: impl FnMut(usize, usize)) -> Result<()> {
        self.need_wired("Imagem na tela")?;
        let Link::Wired { lcd, .. } = &self.link else { unreachable!() };
        let lcd = lcd.as_ref().ok_or_else(|| anyhow!("interface 0xFF68 da tela não encontrada"))?;
        let n = blob.len() / proto::SCREEN_CHUNK;
        self.cmd(0x18, 0, &[], true, DELAY)?;
        self.cmd(0x72, n as u16, &[0x01], true, DELAY)?;
        for (i, chunk) in blob.chunks(proto::SCREEN_CHUNK).enumerate() {
            let mut out = Vec::with_capacity(proto::SCREEN_CHUNK + 1);
            out.push(0);
            out.extend_from_slice(chunk);
            lcd.write(&out).context("enviar bloco da tela")?;
            let mut ack = [0u8; 65];
            let _ = lcd.read_timeout(&mut ack, 300); // o programa oficial espera até 300 ms
            progress(i + 1, n);
        }
        self.cmd(0x02, 0, &[], true, DELAY)
    }
}
