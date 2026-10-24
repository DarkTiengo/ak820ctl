use ak820ctl::ambient;
use ak820ctl::device::{self, Keyboard, Via};
use ak820ctl::proto::{self, Light};
use ak820ctl::screen;
use ak820ctl::watchdog;
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use hidapi::HidApi;

#[derive(Parser)]
#[command(version, about = "Controle do AJAZZ AK820 Pro (firmware original), pelo cabo ou pelo dongle 2.4G")]
struct Cli {
    /// Por onde falar com o teclado
    #[arg(long, value_enum, default_value_t = Via::Auto, global = true)]
    via: Via,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Lista as interfaces do teclado e do dongle encontradas
    List,
    /// Confere se o teclado responde (só leitura)
    Ping,
    /// Efeito de luz embutido
    Mode {
        /// Nome ou número (veja `modes`)
        mode: String,
        /// Cor RRGGBB (desliga o arco-íris)
        #[arg(short, long)]
        color: Option<String>,
        #[arg(short, long, default_value_t = 5, value_parser = clap::value_parser!(u8).range(0..=5))]
        brightness: u8,
        #[arg(short, long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(0..=5))]
        speed: u8,
        /// 0 = esquerda→direita / para baixo, 1 = ao contrário
        #[arg(short, long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=1))]
        direction: u8,
    },
    /// Lista os efeitos
    Modes,
    /// Cor fixa por tecla, ex.: keys --all 000020 w=ff0000 a=ff0000 (cabo)
    Keys {
        assign: Vec<String>,
        #[arg(long)]
        all: Option<String>,
        #[arg(short, long, default_value_t = 5, value_parser = clap::value_parser!(u8).range(0..=5))]
        brightness: u8,
        /// Mostra os nomes das teclas
        #[arg(long)]
        list: bool,
    },
    /// Sincroniza o relógio da tela com este PC
    Time,
    /// Mostra modelo e versão do firmware (dongle)
    Info,
    /// Acompanha as cores do HyperHDR (cabo: tempo real; dongle: cor do teclado inteiro, com limite)
    Hyperhdr {
        /// Endereço do HyperHDR (ex.: 127.0.0.1 no próprio Bazzite)
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Porta do servidor JSON do HyperHDR
        #[arg(long, default_value_t = 19444)]
        port: u16,
        /// Pelo dongle: segundos mínimos entre trocas de cor (poupa a memória do teclado)
        #[arg(long, default_value_t = 2.0)]
        min_gap: f64,
        /// Diferença mínima de cor (0–255) para trocar
        #[arg(long, default_value_t = 24)]
        threshold: u8,
        #[arg(short, long, default_value_t = 5, value_parser = clap::value_parser!(u8).range(0..=5))]
        brightness: u8,
    },
    /// Troca a função de uma tecla (dongle), ex.: remap f5 media:volup | remap f5 default | remap f6 ctrl+c
    Remap {
        /// Tecla física (veja `keys --list`)
        key: String,
        /// default, nome de tecla com ctrl+/shift+/alt+/win+, key:0xNN, ou media:<volup|voldown|mute|play|next|prev|stop|calc|mail|browser|mycomputer>
        action: String,
    },
    /// Reinicia o dongle 2.4G quando ele trava (Linux): age depois de silêncio que segue atividade
    Watchdog {
        /// Só registra o que faria, sem reiniciar
        #[arg(long)]
        dry_run: bool,
        /// Segundos de silêncio, depois de uma tecla, até reiniciar
        #[arg(long, default_value_t = 30)]
        silence: u64,
        /// Segundos de silêncio em que reinicia a cada --silence; depois o intervalo dobra
        #[arg(long, default_value_t = 900)]
        fast_window: u64,
        /// Primeiro intervalo depois da janela, em segundos
        #[arg(long, default_value_t = 60)]
        backoff_start: u64,
        /// Intervalo máximo, em segundos
        #[arg(long, default_value_t = 1800)]
        backoff_max: u64,
        /// Segundos sem atividade até desarmar (volta na próxima tecla)
        #[arg(long, default_value_t = 7200)]
        give_up: u64,
    },
    /// Envia uma imagem, GIF ou vídeo curto para a tela de 128×128 (cabo)
    Screen {
        /// PNG, JPG, GIF ou vídeo (mp4, webm, mkv, mov, avi; vídeo usa o ffmpeg)
        image: std::path::PathBuf,
        /// Recorta para preencher (padrão) ou encaixa com bordas
        #[arg(long)]
        pad: bool,
        /// Vídeo: quadros por segundo (1–16; a tela guarda até 140 quadros)
        #[arg(long, default_value_t = 10.0)]
        fps: f64,
        /// Vídeo: início do trecho, em segundos
        #[arg(long, default_value_t = 0.0)]
        start: f64,
        /// Vídeo: duração do trecho, em segundos
        #[arg(long)]
        duration: Option<f64>,
        /// Também salva o resultado como GIF neste arquivo
        #[arg(long)]
        save_gif: Option<std::path::PathBuf>,
        /// Só converte (com --save-gif), sem enviar ao teclado
        #[arg(long)]
        no_upload: bool,
    },
}

fn rgb(s: &str) -> Result<[u8; 3]> {
    proto::parse_hex_rgb(s).with_context(|| format!("cor inválida {s:?}: use RRGGBB"))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.cmd {
        Cmd::Modes => {
            for (id, name, _) in proto::MODES {
                println!("{id:>2}  {name}");
            }
            return Ok(());
        }
        Cmd::Keys { list: true, .. } => {
            println!("{}", proto::KEYS.iter().map(|k| k.0).collect::<Vec<_>>().join(" "));
            return Ok(());
        }
        &Cmd::Watchdog { dry_run, silence, fast_window, backoff_start, backoff_max, give_up } => {
            let s = std::time::Duration::from_secs;
            let policy = watchdog::Policy {
                silence: s(silence.max(1)),
                fast_window: s(fast_window),
                backoff_start: s(backoff_start.max(1)),
                backoff_max: s(backoff_max.max(backoff_start)),
                give_up: s(give_up),
            };
            return watchdog::run(policy, dry_run);
        }
        _ => {}
    }

    let api = HidApi::new().context("iniciar o HID")?;
    if let Cmd::List = cli.cmd {
        let found = device::list(&api);
        if found.is_empty() {
            println!("nada encontrado");
        }
        for l in found {
            println!("{l}");
        }
        return Ok(());
    }

    if let Cmd::Screen { image, pad, fps, start, duration, save_gif, no_upload } = &cli.cmd {
        let frames = if screen::is_video(image) {
            let opt = screen::VideoOptions { start: *start, duration: *duration, fps: *fps, pad: *pad };
            screen::load_video(image, &opt)?
        } else {
            screen::load(image, *pad)?
        };
        eprintln!("{} quadro(s) de 128×128", frames.len());
        if let Some(out) = save_gif {
            screen::save_gif(&frames, out)?;
            println!("GIF salvo em {}", out.display());
        }
        if *no_upload {
            return Ok(());
        }
        let kb = Keyboard::open(&api, cli.via)?;
        eprintln!("usando {}", kb.desc);
        kb.upload_screen(&screen::blob(&frames), |i, n| eprint!("\r{}%", 100 * i / n))?;
        eprintln!();
        println!("enviado para a tela");
        return Ok(());
    }

    let kb = Keyboard::open(&api, cli.via)?;
    eprintln!("usando {}", kb.desc);
    match cli.cmd {
        Cmd::Ping => println!("{}", kb.ping()?),
        Cmd::Mode { mode, color, brightness, speed, direction } => {
            let Some(id) = proto::mode_by_name(&mode) else { bail!("efeito desconhecido {mode:?} (veja `modes`)") };
            let rgb_v = color.as_deref().map(rgb).transpose()?;
            kb.set_light(&Light {
                mode: id,
                rgb: rgb_v.unwrap_or([255, 0, 0]),
                rainbow: rgb_v.is_none(),
                brightness,
                speed,
                direction,
            })?;
            println!("efeito aplicado");
        }
        Cmd::Keys { assign, all, brightness, .. } => {
            let base = all.as_deref().map(rgb).transpose()?.unwrap_or([0, 0, 0]);
            let mut colors: Vec<(u8, [u8; 3])> = proto::KEYS.iter().map(|k| (k.1, base)).collect();
            for kv in &assign {
                let (name, col) = kv.split_once('=').with_context(|| format!("use tecla=RRGGBB, não {kv:?}"))?;
                let idx = proto::key_index(name).with_context(|| format!("tecla desconhecida {name:?} (veja --list)"))?;
                let c = rgb(col)?;
                colors.iter_mut().filter(|e| e.0 == idx).for_each(|e| e.1 = c);
            }
            kb.set_keys(&colors, brightness)?;
            println!("cores aplicadas");
        }
        Cmd::Time => {
            kb.sync_time()?;
            println!("relógio sincronizado");
        }
        Cmd::Info => {
            let i = kb.info()?;
            println!(
                "teclado {:04x}:{:04x}, firmware {}.{:02x}",
                i.vid, i.pid, i.version >> 8, i.version & 0xFF
            );
        }
        Cmd::Hyperhdr { host, port, min_gap, threshold, brightness } => {
            let opt = ambient::Options {
                host,
                port,
                interval_ms: if kb.is_wired() { 66 } else { 250 },
                min_gap: std::time::Duration::from_secs_f64(min_gap.max(0.5)),
                threshold,
                brightness,
            };
            eprintln!("acompanhando o HyperHDR em {}:{} (Ctrl+C para parar)", opt.host, opt.port);
            let never = std::sync::atomic::AtomicBool::new(false);
            let hooks = ak820ctl::hooks::Hooks::load();
            loop {
                let res = ambient::run(&kb, &opt, &never, |[r, g, b], wired| {
                    if !wired {
                        println!("cor #{r:02x}{g:02x}{b:02x}");
                    }
                });
                if let Err(e) = res {
                    hooks.fire(ak820ctl::hooks::Event::Error, &format!("sincronização: {e:#}"));
                    eprintln!("{e:#}; tentando de novo em 5 s");
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
        Cmd::Remap { key, action } => {
            let k = proto::key(&key).with_context(|| format!("tecla desconhecida {key:?} (veja keys --list)"))?;
            if k.0 == "fn" {
                bail!("a tecla Fn não pode ser trocada (você perderia os atalhos do teclado)");
            }
            let a = proto::parse_action(&action, k.2).with_context(|| format!("função inválida {action:?}"))?;
            kb.remap(k.1, &a)?;
            println!("{} → {:?}", k.0, a);
        }
        Cmd::List | Cmd::Modes | Cmd::Screen { .. } | Cmd::Watchdog { .. } => unreachable!(),
    }
    Ok(())
}
