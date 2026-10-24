//! Montagem dos pacotes do firmware original do AK820 Pro.
//!
//! Cabo: feature reports de 64 bytes na interface 0xFF13 (sessões `04 18 … 04 02`) e
//! relatórios de 4096 bytes na 0xFF68 (tela). Protocolo do cabo documentado por
//! hcristosm/ak820-linux (MIT), a partir do DeviceDriver.exe v1.0.0.5.
//!
//! Rádio (dongle 0C45:FDFD, interface 0xFF60): relatórios de 32 bytes
//! `cmd, len, arg, dados[len]…, soma` com a soma dos bytes 0..=30 no byte 31. Capturado
//! com USBPcap em 2026-10-22:
//!   02 00 00          informações (resposta: VID, PID, versão do firmware)
//!   03 04 <tecla>     função da tecla: `02 mod uso 00` (teclado) ou `03 uso_lo uso_hi 00` (mídia)
//!   05 10 00          luz: os mesmos 16 bytes de dados do cabo
//!   0C 10 00          relógio: os 16 primeiros bytes do pacote de relógio do cabo, com AA 55 em 14..15
//!   20 01             consulta de estado (o programa oficial manda a cada 15 s)

use chrono::{Datelike, NaiveDateTime, Timelike};

pub const VID: u16 = 0x0C45;
pub const PIDS_WIRED: [u16; 2] = [0x8009, 0x800A];
pub const PID_DONGLE: u16 = 0xFDFD;

pub const W: u32 = 128;
pub const H: u32 = 128;
pub const SCREEN_HEAD: usize = 256;
pub const SCREEN_MAX_FRAMES: usize = 140;
pub const SCREEN_CHUNK: usize = 4096;

pub const RADIO_INFO: u8 = 0x02;
pub const RADIO_KEYMAP: u8 = 0x03;
pub const RADIO_LIGHT: u8 = 0x05;
pub const RADIO_TIME: u8 = 0x0C;
pub const RADIO_STATUS: u8 = 0x20;

/// id → (nome, flags). Flags do programa oficial: 1=brilho 2=velocidade 4=direção horizontal
/// 8=direção vertical 0x10=cor 0x20=arco-íris.
pub const MODES: &[(u8, &str, u8)] = &[
    (0, "off", 0x00), (1, "static", 0x31), (2, "singleon", 0x33), (3, "singleoff", 0x33),
    (4, "glittering", 0x33), (5, "falling", 0x33), (6, "colourful", 0x03), (7, "breath", 0x33),
    (8, "spectrum", 0x03), (9, "outward", 0x33), (10, "scrolling", 0x3B), (11, "rolling", 0x37),
    (12, "rotating", 0x37), (13, "explode", 0x33), (14, "launch", 0x33), (15, "ripples", 0x33),
    (16, "flowing", 0x37), (17, "pulsating", 0x33), (18, "tilt", 0x37), (19, "shuttle", 0x33),
];

pub fn mode_by_name(s: &str) -> Option<u8> {
    if let Ok(n) = s.parse::<u8>() {
        return MODES.iter().any(|m| m.0 == n).then_some(n);
    }
    MODES.iter().find(|m| m.1.eq_ignore_ascii_case(s)).map(|m| m.0)
}

/// Nome da tecla → (índice da posição, uso HID padrão). O índice vem de
/// layouts/rgb-keyboard.xml do programa oficial e é o mesmo para LED e para troca de função
/// (capturado: Esc = 1, F5 = 6). Uso 0 = sem uso HID próprio (Fn).
pub const KEYS: &[(&str, u8, u8)] = &[
    ("esc", 1, 0x29), ("f1", 2, 0x3A), ("f2", 3, 0x3B), ("f3", 4, 0x3C), ("f4", 5, 0x3D), ("f5", 6, 0x3E),
    ("f6", 7, 0x3F), ("f7", 8, 0x40), ("f8", 9, 0x41), ("f9", 10, 0x42), ("f10", 11, 0x43), ("f11", 12, 0x44),
    ("f12", 13, 0x45), ("delete", 119, 0x4C),
    ("grave", 19, 0x35), ("1", 20, 0x1E), ("2", 21, 0x1F), ("3", 22, 0x20), ("4", 23, 0x21), ("5", 24, 0x22),
    ("6", 25, 0x23), ("7", 26, 0x24), ("8", 27, 0x25), ("9", 28, 0x26), ("0", 29, 0x27), ("minus", 30, 0x2D),
    ("equal", 31, 0x2E), ("backspace", 103, 0x2A), ("home", 117, 0x4A),
    ("tab", 37, 0x2B), ("q", 38, 0x14), ("w", 39, 0x1A), ("e", 40, 0x08), ("r", 41, 0x15), ("t", 42, 0x17),
    ("y", 43, 0x1C), ("u", 44, 0x18), ("i", 45, 0x0C), ("o", 46, 0x12), ("p", 47, 0x13), ("lbracket", 48, 0x2F),
    ("rbracket", 49, 0x30), ("backslash", 67, 0x31), ("pgup", 118, 0x4B),
    ("capslock", 55, 0x39), ("a", 56, 0x04), ("s", 57, 0x16), ("d", 58, 0x07), ("f", 59, 0x09), ("g", 60, 0x0A),
    ("h", 61, 0x0B), ("j", 62, 0x0D), ("k", 63, 0x0E), ("l", 64, 0x0F), ("semicolon", 65, 0x33),
    ("quote", 66, 0x34), ("enter", 85, 0x28), ("pgdn", 121, 0x4E),
    ("lshift", 73, 0xE1), ("z", 74, 0x1D), ("x", 75, 0x1B), ("c", 76, 0x06), ("v", 77, 0x19), ("b", 78, 0x05),
    ("n", 79, 0x11), ("m", 80, 0x10), ("comma", 81, 0x36), ("period", 82, 0x37), ("slash", 83, 0x38),
    ("rshift", 84, 0xE5), ("up", 101, 0x52),
    ("lctrl", 91, 0xE0), ("win", 92, 0xE3), ("lalt", 93, 0xE2), ("space", 94, 0x2C), ("ralt", 95, 0xE6),
    ("fn", 96, 0x00), ("rctrl", 98, 0xE4), ("left", 99, 0x50), ("down", 100, 0x51), ("right", 102, 0x4F),
];

pub fn key_index(name: &str) -> Option<u8> {
    key(name).map(|k| k.1)
}

pub fn key(name: &str) -> Option<&'static (&'static str, u8, u8)> {
    KEYS.iter().find(|k| k.0.eq_ignore_ascii_case(name))
}

/// Teclas de mídia (HID Consumer Page).
pub const MEDIA: &[(&str, u16)] = &[
    ("volup", 0x00E9), ("voldown", 0x00EA), ("mute", 0x00E2), ("play", 0x00CD), ("next", 0x00B5),
    ("prev", 0x00B6), ("stop", 0x00B7), ("calc", 0x0192), ("mail", 0x018A), ("browser", 0x0196),
    ("mycomputer", 0x0194),
];

/// Função de uma tecla no firmware original.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Tecla comum: modificadores (bits do HID) + uso HID.
    Key { mods: u8, usage: u8 },
    /// Tecla de mídia (Consumer Page).
    Media(u16),
}

impl Action {
    pub fn bytes(&self) -> [u8; 4] {
        match *self {
            Action::Key { mods, usage } => [0x02, mods, usage, 0],
            Action::Media(u) => [0x03, (u & 0xFF) as u8, (u >> 8) as u8, 0],
        }
    }
}

/// Lê `default`, `media:volup`, `ctrl+shift+t`, `key:0x3a` ou um nome de tecla (`f5`, `a`…).
pub fn parse_action(spec: &str, default_usage: u8) -> Option<Action> {
    let s = spec.to_ascii_lowercase();
    if s == "default" {
        return Some(Action::Key { mods: 0, usage: default_usage });
    }
    if let Some(m) = s.strip_prefix("media:") {
        return MEDIA.iter().find(|x| x.0 == m).map(|x| Action::Media(x.1));
    }
    let mut mods = 0u8;
    let mut parts: Vec<&str> = s.split('+').collect();
    let last = parts.pop()?;
    for p in parts {
        mods |= match p {
            "ctrl" => 0x01,
            "shift" => 0x02,
            "alt" => 0x04,
            "win" | "gui" => 0x08,
            _ => return None,
        };
    }
    let usage = if let Some(h) = last.strip_prefix("key:0x") {
        u8::from_str_radix(h, 16).ok()?
    } else {
        key(last).map(|k| k.2).filter(|u| *u != 0)?
    };
    Some(Action::Key { mods, usage })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub mode: u8,
    pub rgb: [u8; 3],
    pub rainbow: bool,
    pub brightness: u8,
    pub speed: u8,
    pub direction: u8,
}

impl Light {
    /// Os 16 bytes de dados de luz, iguais no cabo e no rádio.
    pub fn payload(&self) -> [u8; 16] {
        let mut p = [0u8; 16];
        p[0] = self.mode;
        if self.mode != 0 {
            p[1..4].copy_from_slice(&self.rgb);
            p[8] = self.rainbow as u8;
            p[9] = self.brightness;
            p[10] = self.speed;
            p[11] = self.direction;
        }
        p[14] = 0xAA;
        p[15] = 0x55;
        p
    }
}

pub fn wired_light_packet(l: &Light) -> [u8; 64] {
    let mut p = [0u8; 64];
    p[..16].copy_from_slice(&l.payload());
    p
}

/// Cabeçalho de comando do cabo: `04 op extra… ; [8..9] = número de blocos que seguem`.
pub fn wired_cmd(op: u8, count: u16, extra: &[u8]) -> [u8; 64] {
    let mut p = [0u8; 64];
    p[0] = 0x04;
    p[1] = op;
    p[2..2 + extra.len()].copy_from_slice(extra);
    p[8] = (count & 0xFF) as u8;
    p[9] = (count >> 8) as u8;
    p
}

pub fn radio_frame(cmd: u8, arg: u8, payload: &[u8]) -> [u8; 32] {
    assert!(payload.len() <= 28, "dados do rádio limitados a 28 bytes");
    let mut f = [0u8; 32];
    f[0] = cmd;
    f[1] = payload.len() as u8;
    f[2] = arg;
    f[3..3 + payload.len()].copy_from_slice(payload);
    f[31] = radio_checksum(&f);
    f
}

/// Relógio pelo rádio: os 16 primeiros bytes do pacote do cabo, com AA 55 no fim.
pub fn radio_time_payload(t: NaiveDateTime) -> [u8; 16] {
    let mut p = [0u8; 16];
    p.copy_from_slice(&time_packet(t)[..16]);
    p[14] = 0xAA;
    p[15] = 0x55;
    p
}

#[derive(Debug, PartialEq)]
pub struct Info {
    pub vid: u16,
    pub pid: u16,
    pub version: u16,
}

/// Resposta de `02 00`: `02 00 00 40 30 00 00 45 0c 0a 80 10 01 ff ff …`.
pub fn parse_info(r: &[u8; 32]) -> Info {
    let le = |i: usize| u16::from_le_bytes([r[i], r[i + 1]]);
    Info { vid: le(7), pid: le(9), version: le(11) }
}

/// Consulta de estado que o programa oficial manda a cada 15 s: `20 01 00… 21`.
pub fn radio_status_frame() -> [u8; 32] {
    let mut f = [0u8; 32];
    f[0] = RADIO_STATUS;
    f[1] = 0x01;
    f[31] = radio_checksum(&f);
    f
}

pub fn radio_checksum(f: &[u8; 32]) -> u8 {
    f[..31].iter().fold(0u8, |s, b| s.wrapping_add(*b))
}

pub fn time_packet(t: NaiveDateTime) -> [u8; 64] {
    let mut p = [0u8; 64];
    p[1] = 1;
    p[2] = 0x5A;
    p[3] = (t.year() % 2000) as u8;
    p[4] = t.month() as u8;
    p[5] = t.day() as u8;
    p[6] = t.hour() as u8;
    p[7] = t.minute() as u8;
    p[8] = t.second() as u8;
    p[10] = t.weekday().num_days_from_sunday() as u8;
    p[62] = 0xAA;
    p[63] = 0x55;
    p
}

/// {índice: rgb} → 576 bytes (144 posições de índice,r,g,b) terminando em AA 55.
pub fn keys_packet(colors: &[(u8, [u8; 3])]) -> Vec<u8> {
    let mut p = vec![0u8; 0x240];
    for &(idx, [r, g, b]) in colors {
        let o = idx as usize * 4;
        p[o..o + 4].copy_from_slice(&[idx, r, g, b]);
    }
    p[0x23E] = 0xAA;
    p[0x23F] = 0x55;
    p
}

/// Cor por tecla em tempo real pelo cabo (modo música do programa oficial, não salva):
/// até 128 entradas (índice, r, g, b), 512 bytes.
pub fn live_packet(colors: &[(u8, [u8; 3])]) -> Vec<u8> {
    let mut sorted = colors.to_vec();
    sorted.sort_by_key(|c| c.0);
    let mut p: Vec<u8> = sorted.iter().take(128).flat_map(|&(i, [r, g, b])| [i, r, g, b]).collect();
    p.resize(512, 0);
    p
}

pub fn rgb565(rgb: &[u8]) -> Vec<u8> {
    rgb.chunks_exact(3)
        .flat_map(|px| {
            let v = ((px[0] as u16 & 0xF8) << 8) | ((px[1] as u16 & 0xFC) << 3) | (px[2] as u16 >> 3);
            v.to_le_bytes()
        })
        .collect()
}

/// frames: (rgb565, atraso_ms) → cabeçalho + quadros, completado com 0xFF até blocos de 4 KiB.
pub fn screen_blob(frames: &[(Vec<u8>, u32)]) -> Vec<u8> {
    let frames = &frames[..frames.len().min(SCREEN_MAX_FRAMES)];
    let mut blob = vec![0xFFu8; SCREEN_HEAD];
    blob[0] = frames.len() as u8;
    for (i, (_, ms)) in frames.iter().enumerate() {
        blob[1 + i] = (ms / 2).clamp(1, 255) as u8;
    }
    for (f, _) in frames {
        blob.extend_from_slice(f);
    }
    let pad = (SCREEN_CHUNK - blob.len() % SCREEN_CHUNK) % SCREEN_CHUNK;
    blob.resize(blob.len() + pad, 0xFF);
    blob
}

pub fn parse_hex_rgb(s: &str) -> Option<[u8; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_matches_captures() {
        // Cabo, 2026-10-22: azul estático, brilho 5, velocidade 3.
        let blue = Light { mode: 1, rgb: [0, 0, 0xFF], rainbow: false, brightness: 5, speed: 3, direction: 0 };
        assert_eq!(
            &wired_light_packet(&blue)[..16],
            &[0x01, 0, 0, 0xFF, 0, 0, 0, 0, 0, 0x05, 0x03, 0, 0, 0, 0xAA, 0x55]
        );
        // Rádio, 2026-10-22: vermelho com arco-íris ligado → `05 10 00 01 ff …` soma 1D.
        let red = Light { mode: 1, rgb: [0xFF, 0, 0], rainbow: true, brightness: 5, speed: 3, direction: 0 };
        let f = radio_frame(RADIO_LIGHT, 0, &red.payload());
        assert_eq!(&f[..3], &[0x05, 0x10, 0x00]);
        assert_eq!(&f[3..19], &red.payload());
        assert_eq!(f[31], 0x1D);
        // Rádio: azul, brilho 1 → soma 18.
        let dim = Light { brightness: 1, ..blue };
        assert_eq!(radio_frame(RADIO_LIGHT, 0, &dim.payload())[31], 0x18);
    }

    #[test]
    fn radio_captures_2() {
        // Informações: TX `02 00 … 02`.
        assert_eq!(radio_frame(RADIO_INFO, 0, &[])[31], 0x02);
        let mut r = [0u8; 32];
        r[..15].copy_from_slice(&[0x02, 0, 0, 0x40, 0x30, 0, 0, 0x45, 0x0C, 0x0A, 0x80, 0x10, 0x01, 0xFF, 0xFF]);
        assert_eq!(parse_info(&r), Info { vid: 0x0C45, pid: 0x800A, version: 0x0110 });
        // Relógio 2026-10-22 08:35:10, quinta → `0c 10 00 00 01 5a 1a 0a 16 08 23 0a 00 04 …` soma E9.
        let t = NaiveDateTime::parse_from_str("2026-10-22 08:35:10", "%Y-%m-%d %H:%M:%S").unwrap();
        let f = radio_frame(RADIO_TIME, 0, &radio_time_payload(t));
        assert_eq!(&f[..19], &[0x0C, 0x10, 0x00, 0x00, 0x01, 0x5A, 0x1A, 0x0A, 0x16, 0x08, 0x23, 0x0A, 0x00,
                               0x04, 0x00, 0x00, 0x00, 0xAA, 0x55]);
        assert_eq!(f[31], 0xE9);
        // F5 → Volume +: `03 04 06 03 e9 00 00` soma F9; F5 de volta: `03 04 06 02 00 3e 00` soma 4D.
        let vol = parse_action("media:volup", 0x3E).unwrap();
        let f = radio_frame(RADIO_KEYMAP, key_index("f5").unwrap(), &vol.bytes());
        assert_eq!(&f[..7], &[0x03, 0x04, 0x06, 0x03, 0xE9, 0x00, 0x00]);
        assert_eq!(f[31], 0xF9);
        let def = parse_action("default", key("f5").unwrap().2).unwrap();
        assert_eq!(radio_frame(RADIO_KEYMAP, 6, &def.bytes())[31], 0x4D);
        // Esc padrão: `03 04 01 02 00 29 00` soma 33.
        assert_eq!(radio_frame(RADIO_KEYMAP, 1, &parse_action("esc", 0).unwrap().bytes())[31], 0x33);
        assert_eq!(parse_action("ctrl+shift+t", 0), Some(Action::Key { mods: 0x03, usage: 0x17 }));
        assert_eq!(parse_action("fn", 0), None);
        assert_eq!(parse_action("media:nada", 0), None);
    }

    #[test]
    fn status_frame_matches_capture() {
        let f = radio_status_frame();
        assert_eq!(&f[..2], &[0x20, 0x01]);
        assert_eq!(f[31], 0x21);
    }

    #[test]
    fn time_matches_capture() {
        let t = NaiveDateTime::parse_from_str("2026-10-22 08:09:32", "%Y-%m-%d %H:%M:%S").unwrap();
        let p = time_packet(t);
        assert_eq!(&p[..11], &[0x00, 0x01, 0x5A, 0x1A, 0x0A, 0x16, 0x08, 0x09, 0x20, 0x00, 0x04]);
        assert_eq!(&p[62..], &[0xAA, 0x55]);
    }

    #[test]
    fn packets() {
        let k = keys_packet(&[(1, [255, 0, 0]), (121, [0, 0, 255])]);
        assert_eq!(k.len(), 576);
        assert_eq!(&k[4..8], &[1, 255, 0, 0]);
        assert_eq!(&k[484..488], &[121, 0, 0, 255]);
        assert_eq!(&k[574..], &[0xAA, 0x55]);
        assert_eq!(rgb565(&[255, 0, 0]), vec![0x00, 0xF8]);
        assert_eq!(rgb565(&[0, 0, 255]), vec![0x1F, 0x00]);
        let b = screen_blob(&[(vec![0; 128 * 128 * 2], 100), (vec![0; 128 * 128 * 2], 100)]);
        assert_eq!(b.len(), 69632);
        assert_eq!(&b[..4], &[2, 0x32, 0x32, 0xFF]);
        assert_eq!(wired_cmd(0x13, 1, &[])[..10], [4, 0x13, 0, 0, 0, 0, 0, 0, 1, 0]);
        assert_eq!(mode_by_name("Static"), Some(1));
        assert_eq!(mode_by_name("7"), Some(7));
        assert_eq!(mode_by_name("99"), None);
        assert_eq!(parse_hex_rgb("#ff0080"), Some([255, 0, 128]));
    }
}
