//! Imagens e GIFs → quadros RGB565 de 128×128 para a tela.

use crate::proto;
use anyhow::Result;
use image::{imageops::FilterType, AnimationDecoder, DynamicImage, RgbImage};
use std::path::Path;

/// Redimensiona para 128×128: recorta para preencher, ou encaixa com bordas pretas (`pad`).
pub fn fit(img: DynamicImage, pad: bool) -> RgbImage {
    if pad {
        let s = img.resize(proto::W, proto::H, FilterType::Lanczos3).to_rgb8();
        let mut canvas = RgbImage::new(proto::W, proto::H);
        let (x, y) = ((proto::W - s.width()) / 2, (proto::H - s.height()) / 2);
        image::imageops::overlay(&mut canvas, &s, x as i64, y as i64);
        canvas
    } else {
        img.resize_to_fill(proto::W, proto::H, FilterType::Lanczos3).to_rgb8()
    }
}

/// Quadros já em 128×128 RGB com o atraso de cada um em ms.
pub fn load(path: &Path, pad: bool) -> Result<Vec<(RgbImage, u32)>> {
    let is_gif = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gif"));
    if is_gif {
        let file = std::io::BufReader::new(std::fs::File::open(path)?);
        let frames = image::codecs::gif::GifDecoder::new(file)?.into_frames().collect_frames()?;
        return Ok(frames
            .into_iter()
            .take(proto::SCREEN_MAX_FRAMES)
            .map(|f| {
                let (n, d) = f.delay().numer_denom_ms();
                let ms = if d == 0 || n == 0 { 100 } else { n / d };
                (fit(DynamicImage::ImageRgba8(f.into_buffer()), pad), ms)
            })
            .collect());
    }
    Ok(vec![(fit(image::open(path)?, pad), 100)])
}

pub const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "mkv", "webm", "mov", "avi"];

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTS.iter().any(|v| v.eq_ignore_ascii_case(e)))
}

pub struct VideoOptions {
    /// Início do trecho, em segundos.
    pub start: f64,
    /// Duração do trecho; `None` = até completar 140 quadros.
    pub duration: Option<f64>,
    /// Quadros por segundo (a tela não passa de ~16).
    pub fps: f64,
    pub pad: bool,
}

impl Default for VideoOptions {
    fn default() -> Self {
        VideoOptions { start: 0.0, duration: None, fps: 10.0, pad: false }
    }
}

/// ffmpeg do sistema, ou o indicado em `AK820_FFMPEG`.
fn ffmpeg() -> std::ffi::OsString {
    std::env::var_os("AK820_FFMPEG").unwrap_or_else(|| "ffmpeg".into())
}

/// Filtro do ffmpeg que deixa o vídeo em 128×128, recortando ou com bordas.
pub fn video_filter(fps: f64, pad: bool) -> String {
    let (w, h) = (proto::W, proto::H);
    let fit = if pad {
        format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2")
    } else {
        format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}")
    };
    format!("fps={fps},{fit}")
}

/// Vídeo curto → quadros 128×128 (no máximo 140), usando o ffmpeg.
pub fn load_video(path: &Path, opt: &VideoOptions) -> Result<Vec<(RgbImage, u32)>> {
    let fps = opt.fps.clamp(1.0, 16.0);
    let mut cmd = std::process::Command::new(ffmpeg());
    cmd.args(["-v", "error", "-ss", &opt.start.max(0.0).to_string(), "-i"]).arg(path);
    if let Some(d) = opt.duration {
        cmd.args(["-t", &d.max(0.1).to_string()]);
    }
    cmd.args(["-vf", &video_filter(fps, opt.pad)])
        .args(["-frames:v", &proto::SCREEN_MAX_FRAMES.to_string()])
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
        .stdin(std::process::Stdio::null());
    let out = cmd.output().map_err(|e| {
        anyhow::anyhow!("não consegui rodar o ffmpeg ({e}). Instale o ffmpeg (Windows: winget install Gyan.FFmpeg) ou indique o caminho em AK820_FFMPEG")
    })?;
    if !out.status.success() {
        anyhow::bail!("o ffmpeg falhou: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let size = (proto::W * proto::H * 3) as usize;
    let delay = (1000.0 / fps).round() as u32;
    let frames: Vec<_> = out
        .stdout
        .chunks_exact(size)
        .map(|c| (RgbImage::from_raw(proto::W, proto::H, c.to_vec()).expect("tamanho do quadro"), delay))
        .collect();
    if frames.is_empty() {
        anyhow::bail!("o vídeo não gerou nenhum quadro (início depois do fim?)");
    }
    Ok(frames)
}

/// Salva os quadros como GIF animado (o mesmo que vai para a tela).
pub fn save_gif(frames: &[(RgbImage, u32)], path: &Path) -> Result<()> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame};
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = GifEncoder::new_with_speed(file, 10);
    enc.set_repeat(Repeat::Infinite)?;
    for (img, ms) in frames {
        let rgba = DynamicImage::ImageRgb8(img.clone()).to_rgba8();
        enc.encode_frame(Frame::from_parts(rgba, 0, 0, Delay::from_numer_denom_ms(*ms, 1)))?;
    }
    Ok(())
}

pub fn blob(frames: &[(RgbImage, u32)]) -> Vec<u8> {
    let raw: Vec<(Vec<u8>, u32)> = frames.iter().map(|(f, ms)| (proto::rgb565(f.as_raw()), *ms)).collect();
    proto::screen_blob(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_helpers() {
        assert!(is_video(Path::new("clip.MP4")));
        assert!(!is_video(Path::new("gato.gif")));
        assert_eq!(video_filter(10.0, false), "fps=10,scale=128:128:force_original_aspect_ratio=increase,crop=128:128");
        assert!(video_filter(12.0, true).contains("pad=128:128"));
    }
}
