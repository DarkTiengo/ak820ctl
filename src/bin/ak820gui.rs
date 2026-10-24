//! Interface gráfica do ak820ctl (Windows e Linux).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use ak820ctl::ambient;
use ak820ctl::device::{Keyboard, Via};
use ak820ctl::hooks::{Event, Hooks};
use ak820ctl::proto::{self, Action, Light};
use ak820ctl::screen;
use eframe::egui;
use hidapi::HidApi;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---- sincronização com o HyperHDR (thread própria, com a sua própria conexão ao teclado) ----

struct Sync {
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<String>>,
    color: Arc<Mutex<Option<[u8; 3]>>>,
}

fn start_sync(via: Via, opt: ambient::Options, ctx: egui::Context) -> Sync {
    let stop = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new("conectando…".to_string()));
    let color = Arc::new(Mutex::new(None));
    let (st, stat, col) = (stop.clone(), status.clone(), color.clone());
    std::thread::spawn(move || {
        let hooks = Hooks::load();
        let set = |s: String| {
            *stat.lock().unwrap() = s;
            ctx.request_repaint();
        };
        while !st.load(Ordering::Relaxed) {
            let res = HidApi::new()
                .map_err(anyhow::Error::from)
                .and_then(|api| Keyboard::open(&api, via))
                .and_then(|kb| {
                    set(format!("acompanhando {}:{} ({})", opt.host, opt.port, kb.desc));
                    ambient::run(&kb, &opt, &st, |c, _| {
                        *col.lock().unwrap() = Some(c);
                        ctx.request_repaint();
                    })
                });
            match res {
                Ok(()) => break,
                Err(e) => {
                    hooks.fire(Event::Error, &format!("sincronização: {e:#}"));
                    set(format!("{e:#}; tentando de novo em 5 s"));
                }
            }
            for _ in 0..50 {
                if st.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        set("parado".into());
    });
    Sync { stop, status, color }
}

// ---- trabalho com o teclado (thread separada) ---------------------------------------------

enum Job {
    Connect(Via),
    Light(Light),
    Time,
    Keys(Vec<(u8, [u8; 3])>, u8),
    Remap(u8, Action),
    Screen(Vec<u8>),
}

enum Reply {
    Connected { desc: String, wired: bool },
    Disconnected(String),
    Done(String),
    Error(String),
    Progress(usize, usize),
}

fn worker(jobs: Receiver<Job>, out: Sender<Reply>, ctx: egui::Context) {
    let mut api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            let _ = out.send(Reply::Disconnected(format!("HID indisponível: {e}")));
            return;
        }
    };
    let mut kb: Option<Keyboard> = None;
    let mut desc = String::new();
    let mut via = Via::Auto;
    let hooks = Hooks::load();
    let mut was_connected = false;
    let send = |r: Reply| {
        let _ = out.send(r);
        ctx.request_repaint();
    };
    for job in jobs {
        if let Job::Connect(v) = job {
            via = v;
            kb = None;
        }
        // conecta, ou reconecta sozinho antes de uma ação se a conexão caiu
        if kb.is_none() {
            let _ = api.refresh_devices();
            match Keyboard::open(&api, via) {
                Ok(k) => {
                    desc = k.desc.clone();
                    if let Ok(i) = k.info() {
                        desc += &format!(" · firmware {}.{:02x}", i.version >> 8, i.version & 0xFF);
                    }
                    send(Reply::Connected { wired: k.is_wired(), desc: desc.clone() });
                    hooks.fire(Event::Connect, &desc);
                    was_connected = true;
                    kb = Some(k);
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    if std::mem::take(&mut was_connected) {
                        hooks.fire(Event::Disconnect, &msg);
                    }
                    send(Reply::Disconnected(msg));
                    continue;
                }
            }
        }
        let k = kb.as_ref().unwrap();
        let res = match job {
            Job::Connect(_) => {
                send(Reply::Done(format!("conectado: {desc}")));
                continue;
            }
            Job::Light(l) => k.set_light(&l).map(|_| "efeito aplicado".to_string()),
            Job::Time => k.sync_time().map(|_| "relógio sincronizado".to_string()),
            Job::Keys(c, b) => k.set_keys(&c, b).map(|_| "cores por tecla aplicadas".to_string()),
            Job::Remap(i, a) => k.remap(i, &a).map(|_| "função da tecla trocada".to_string()),
            Job::Screen(blob) => k
                .upload_screen(&blob, |i, n| send(Reply::Progress(i, n)))
                .map(|_| "imagem enviada para a tela".to_string()),
        };
        match res {
            Ok(m) => send(Reply::Done(m)),
            Err(e) => {
                let msg = format!("{e:#}");
                hooks.fire(Event::Error, &msg);
                // falhas de rádio não derrubam a conexão; as de HID sim
                if !msg.contains("sem resposta") {
                    kb = None;
                    hooks.fire(Event::Disconnect, &msg);
                    was_connected = false;
                }
                send(Reply::Error(msg));
            }
        }
    }
}

// ---- interface ------------------------------------------------------------------------------

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Light,
    Keys,
    Screen,
    Sync,
}

#[derive(PartialEq, Clone, Copy)]
enum ActionKind {
    Default,
    Media,
    Key,
}

const ROW_ENDS: [&str; 6] = ["delete", "home", "pgup", "pgdn", "up", "right"];

struct App {
    tx: Sender<Job>,
    rx: Receiver<Reply>,
    via: Via,
    status: String,
    connected: Option<(String, bool)>,
    busy: bool,
    progress: Option<(usize, usize)>,
    tab: Tab,
    // luz
    light: Light,
    use_color: bool,
    // teclas
    key_colors: Vec<[u8; 3]>,
    key_brightness: u8,
    paint: [u8; 3],
    selected: usize,
    action_kind: ActionKind,
    media: usize,
    target_key: usize,
    mods: [bool; 4],
    // tela
    image_path: Option<PathBuf>,
    frames: Vec<(image::RgbImage, u32)>,
    preview: Vec<egui::TextureHandle>,
    pad: bool,
    video_fps: f64,
    video_start: f64,
    video_duration: f64,
    // sincronizar
    hyper_host: String,
    hyper_port: u16,
    min_gap: f64,
    sync: Option<Sync>,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let (tx, jobs) = channel();
        let (out, rx) = channel();
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || worker(jobs, out, ctx));
        let _ = tx.send(Job::Connect(Via::Auto));
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        App {
            tx,
            rx,
            via: Via::Auto,
            status: "procurando o teclado…".into(),
            connected: None,
            busy: true,
            progress: None,
            tab: Tab::Light,
            light: Light { mode: 1, rgb: [0, 160, 255], rainbow: false, brightness: 5, speed: 3, direction: 0 },
            use_color: true,
            key_colors: vec![[0, 0, 48]; proto::KEYS.len()],
            key_brightness: 5,
            paint: [255, 0, 0],
            selected: proto::KEYS.iter().position(|k| k.0 == "f5").unwrap(),
            action_kind: ActionKind::Media,
            media: 0,
            target_key: 0,
            mods: [false; 4],
            image_path: None,
            frames: Vec::new(),
            preview: Vec::new(),
            pad: false,
            video_fps: 10.0,
            video_start: 0.0,
            video_duration: 0.0,
            hyper_host: "127.0.0.1".into(),
            hyper_port: 19444,
            min_gap: 2.0,
            sync: None,
        }
    }

    fn run(&mut self, job: Job) {
        self.busy = true;
        self.progress = None;
        self.status = "enviando…".into();
        let _ = self.tx.send(job);
    }

    fn wired(&self) -> bool {
        self.connected.as_ref().is_some_and(|c| c.1)
    }

    fn poll(&mut self) {
        while let Ok(r) = self.rx.try_recv() {
            match r {
                Reply::Connected { desc, wired } => self.connected = Some((desc, wired)),
                Reply::Disconnected(e) => {
                    self.connected = None;
                    self.busy = false;
                    self.status = e;
                }
                Reply::Done(m) => {
                    self.busy = false;
                    self.progress = None;
                    self.status = m;
                }
                Reply::Error(e) => {
                    self.busy = false;
                    self.progress = None;
                    self.status = format!("erro: {e}");
                }
                Reply::Progress(i, n) => self.progress = Some((i, n)),
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("AK820 Pro");
            ui.separator();
            ui.label("Conexão:");
            let before = self.via;
            egui::ComboBox::from_id_salt("via")
                .selected_text(match self.via {
                    Via::Auto => "Automática",
                    Via::Wired => "Cabo USB",
                    Via::Dongle => "Dongle 2.4G",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.via, Via::Auto, "Automática");
                    ui.selectable_value(&mut self.via, Via::Wired, "Cabo USB");
                    ui.selectable_value(&mut self.via, Via::Dongle, "Dongle 2.4G");
                });
            if ui.button("Reconectar").clicked() || before != self.via {
                self.connected = None;
                self.status = "procurando o teclado…".into();
                self.run(Job::Connect(self.via));
            }
        });
        ui.horizontal(|ui| {
            let color = match &self.connected {
                Some(_) => egui::Color32::from_rgb(60, 200, 90),
                None => egui::Color32::from_rgb(220, 70, 60),
            };
            let (dot, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter().circle_filled(dot.center(), 5.0, color);
            if self.busy {
                ui.spinner();
            }
            ui.label(&self.status);
            if let Some((i, n)) = self.progress {
                ui.add(egui::ProgressBar::new(i as f32 / n as f32).desired_width(140.0).show_percentage());
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Light, "Iluminação");
            ui.selectable_value(&mut self.tab, Tab::Keys, "Teclas");
            ui.selectable_value(&mut self.tab, Tab::Screen, "Tela");
            ui.selectable_value(&mut self.tab, Tab::Sync, "Sincronizar");
        });
    }

    fn light_tab(&mut self, ui: &mut egui::Ui) {
        let flags = proto::MODES.iter().find(|m| m.0 == self.light.mode).map_or(0, |m| m.2);
        egui::Grid::new("light").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Efeito");
            let name = proto::MODES.iter().find(|m| m.0 == self.light.mode).map_or("?", |m| m.1);
            egui::ComboBox::from_id_salt("mode").selected_text(name).show_ui(ui, |ui| {
                for (id, n, _) in proto::MODES {
                    ui.selectable_value(&mut self.light.mode, *id, *n);
                }
            });
            ui.end_row();
            if flags & 0x10 != 0 {
                ui.label("Cor");
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(flags & 0x20 == 0 || self.use_color, |ui| {
                        egui::color_picker::color_edit_button_srgb(ui, &mut self.light.rgb);
                    });
                    if flags & 0x20 != 0 {
                        ui.radio_value(&mut self.use_color, true, "cor única");
                        ui.radio_value(&mut self.use_color, false, "arco-íris");
                    }
                });
                ui.end_row();
            }
            if flags & 0x01 != 0 {
                ui.label("Brilho");
                ui.add(egui::Slider::new(&mut self.light.brightness, 0..=5));
                ui.end_row();
            }
            if flags & 0x02 != 0 {
                ui.label("Velocidade");
                ui.add(egui::Slider::new(&mut self.light.speed, 0..=5));
                ui.end_row();
            }
            if flags & 0x0C != 0 {
                ui.label("Direção");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.light.direction, 0, "esq. p/ dir. / p/ baixo");
                    ui.radio_value(&mut self.light.direction, 1, "ao contrário");
                });
                ui.end_row();
            }
        });
        ui.add_space(12.0);
        let ok = self.connected.is_some() && !self.busy;
        if ui.add_enabled(ok, egui::Button::new("Aplicar efeito").min_size([160.0, 32.0].into())).clicked() {
            let mut l = self.light;
            l.rainbow = flags & 0x20 != 0 && (!self.use_color || flags & 0x10 == 0);
            self.run(Job::Light(l));
        }
    }

    fn keyboard(&mut self, ui: &mut egui::Ui, paint_mode: bool) {
        let unit = 34.0;
        let mut row: Vec<usize> = Vec::new();
        let mut rows: Vec<Vec<usize>> = Vec::new();
        for (i, k) in proto::KEYS.iter().enumerate() {
            row.push(i);
            if ROW_ENDS.contains(&k.0) {
                rows.push(std::mem::take(&mut row));
            }
        }
        for r in rows {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for i in r {
                    let (name, _, _) = proto::KEYS[i];
                    let w = match name {
                        "space" => unit * 6.2,
                        "backspace" | "lshift" | "enter" | "capslock" | "tab" => unit * 1.8,
                        "rshift" | "backslash" => unit * 1.4,
                        _ => unit,
                    };
                    let [r, g, b] = self.key_colors[i];
                    let fill = if paint_mode { egui::Color32::from_rgb(r, g, b) } else { ui.visuals().widgets.inactive.bg_fill };
                    let lum = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
                    let text_col = if paint_mode && lum > 140.0 { egui::Color32::BLACK } else { ui.visuals().strong_text_color() };
                    let label = egui::RichText::new(short(name)).size(11.0).color(text_col);
                    let mut btn = egui::Button::new(label).fill(fill).min_size([w, unit].into());
                    if i == self.selected {
                        btn = btn.stroke(egui::Stroke::new(2.0, ui.visuals().selection.stroke.color));
                    }
                    if ui.add(btn).clicked() {
                        self.selected = i;
                        if paint_mode {
                            self.key_colors[i] = self.paint;
                        }
                    }
                }
            });
        }
    }

    fn keys_tab(&mut self, ui: &mut egui::Ui) {
        let ok = self.connected.is_some() && !self.busy;
        let wired_now = self.wired();
        egui::CollapsingHeader::new("Cor por tecla (pelo cabo)").default_open(wired_now).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Pincel:");
                egui::color_picker::color_edit_button_srgb(ui, &mut self.paint);
                if ui.button("Pintar todas").clicked() {
                    self.key_colors.iter_mut().for_each(|c| *c = self.paint);
                }
                ui.add(egui::Slider::new(&mut self.key_brightness, 0..=5).text("brilho"));
            });
            ui.label("Clique nas teclas para pintar.");
            self.keyboard(ui, true);
            let wired = self.wired();
            ui.horizontal(|ui| {
                if ui.add_enabled(ok && wired, egui::Button::new("Aplicar cores")).clicked() {
                    let colors = proto::KEYS.iter().zip(&self.key_colors).map(|(k, c)| (k.1, *c)).collect();
                    self.run(Job::Keys(colors, self.key_brightness));
                }
                if !wired {
                    ui.weak("só pelo cabo (o firmware não aceita pelo 2.4G)");
                }
            });
        });
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Trocar função de tecla (pelo dongle)").default_open(!wired_now).show(ui, |ui| {
            ui.label("Escolha a tecla física no teclado abaixo:");
            self.keyboard(ui, false);
            let (name, idx, default_usage) = proto::KEYS[self.selected];
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.strong(format!("Tecla: {}", short(name)));
                ui.separator();
                ui.radio_value(&mut self.action_kind, ActionKind::Default, "padrão");
                ui.radio_value(&mut self.action_kind, ActionKind::Media, "mídia");
                ui.radio_value(&mut self.action_kind, ActionKind::Key, "outra tecla / atalho");
            });
            ui.horizontal(|ui| match self.action_kind {
                ActionKind::Default => {
                    ui.weak("volta à função original");
                }
                ActionKind::Media => {
                    egui::ComboBox::from_id_salt("media").selected_text(media_label(self.media)).show_ui(ui, |ui| {
                        for i in 0..proto::MEDIA.len() {
                            ui.selectable_value(&mut self.media, i, media_label(i));
                        }
                    });
                }
                ActionKind::Key => {
                    for (m, n) in self.mods.iter_mut().zip(["Ctrl", "Shift", "Alt", "Win"]) {
                        ui.checkbox(m, n);
                    }
                    ui.label("+");
                    egui::ComboBox::from_id_salt("target")
                        .selected_text(short(proto::KEYS[self.target_key].0))
                        .show_ui(ui, |ui| {
                            for (i, k) in proto::KEYS.iter().enumerate().filter(|(_, k)| k.2 != 0) {
                                ui.selectable_value(&mut self.target_key, i, short(k.0));
                            }
                        });
                }
            });
            let dongle = self.connected.as_ref().is_some_and(|c| !c.1);
            ui.horizontal(|ui| {
                let allowed = name != "fn";
                if ui.add_enabled(ok && dongle && allowed, egui::Button::new("Aplicar função")).clicked() {
                    let action = match self.action_kind {
                        ActionKind::Default => Action::Key { mods: 0, usage: default_usage },
                        ActionKind::Media => Action::Media(proto::MEDIA[self.media].1),
                        ActionKind::Key => {
                            let mods = self.mods.iter().enumerate().fold(0u8, |m, (i, on)| if *on { m | 1 << i } else { m });
                            Action::Key { mods, usage: proto::KEYS[self.target_key].2 }
                        }
                    };
                    self.run(Job::Remap(idx, action));
                }
                if !allowed {
                    ui.weak("a tecla Fn não pode ser trocada");
                } else if !dongle {
                    ui.weak("só pelo dongle por enquanto (a troca vale também no cabo)");
                }
            });
        });
    }

    fn screen_tab(&mut self, ui: &mut egui::Ui) {
        let ok = self.connected.is_some() && !self.busy;
        ui.horizontal(|ui| {
            if ui.button("Escolher imagem, GIF ou vídeo…").clicked() {
                let mut exts = vec!["png", "jpg", "jpeg", "gif"];
                exts.extend_from_slice(screen::VIDEO_EXTS);
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Imagens e vídeos", &exts)
                    .add_filter("Vídeos", screen::VIDEO_EXTS)
                    .pick_file()
                {
                    self.image_path = Some(p);
                    self.reload_image(ui.ctx());
                }
            }
            if ui.checkbox(&mut self.pad, "encaixar com bordas (sem recortar)").changed() && self.image_path.is_some() {
                self.reload_image(ui.ctx());
            }
        });
        if self.image_path.as_deref().is_some_and(screen::is_video) {
            ui.horizontal(|ui| {
                ui.label("Vídeo:");
                ui.add(egui::Slider::new(&mut self.video_fps, 1.0..=16.0).text("quadros/s").fixed_decimals(0));
                ui.add(egui::DragValue::new(&mut self.video_start).range(0.0..=36000.0).speed(0.1).prefix("início ").suffix(" s"));
                ui.add(egui::DragValue::new(&mut self.video_duration).range(0.0..=600.0).speed(0.1).prefix("duração ").suffix(" s"));
                if ui.button("Atualizar").clicked() {
                    self.reload_image(ui.ctx());
                }
            });
            ui.weak(format!(
                "A tela guarda até 140 quadros: a {:.0} quadros/s cabem {:.1} s de vídeo. Duração 0 = o máximo que couber.",
                self.video_fps,
                proto::SCREEN_MAX_FRAMES as f64 / self.video_fps
            ));
        }
        if let Some(p) = &self.image_path {
            ui.weak(format!(
                "{} · {} quadro(s){}",
                p.file_name().map(|f| f.to_string_lossy()).unwrap_or_default(),
                self.frames.len(),
                if self.frames.len() >= proto::SCREEN_MAX_FRAMES { " (limite de 140)" } else { "" }
            ));
        }
        ui.add_space(6.0);
        if !self.preview.is_empty() {
            // prévia animada, no mesmo ritmo que a tela vai mostrar
            let total: u32 = self.frames.iter().map(|f| f.1.max(1)).sum();
            let mut t = (ui.input(|i| i.time) * 1000.0) as u32 % total.max(1);
            let mut idx = 0;
            for (i, (_, ms)) in self.frames.iter().enumerate() {
                if t < (*ms).max(1) {
                    idx = i;
                    break;
                }
                t -= (*ms).max(1);
            }
            let tex = &self.preview[idx.min(self.preview.len() - 1)];
            egui::Frame::default().stroke(ui.visuals().widgets.noninteractive.bg_stroke).inner_margin(4.0).show(ui, |ui| {
                ui.image((tex.id(), egui::vec2(256.0, 256.0)));
            });
            if self.preview.len() > 1 {
                ui.ctx().request_repaint_after(Duration::from_millis(30));
            }
        }
        ui.add_space(8.0);
        let wired = self.wired();
        ui.horizontal(|ui| {
            if ui.add_enabled(ok && wired && !self.frames.is_empty(), egui::Button::new("Enviar para a tela")).clicked() {
                self.run(Job::Screen(screen::blob(&self.frames)));
            }
            if ui.add_enabled(!self.frames.is_empty(), egui::Button::new("Salvar como GIF…")).clicked() {
                if let Some(out) = rfd::FileDialog::new().add_filter("GIF", &["gif"]).set_file_name("ak820.gif").save_file() {
                    self.status = match screen::save_gif(&self.frames, &out) {
                        Ok(()) => format!("GIF salvo em {}", out.display()),
                        Err(e) => format!("erro ao salvar o GIF: {e:#}"),
                    };
                }
            }
            if !wired {
                ui.weak("imagem só pelo cabo (o firmware não aceita pelo 2.4G)");
            }
        });
        ui.add_space(12.0);
        ui.separator();
        if ui.add_enabled(ok, egui::Button::new("Sincronizar relógio com o PC")).clicked() {
            self.run(Job::Time);
        }
    }

    fn sync_tab(&mut self, ui: &mut egui::Ui) {
        ui.strong("HyperHDR");
        ui.label("O teclado acompanha a cor média que o HyperHDR manda para as suas fitas de LED.");
        ui.weak("No HyperHDR, o servidor JSON (porta 19444) precisa estar ligado.");
        ui.add_space(8.0);
        let running = self.sync.is_some();
        ui.add_enabled_ui(!running, |ui| {
            egui::Grid::new("sync").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                ui.label("Endereço");
                ui.text_edit_singleline(&mut self.hyper_host);
                ui.end_row();
                ui.label("Porta");
                ui.add(egui::DragValue::new(&mut self.hyper_port).range(1..=65535));
                ui.end_row();
                ui.label("Intervalo no dongle");
                ui.add(egui::Slider::new(&mut self.min_gap, 0.5..=10.0).suffix(" s"));
                ui.end_row();
            });
        });
        ui.weak("Pelo cabo a cor muda em tempo real. Pelo dongle a cor só troca quando muda de verdade e no máximo no intervalo acima, para poupar a memória do teclado.");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if !running {
                if ui.add(egui::Button::new("Iniciar sincronização").min_size([180.0, 30.0].into())).clicked() {
                    let opt = ambient::Options {
                        host: self.hyper_host.trim().to_string(),
                        port: self.hyper_port,
                        interval_ms: 250,
                        min_gap: Duration::from_secs_f64(self.min_gap),
                        threshold: 24,
                        brightness: self.light.brightness,
                    };
                    self.sync = Some(start_sync(self.via, opt, ui.ctx().clone()));
                }
            } else if ui.add(egui::Button::new("Parar").min_size([120.0, 30.0].into())).clicked() {
                if let Some(s) = self.sync.take() {
                    s.stop.store(true, Ordering::Relaxed);
                }
            }
        });
        if let Some(s) = &self.sync {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if let Some([r, g, b]) = *s.color.lock().unwrap() {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 18.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgb(r, g, b));
                    ui.monospace(format!("#{r:02x}{g:02x}{b:02x}"));
                    ui.separator();
                }
                ui.label(s.status.lock().unwrap().as_str());
            });
        }
    }

    fn reload_image(&mut self, ctx: &egui::Context) {
        let Some(p) = self.image_path.clone() else { return };
        let loaded = if screen::is_video(&p) {
            let opt = screen::VideoOptions {
                start: self.video_start,
                duration: (self.video_duration > 0.0).then_some(self.video_duration),
                fps: self.video_fps,
                pad: self.pad,
            };
            screen::load_video(&p, &opt)
        } else {
            screen::load(&p, self.pad)
        };
        match loaded {
            Ok(frames) => {
                self.preview = frames
                    .iter()
                    .enumerate()
                    .map(|(i, (f, _))| {
                        let img = egui::ColorImage::from_rgb([f.width() as usize, f.height() as usize], f.as_raw());
                        ctx.load_texture(format!("preview{i}"), img, egui::TextureOptions::NEAREST)
                    })
                    .collect();
                self.frames = frames;
            }
            Err(e) => {
                self.frames.clear();
                self.preview.clear();
                self.status = format!("erro ao abrir o arquivo: {e:#}");
            }
        }
    }
}

fn short(name: &str) -> String {
    match name {
        "grave" => "`".into(),
        "minus" => "-".into(),
        "equal" => "=".into(),
        "lbracket" => "[".into(),
        "rbracket" => "]".into(),
        "backslash" => "\\".into(),
        "semicolon" => ";".into(),
        "quote" => "'".into(),
        "comma" => ",".into(),
        "period" => ".".into(),
        "slash" => "/".into(),
        "backspace" => "Bksp".into(),
        "enter" => "Enter".into(),
        "lshift" | "rshift" => "Shift".into(),
        "lctrl" | "rctrl" => "Ctrl".into(),
        "lalt" | "ralt" => "Alt".into(),
        "capslock" => "Caps".into(),
        "delete" => "Del".into(),
        "pgup" => "PgUp".into(),
        "pgdn" => "PgDn".into(),
        "up" => "Cima".into(),
        "down" => "Baixo".into(),
        "left" => "Esq".into(),
        "right" => "Dir".into(),
        "space" => "Espaço".into(),
        "win" => "Win".into(),
        "fn" => "Fn".into(),
        "home" => "Home".into(),
        "tab" => "Tab".into(),
        "esc" => "Esc".into(),
        n => n.to_uppercase(),
    }
}

fn media_label(i: usize) -> &'static str {
    match proto::MEDIA[i].0 {
        "volup" => "Volume +",
        "voldown" => "Volume -",
        "mute" => "Mudo",
        "play" => "Play / Pausa",
        "next" => "Próxima faixa",
        "prev" => "Faixa anterior",
        "stop" => "Parar",
        "calc" => "Calculadora",
        "mail" => "E-mail",
        "browser" => "Navegador",
        "mycomputer" => "Meu computador",
        other => other,
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                Tab::Light => self.light_tab(ui),
                Tab::Keys => self.keys_tab(ui),
                Tab::Screen => self.screen_tab(ui),
                Tab::Sync => self.sync_tab(ui),
            });
        });
    }
}

fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AK820 Pro")
            .with_inner_size([720.0, 520.0])
            .with_min_inner_size([560.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("AK820 Pro", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
