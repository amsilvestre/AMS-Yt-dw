fn main() {
    // Converte icon.ico → icon_window.png (usado pelo Slint para janela/taskbar)
    // O ICO multi-tamanho nem sempre é suportado pelo Slint; PNG é mais confiável.
    convert_ico_to_png("assets/icon.ico", "assets/icon_window.png");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    slint_build::compile("ui/app.slint").unwrap();

    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon_with_id("assets/icon.ico", "1");
        res.compile().unwrap();
    }
}

fn convert_ico_to_png(src: &str, dst: &str) {
    use image::imageops::FilterType;
    match image::open(src) {
        Ok(img) => {
            let sized = img.resize_exact(256, 256, FilterType::Lanczos3);
            if let Err(e) = sized.save(dst) {
                eprintln!("cargo:warning=Não foi possível salvar {dst}: {e}");
            }
        }
        Err(e) => eprintln!("cargo:warning=Não foi possível abrir {src}: {e}"),
    }
}
