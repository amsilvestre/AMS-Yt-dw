#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

slint::include_modules!();

// Windows: hide the console window spawned by child processes
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

// ── Nomes das ferramentas externas ───────────────────────────────────────────

#[cfg(windows)]
const YTDLP: &str = "yt-dlp.exe";
#[cfg(windows)]
const FFMPEG: &str = "ffmpeg.exe";

#[cfg(not(windows))]
const YTDLP: &str = "yt-dlp";
#[cfg(not(windows))]
const FFMPEG: &str = "ffmpeg";

/// Fonte monoespaçada do log — "Consolas" não existe fora do Windows.
#[cfg(windows)]
fn mono_font() -> String {
    "Consolas".to_owned()
}

#[cfg(target_os = "macos")]
fn mono_font() -> String {
    "Menlo".to_owned()
}

/// O Slint consulta a fonte pelo **nome da família**; o alias genérico "monospace"
/// não resolve e cairia silenciosamente na fonte sans padrão — justamente o
/// desalinhamento da tabela de formatos que queremos evitar. Perguntamos ao
/// fontconfig qual família o sistema usa. Qual delas é varia por distro
/// (Fedora recente responde "Noto Sans Mono", não "DejaVu Sans Mono").
#[cfg(all(not(windows), not(target_os = "macos")))]
fn mono_font() -> String {
    if let Ok(out) = Command::new("fc-match")
        .args(["-f", "%{family[0]}", "monospace"])
        .output()
    {
        if out.status.success() {
            let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if !name.is_empty() {
                return name;
            }
        }
    }
    // fc-match pode não estar instalado (sistema só com libfontconfig).
    "Liberation Mono".to_owned()
}

// ── Tool finder ──────────────────────────────────────────────────────────────

/// Looks for `name` (e.g. "yt-dlp") next to the running executable first,
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
    #[cfg(windows)]
    let candidates: &[&str] = &["node.exe", "deno.exe"];
    #[cfg(not(windows))]
    let candidates: &[&str] = &["node", "deno"];

    for candidate in candidates {
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

/// `--js-runtimes` só existe em versões recentes do yt-dlp. Passar a flag para uma
/// versão antiga faz o yt-dlp abortar por argumento desconhecido — e aí *nenhum*
/// download funciona. No Linux o yt-dlp costuma vir do repositório da distro, que
/// fica atrás, então a flag precisa ser condicional. Sondado uma única vez.
fn ytdlp_supports_js_runtimes() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let mut cmd = Command::new(find_tool(YTDLP));
        cmd.arg("--help").stdin(Stdio::null());
        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);
        match cmd.output() {
            Ok(out) => String::from_utf8_lossy(&out.stdout).contains("--js-runtimes"),
            Err(_) => false,
        }
    })
}

// ── Abrir URL / pasta no aplicativo padrão ───────────────────────────────────

fn open_url(url: &str) {
    #[cfg(target_os = "windows")]
    let _ = Command::new("cmd")
        .args(["/c", "start", "", url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(url).spawn();
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let _ = Command::new("xdg-open").arg(url).spawn();
}

/// Abre uma pasta no gerenciador de arquivos. Separado de `open_url` de propósito:
/// o caminho vem de um campo editável pelo usuário e não pode passar pelo parser
/// do cmd.exe (um `&` ou `|` no nome seria reinterpretado).
fn open_folder(path: &str) {
    if path.is_empty() {
        return;
    }
    #[cfg(target_os = "windows")]
    let _ = Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(path).spawn();
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let _ = Command::new("xdg-open").arg(path).spawn();
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
    // Ordem importa: sem node/deno instalado nem chegamos a sondar o yt-dlp.
    if let Some(runtime) = find_js_runtime() {
        if ytdlp_supports_js_runtimes() {
            a.push("--js-runtimes".into());
            a.push(runtime);
        }
    }

    // ── FFmpeg location ───────────────────────────────────────────────────────
    let ffmpeg = find_tool(FFMPEG);
    a.push("--ffmpeg-location".into());
    a.push(ffmpeg);

    // ── Output template ───────────────────────────────────────────────────────
    // Pasta vazia viraria "-o /%(title)s..." → tentativa de escrita na raiz.
    let folder = if output_folder.trim().is_empty() {
        "."
    } else {
        output_folder
    };
    // Normalizar "\\" só faz sentido no Windows: no Linux é caractere legal em nomes.
    #[cfg(windows)]
    let folder = folder.replace('\\', "/");
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
    // Montado dentro da thread: build_args sonda o yt-dlp e não pode travar a UI.
    thread::spawn(move || {
        let ytdlp = find_tool(YTDLP);
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
    thread::spawn(move || {
        let ytdlp = find_tool(YTDLP);
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

    // Wayland/X11: sem app_id o compositor não casa a janela com o
    // ams-yt-dw.desktop e a dock mostra um ícone genérico. Medido com
    // WAYLAND_DEBUG=1: sem esta chamada o protocolo nunca recebe um
    // xdg_toplevel.set_app_id.
    //
    // Ordem importa: precisa ser DEPOIS de AppWindow::new() (que inicializa o
    // contexto do Slint — antes disso a função devolve Err(NoPlatform) em
    // silêncio) e ANTES de run(), que é quando a janela é exibida.
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Err(e) = slint::set_xdg_app_id("ams-yt-dw") {
        eprintln!("Aviso: não foi possível definir o app id XDG: {e}");
    }

    // Versão do app
    app.set_app_version(env!("CARGO_PKG_VERSION").into());

    // Fonte monoespaçada do log (varia por plataforma)
    app.set_mono_font(mono_font().into());

    // QR Code Pix
    let payload = pix_br_code(PIX_KEY, "AMS Silvestre", "Rio de Janeiro");
    app.set_qr_image(generate_qr_image(&payload));

    // Default output folder → user's Downloads directory.
    // No Linux `download_dir()` depende do xdg-user-dirs, que pode não existir;
    // sem fallback a pasta ficaria vazia e o yt-dlp tentaria escrever na raiz.
    let downloads = dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    app.set_output_folder(downloads.to_string_lossy().into_owned().into());

    let cancel = Arc::new(Mutex::new(false));

    // ── Abrir GitHub ──────────────────────────────────────────────────────────
    app.on_open_github(|| {
        open_url("https://github.com/amsilvestre/AMS-Yt-dw");
    });

    // ── Copiar chave Pix ──────────────────────────────────────────────────────
    {
        let weak = app.as_weak();
        // O clipboard do X11 é baseado em posse: o processo dono precisa continuar
        // vivo para servir a seleção. Criar e dropar o Clipboard a cada clique
        // (como antes) faz o conteúdo sumir no Linux. Mantemos uma instância viva.
        let clipboard: Rc<RefCell<Option<arboard::Clipboard>>> = Rc::new(RefCell::new(None));

        app.on_copy_pix_key(move || {
            let a = match weak.upgrade() {
                Some(a) => a,
                None => return,
            };

            let mut slot = clipboard.borrow_mut();
            if slot.is_none() {
                match arboard::Clipboard::new() {
                    Ok(cb) => *slot = Some(cb),
                    Err(e) => {
                        prepend_log(&a, &format!("❌ Clipboard indisponível: {e}"));
                        return;
                    }
                }
            }

            if let Err(e) = slot.as_mut().unwrap().set_text(PIX_KEY) {
                // Instância pode ter perdido a conexão; força recriação no próximo clique.
                *slot = None;
                prepend_log(&a, &format!("❌ Não foi possível copiar a chave Pix: {e}"));
                return;
            }

            // Libera o empréstimo antes de mexer na UI.
            drop(slot);

            a.set_pix_copied(true);
            let w = weak.clone();
            slint::Timer::single_shot(std::time::Duration::from_secs(2), move || {
                if let Some(a) = w.upgrade() {
                    a.set_pix_copied(false);
                }
            });
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
                open_folder(a.get_output_folder().as_ref());
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
