//! Luz ambiente: o teclado acompanha as cores que o HyperHDR já manda para as fitas de LED.
//!
//! Lê pela API JSON do HyperHDR (TCP, padrão 19444), com `ledcolors` / `ledstream-start`.
//! A resposta chega a cada `interval` ms como
//! `{"command":"ledcolors-ledstream-update","result":{"leds":[r,g,b,r,g,b,…]}}`.
//! Nada muda na configuração do HyperHDR.
//!
//! Pelo cabo, a cor vai em tempo real (não salva). Pelo dongle, só existe o comando de efeito,
//! que o firmware provavelmente salva a cada envio; por isso ele tem um intervalo mínimo e só é
//! mandado quando a cor muda de verdade.

use crate::device::Keyboard;
use crate::proto::{self, Light};
use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub struct Options {
    pub host: String,
    pub port: u16,
    /// Intervalo pedido ao HyperHDR (mínimo dele: 50 ms).
    pub interval_ms: u32,
    /// Pelo dongle: tempo mínimo entre envios ao teclado.
    pub min_gap: Duration,
    /// Diferença mínima (0–255, no canal que mais mudou) para mandar uma cor nova.
    pub threshold: u8,
    pub brightness: u8,
}

/// Média das cores, ignorando LEDs apagados (bordas pretas de filmes não escurecem tudo).
pub fn average(leds: &[u8]) -> Option<[u8; 3]> {
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for px in leds.chunks_exact(3) {
        if px.iter().map(|&v| v as u32).sum::<u32>() < 24 {
            continue;
        }
        for i in 0..3 {
            sum[i] += px[i] as u64;
        }
        n += 1;
    }
    (n > 0).then(|| [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8])
}

fn distance(a: [u8; 3], b: [u8; 3]) -> u8 {
    (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap_or(0)
}

/// Decide quando uma cor nova deve ir para o teclado.
pub struct Gate {
    min_gap: Duration,
    threshold: u8,
    last: Option<([u8; 3], Instant)>,
}

impl Gate {
    pub fn new(min_gap: Duration, threshold: u8) -> Self {
        Gate { min_gap, threshold, last: None }
    }

    pub fn should_send(&mut self, color: [u8; 3], now: Instant) -> bool {
        let send = match self.last {
            None => true,
            Some((c, t)) => now.duration_since(t) >= self.min_gap && distance(c, color) >= self.threshold,
        };
        if send {
            self.last = Some((color, now));
        }
        send
    }
}

fn leds_from(line: &str) -> Option<Vec<u8>> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("command")?.as_str()? != "ledcolors-ledstream-update" {
        return None;
    }
    let arr = v.get("result")?.get("leds")?.as_array()?;
    Some(arr.iter().map(|x| x.as_u64().unwrap_or(0).min(255) as u8).collect())
}

/// Fica conectado ao HyperHDR e repassa as cores até `stop` virar verdadeiro (retorna Ok) ou dar
/// erro (o chamador pode reconectar).
pub fn run(kb: &Keyboard, opt: &Options, stop: &AtomicBool, mut on_color: impl FnMut([u8; 3], bool)) -> Result<()> {
    let addr = format!("{}:{}", opt.host, opt.port);
    let sock = addr
        .to_socket_addrs()
        .with_context(|| format!("endereço inválido {addr}"))?
        .next()
        .with_context(|| format!("endereço inválido {addr}"))?;
    let mut stream =
        TcpStream::connect_timeout(&sock, Duration::from_secs(3)).with_context(|| format!("conectar ao HyperHDR em {addr}"))?;
    // leitura curta para poder parar a qualquer momento
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    let start = serde_json::json!({
        "command": "ledcolors", "subcommand": "ledstream-start", "tan": 1,
        "interval": opt.interval_ms.max(50),
    });
    writeln!(stream, "{start}")?;

    let wired = kb.is_wired();
    // Pelo cabo, sem limite de intervalo (não salva); só ignora variações mínimas.
    let mut gate = if wired { Gate::new(Duration::ZERO, 2) } else { Gate::new(opt.min_gap, opt.threshold) };
    let keys: Vec<u8> = proto::KEYS.iter().map(|k| k.1).collect();
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    let mut idle = Duration::ZERO;
    while !stop.load(Ordering::Relaxed) {
        // read_line acumula o que chegou antes de um timeout, então só limpa depois de uma linha inteira
        match reader.read_line(&mut line) {
            Ok(0) => bail!("o HyperHDR fechou a conexão"),
            Ok(_) => idle = Duration::ZERO,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                idle += Duration::from_millis(500);
                if idle >= Duration::from_secs(15) {
                    bail!("o HyperHDR parou de mandar cores");
                }
                continue;
            }
            Err(e) => return Err(e).context("ler do HyperHDR"),
        }
        let full = std::mem::take(&mut line);
        let Some(leds) = leds_from(&full) else { continue };
        let Some(color) = average(&leds) else { continue };
        if !gate.should_send(color, Instant::now()) {
            continue;
        }
        if wired {
            let colors: Vec<(u8, [u8; 3])> = keys.iter().map(|&k| (k, color)).collect();
            kb.live(&colors)?;
        } else {
            kb.set_light(&Light { mode: 1, rgb: color, rainbow: false, brightness: opt.brightness, speed: 3, direction: 0 })?;
        }
        on_color(color, wired);
    }
    let _ = writeln!(stream, "{}", serde_json::json!({"command": "ledcolors", "subcommand": "ledstream-stop", "tan": 2}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn average_ignores_black() {
        assert_eq!(average(&[0, 0, 0, 200, 100, 0, 0, 0, 0, 100, 50, 0]), Some([150, 75, 0]));
        assert_eq!(average(&[0, 0, 0, 5, 5, 5]), None);
    }

    #[test]
    fn gate_limits_dongle_writes() {
        let t0 = Instant::now();
        let mut g = Gate::new(Duration::from_secs(2), 16);
        assert!(g.should_send([100, 0, 0], t0));
        assert!(!g.should_send([200, 0, 0], t0 + Duration::from_millis(500))); // cedo demais
        assert!(!g.should_send([110, 0, 0], t0 + Duration::from_secs(3))); // mudou pouco
        assert!(g.should_send([200, 0, 0], t0 + Duration::from_secs(3)));
    }

    #[test]
    fn parses_ledstream_update() {
        let line = r#"{"success":true,"command":"ledcolors-ledstream-update","tan":1,"result":{"leds":[255,0,0,0,0,255]}}"#;
        assert_eq!(leds_from(line), Some(vec![255, 0, 0, 0, 0, 255]));
        assert_eq!(leds_from(r#"{"command":"ledcolors-ledstream-start","success":true}"#), None);
    }
}
