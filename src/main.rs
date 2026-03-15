#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

slint::include_modules!();

// Windows: hide the console window spawned by child processes
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

// ── Tool finder ──────────────────────────────────────────────────────────────

/// Looks for `name` (e.g. "yt-dlp.exe") next to the running executable first,
/// then in the current working directory, and finally falls back to PATH.
fn find_tool(name: &str) -> String {
    // 1. Alongside our own binary (release case)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(name);
            if p.exists() {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    // 2. Current working directory (cargo run / dev case)
    if std::path::Path::new(name).exists() {
        return name.to_owned();
    }
    // 3. Rely on PATH
    name.to_owned()
}

// ── Progress parser ───────────────────────────────────────────────────────────

/// Parses lines like:  [download]  47.0% of 10.52MiB at  1.35MiB/s ETA 00:04
fn parse_progress(line: &str) -> Option<f32> {
    if line.contains("[download]") && line.contains('%') {
        for token in line.split_whitespace() {
            if token.ends_with('%') {
                if let Ok(pct) = token.trim_end_matches('%').parse::<f32>() {
                    return Some((pct / 100.0).clamp(0.0, 1.0));
                }
            }
        }
    }
    None
}

// ── JS runtime detector ───────────────────────────────────────────────────────

/// Returns the path to `node` or `deno` if found, so yt-dlp can use a JS runtime.
fn find_js_runtime() -> Option<String> {
    for candidate in &["node", "node.exe", "deno", "deno.exe"] {
        if Command::new(candidate)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return Some(candidate.trim_end_matches(".exe").to_owned());
        }
    }
    None
}

// ── Command builder ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn build_args(
    url: &str,
    output_folder: &str,
    quality_index: i32,
    format_index: i32,
    audio_only: bool,
    audio_format_index: i32,
    use_time_range: bool,
    start_time: &str,
    end_time: &str,
    download_subtitles: bool,
    embed_thumbnail: bool,
    sponsorblock: bool,
    playlist_mode: bool,
) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();

    // ── JS runtime (Node.js / Deno) ───────────────────────────────────────────
    if let Some(runtime) = find_js_runtime() {
        a.push("--js-runtimes".into());
        a.push(runtime);
    }

    // ── FFmpeg location ───────────────────────────────────────────────────────
    let ffmpeg = find_tool("ffmpeg.exe");
    a.push("--ffmpeg-location".into());
    a.push(ffmpeg);

    // ── Output template ───────────────────────────────────────────────────────
    let folder = output_folder.replace('\\', "/");
    let template = if playlist_mode {
        format!("{folder}/%(playlist_index)02d - %(title)s.%(ext)s")
    } else {
        format!("{folder}/%(title)s.%(ext)s")
    };
    a.push("-o".into());
    a.push(template);

    // ── Playlist ──────────────────────────────────────────────────────────────
    if !playlist_mode {
        a.push("--no-playlist".into());
    }

    // ── Format / quality ──────────────────────────────────────────────────────
    if audio_only {
        let fmts = ["mp3", "m4a", "ogg", "wav", "flac"];
        let fmt = fmts.get(audio_format_index as usize).copied().unwrap_or("mp3");
        a.push("-x".into());
        a.push("--audio-format".into());
        a.push(fmt.into());
        a.push("--audio-quality".into());
        a.push("0".into()); // best VBR quality
    } else {
        let height_filter = match quality_index {
            1 => "[height<=2160]",
            2 => "[height<=1080]",
            3 => "[height<=720]",
            4 => "[height<=480]",
            5 => "[height<=360]",
            _ => "",
        };

        let fmt_spec = if quality_index == 6 {
            "worstvideo+worstaudio/worst".to_owned()
        } else {
            format!("bestvideo{hf}+bestaudio/best{hf}", hf = height_filter)
        };

        a.push("-f".into());
        a.push(fmt_spec);

        // Container format
        let containers = ["", "mp4", "mkv", "webm"];
        let container = containers.get(format_index as usize).copied().unwrap_or("");
        if !container.is_empty() {
            a.push("--merge-output-format".into());
            a.push(container.into());
        }
    }

    // ── Time range ────────────────────────────────────────────────────────────
    if use_time_range && !start_time.is_empty() && !end_time.is_empty() {
        a.push("--download-sections".into());
        a.push(format!("*{}-{}", start_time, end_time));
        a.push("--force-keyframes-at-cuts".into());
    }

    // ── Subtitles ─────────────────────────────────────────────────────────────
    if download_subtitles {
        a.push("--write-subs".into());
        a.push("--write-auto-subs".into());
        a.push("--sub-langs".into());
        a.push("pt,en".into());
        a.push("--embed-subs".into());
    }

    // ── Thumbnail ─────────────────────────────────────────────────────────────
    if embed_thumbnail {
        a.push("--embed-thumbnail".into());
    }

    // ── SponsorBlock ──────────────────────────────────────────────────────────
    if sponsorblock {
        a.push("--sponsorblock-remove".into());
        a.push("sponsor,intro,outro,selfpromo".into());
    }

    // Progress: one line per update, no ANSI escape codes
    a.push("--newline".into());
    a.push("--no-colors".into());

    a.push(url.to_owned());
    a
}

// ── Log helper ────────────────────────────────────────────────────────────────

/// Prepends `line` to the log (newest entry at the top, max ~8 000 chars).
fn prepend_log(app: &AppWindow, line: &str) {
    let current = app.get_log_text().to_string();
    let new_text = format!("{}\n{}", line, current);
    let trimmed = if new_text.len() > 8_000 {
        new_text[..8_000].to_owned()
    } else {
        new_text
    };
    app.set_log_text(trimmed.into());
}

// ── Spawn helpers ─────────────────────────────────────────────────────────────

fn new_command(bin: &str, args: &[String]) -> std::io::Result<std::process::Child> {
    #[cfg(target_os = "windows")]
    {
        Command::new(bin)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Command::new(bin)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }
}

// ── Download thread ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn spawn_download(
    weak: slint::Weak<AppWindow>,
    url: String,
    output_folder: String,
    quality_index: i32,
    format_index: i32,
    audio_only: bool,
    audio_format_index: i32,
    use_time_range: bool,
    start_time: String,
    end_time: String,
    download_subtitles: bool,
    embed_thumbnail: bool,
    sponsorblock: bool,
    playlist_mode: bool,
    cancel: Arc<Mutex<bool>>,
) {
    let ytdlp = find_tool("yt-dlp.exe");
    let args = build_args(
        &url,
        &output_folder,
        quality_index,
        format_index,
        audio_only,
        audio_format_index,
        use_time_range,
        &start_time,
        &end_time,
        download_subtitles,
        embed_thumbnail,
        sponsorblock,
        playlist_mode,
    );

    thread::spawn(move || {
        let mut child = match new_command(&ytdlp, &args) {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("❌ Erro ao iniciar yt-dlp: {e}");
                weak.upgrade_in_event_loop(move |app| {
                    prepend_log(&app, &msg);
                    app.set_is_downloading(false);
                })
                .ok();
                return;
            }
        };

        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");

        // Read stderr in background thread
        let weak_err = weak.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().flatten() {
                if !line.trim().is_empty() {
                    let l = line.clone();
                    weak_err
                        .upgrade_in_event_loop(move |app| prepend_log(&app, &l))
                        .ok();
                }
            }
        });

        // Read stdout (progress + status lines)
        for line in BufReader::new(stdout).lines().flatten() {
            if *cancel.lock().unwrap() {
                child.kill().ok();
                break;
            }
            let progress = parse_progress(&line);
            let l = line.clone();
            weak.upgrade_in_event_loop(move |app| {
                if let Some(p) = progress {
                    app.set_progress(p);
                }
                // Show non-empty, non-redundant lines
                if !l.trim().is_empty() {
                    prepend_log(&app, &l);
                }
            })
            .ok();
        }

        let _ = child.wait();
        let cancelled = *cancel.lock().unwrap();

        weak.upgrade_in_event_loop(move |app| {
            app.set_is_downloading(false);
            if cancelled {
                app.set_progress(0.0);
                prepend_log(&app, "⛔ Download cancelado pelo usuário.");
            } else {
                app.set_progress(1.0);
                prepend_log(&app, "✅ Download concluído com sucesso!");
            }
        })
        .ok();
    });
}

// ── Fetch formats thread ──────────────────────────────────────────────────────

fn spawn_fetch_formats(weak: slint::Weak<AppWindow>, url: String) {
    let ytdlp = find_tool("yt-dlp.exe");
    thread::spawn(move || {
        let args: Vec<String> = vec![
            "-F".to_owned(),
            "--no-playlist".to_owned(),
            "--no-colors".to_owned(),
            url,
        ];
        let mut child = match new_command(&ytdlp, &args) {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("❌ Erro ao iniciar yt-dlp: {e}");
                weak.upgrade_in_event_loop(move |app| {
                    app.set_log_text(msg.into());
                    app.set_is_downloading(false);
                })
                .ok();
                return;
            }
        };

        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");

        // Discard stderr for this call
        thread::spawn(move || {
            for _ in BufReader::new(stderr).lines() {}
        });

        let mut out = String::from("=== Formatos disponíveis ===\n\n");
        for line in BufReader::new(stdout).lines().flatten() {
            out.push_str(&line);
            out.push('\n');
        }
        let _ = child.wait();

        weak.upgrade_in_event_loop(move |app| {
            app.set_log_text(out.into());
            app.set_is_downloading(false);
        })
        .ok();
    });
}

// ── Pix BR Code (EMV QR Code) ─────────────────────────────────────────────────

fn crc16_ccitt(data: &str) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for byte in data.bytes() {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

fn pix_br_code(key: &str, name: &str, city: &str) -> String {
    let emv = |id: &str, val: &str| format!("{}{:02}{}", id, val.len(), val);
    let mai = emv("00", "br.gov.bcb.pix") + &emv("01", key);
    let name = if name.len() > 25 { &name[..25] } else { name };
    let city = if city.len() > 15 { &city[..15] } else { city };
    let payload = "000201".to_owned()
        + "010211"
        + &emv("26", &mai)
        + "52040000"
        + "5303986"
        + "5802BR"
        + &emv("59", name)
        + &emv("60", city)
        + "62070503***"
        + "6304";
    format!("{}{:04X}", payload, crc16_ccitt(&payload))
}

// ── QR Code → Slint Image ─────────────────────────────────────────────────────

fn generate_qr_image(data: &str) -> slint::Image {
    let code = qrcode::QrCode::with_error_correction_level(
        data.as_bytes(),
        qrcode::EcLevel::M,
    )
    .unwrap_or_else(|_| qrcode::QrCode::new(data.as_bytes()).unwrap());

    let modules = code.width();
    let scale: usize = 6;
    let quiet: usize = 4;
    let full = (modules + 2 * quiet) * scale;

    let mut px = vec![255u8; full * full * 4]; // RGBA branco

    for (i, color) in code.to_colors().iter().enumerate() {
        if *color == qrcode::Color::Dark {
            let row = i / modules;
            let col = i % modules;
            for dy in 0..scale {
                for dx in 0..scale {
                    let idx = ((row + quiet) * scale + dy) * full + (col + quiet) * scale + dx;
                    px[idx * 4]     = 0;
                    px[idx * 4 + 1] = 0;
                    px[idx * 4 + 2] = 0;
                    // alpha já é 255
                }
            }
        }
    }

    slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
        &px,
        full as u32,
        full as u32,
    ))
}

// ── Main ──────────────────────────────────────────────────────────────────────

const PIX_KEY: &str = "pixcafe@silvestrehost.com";

fn main() {
    let app = AppWindow::new().expect("Failed to create window");

    // Versão do app
    app.set_app_version(env!("CARGO_PKG_VERSION").into());

    // QR Code Pix
    let payload = pix_br_code(PIX_KEY, "AMS Silvestre", "Rio de Janeiro");
    app.set_qr_image(generate_qr_image(&payload));

    // Default output folder → user's Downloads directory
    if let Some(dl) = dirs::download_dir() {
        app.set_output_folder(dl.to_string_lossy().into_owned().into());
    }

    let cancel = Arc::new(Mutex::new(false));

    // ── Abrir GitHub ──────────────────────────────────────────────────────────
    app.on_open_github(|| {
        #[cfg(target_os = "windows")]
        let _ = Command::new("cmd")
            .args(["/c", "start", "", "https://github.com/amsilvestre/AMS-Yt-dw"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    });

    // ── Copiar chave Pix ──────────────────────────────────────────────────────
    {
        let weak = app.as_weak();
        app.on_copy_pix_key(move || {
            if let Ok(mut ctx) = arboard::Clipboard::new() {
                let _ = ctx.set_text(PIX_KEY);
            }
            if let Some(a) = weak.upgrade() {
                a.set_pix_copied(true);
                let w = weak.clone();
                slint::Timer::single_shot(std::time::Duration::from_secs(2), move || {
                    if let Some(a) = w.upgrade() {
                        a.set_pix_copied(false);
                    }
                });
            }
        });
    }

    // ── Browse folder ─────────────────────────────────────────────────────────
    {
        let weak = app.as_weak();
        app.on_browse_folder(move || {
            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                if let Some(a) = weak.upgrade() {
                    a.set_output_folder(folder.to_string_lossy().into_owned().into());
                }
            }
        });
    }

    // ── Open output folder in Explorer ────────────────────────────────────────
    {
        let weak = app.as_weak();
        app.on_open_output_folder(move || {
            if let Some(a) = weak.upgrade() {
                let path = a.get_output_folder().to_string();
                if !path.is_empty() {
                    #[cfg(target_os = "windows")]
                    let _ = Command::new("explorer").arg(&path).spawn();
                    #[cfg(target_os = "macos")]
                    let _ = Command::new("open").arg(&path).spawn();
                    #[cfg(target_os = "linux")]
                    let _ = Command::new("xdg-open").arg(&path).spawn();
                }
            }
        });
    }

    // ── Cancel ────────────────────────────────────────────────────────────────
    {
        let c = cancel.clone();
        app.on_cancel_download(move || {
            *c.lock().unwrap() = true;
        });
    }

    // ── Fetch formats ─────────────────────────────────────────────────────────
    {
        let weak = app.as_weak();
        app.on_fetch_formats(move || {
            if let Some(a) = weak.upgrade() {
                let url = a.get_url().to_string();
                if url.is_empty() {
                    return;
                }
                a.set_is_downloading(true);
                a.set_log_text("🔍 Buscando formatos disponíveis...\n".into());
                spawn_fetch_formats(weak.clone(), url);
            }
        });
    }

    // ── Download ──────────────────────────────────────────────────────────────
    {
        let weak = app.as_weak();
        let cancel_clone = cancel.clone();
        app.on_download_clicked(move || {
            let a = match weak.upgrade() {
                Some(x) => x,
                None => return,
            };

            let url = a.get_url().to_string();
            if url.is_empty() {
                return;
            }

            // Reset cancel flag
            *cancel_clone.lock().unwrap() = false;

            a.set_is_downloading(true);
            a.set_progress(0.0);
            a.set_log_text(format!("🚀 Iniciando: {url}\n").into());

            spawn_download(
                weak.clone(),
                url,
                a.get_output_folder().to_string(),
                a.get_quality_index(),
                a.get_format_index(),
                a.get_audio_only(),
                a.get_audio_format_index(),
                a.get_use_time_range(),
                a.get_start_time().to_string(),
                a.get_end_time().to_string(),
                a.get_download_subtitles(),
                a.get_embed_thumbnail(),
                a.get_sponsorblock(),
                a.get_playlist_mode(),
                cancel_clone.clone(),
            );
        });
    }

    app.run().expect("Event loop error");
}
