# AMS YouTube Downloader

> Interface gráfica Windows para o **yt-dlp** — baixe vídeos e áudios do YouTube (e centenas de outros sites) com facilidade.

Desenvolvido em **Rust** com interface **Slint**.

---

## Funcionalidades

- **Cole o link** de qualquer vídeo ou playlist do YouTube
- **Escolha a qualidade**: Melhor disponível, 4K, 1080p, 720p, 480p, 360p ou menor tamanho
- **Escolha o container**: Auto, MP4, MKV ou WebM
- **Somente áudio**: extrai em MP3, M4A, OGG, WAV ou FLAC
- **Baixar playlist inteira** com numeração automática
- **Trecho específico**: define horário de início e fim (HH:MM:SS)
- **Legendas**: baixa e incorpora legendas em PT e EN
- **Thumbnail**: incorpora a capa do vídeo no arquivo
- **SponsorBlock**: remove automaticamente patrocínios, intros e outros segmentos
- **Log em tempo real**: acompanhe o progresso e a saída do yt-dlp
- **Pasta de destino** configurável com atalho para abrir no Explorer

---

## Capturas de tela

![AMS YouTube Downloader](assets/screenshot.png)

---

## Requisitos

O app em si é um único `.exe`, mas precisa das ferramentas abaixo na **mesma pasta** ou no **PATH do sistema**:

| Ferramenta | Obrigatório | Download |
|---|---|---|
| `yt-dlp.exe` | ✅ Sim | [github.com/yt-dlp/yt-dlp/releases](https://github.com/yt-dlp/yt-dlp/releases) |
| `ffmpeg.exe` | ✅ Sim (para mesclar formatos, converter áudio, recorte de trecho) | [ffmpeg.org/download.html](https://ffmpeg.org/download.html) → Windows builds |
| Node.js | ⚠️ Recomendado | [nodejs.org](https://nodejs.org) — evita aviso de runtime JS do yt-dlp |

> **Nota:** sem o Node.js o yt-dlp ainda funciona, mas pode exibir um aviso sobre runtime JavaScript. O app detecta automaticamente se o Node.js estiver instalado e o configura sem intervenção.

---

## Como usar

1. Baixe o `AMS_YT_Downloader.exe` da [página de releases](../../releases)
2. Coloque `yt-dlp.exe` e `ffmpeg.exe` na mesma pasta
3. Execute `AMS_YT_Downloader.exe`
4. Cole o link do vídeo, configure as opções e clique em **Baixar**

---

## Como compilar

### Pré-requisitos

- [Rust](https://rustup.rs) (stable, 1.75+)
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/pt-br/visual-cpp-build-tools/) (para compilar no Windows)

### Passos

```bash
git clone https://github.com/amsilvestre/AMS-Yt-dw.git
cd AMS-Yt-dw

cargo build --release
```

O binário estará em `target/release/ams-yt-dw.exe`.

> O `build.rs` converte automaticamente o `assets/icon.ico` para `assets/icon_window.png` e embute o ícone no executável.

---

## Estrutura do projeto

```
AMS-Yt-dw/
├── src/
│   └── main.rs          # Lógica principal (Rust)
├── ui/
│   └── app.slint        # Interface gráfica (Slint)
├── assets/
│   └── icon.ico         # Ícone do aplicativo
├── build.rs             # Script de build (ícone + Slint)
├── Cargo.toml
└── Cargo.lock
```

---

## Dependências principais

| Crate | Uso |
|---|---|
| [`slint`](https://slint.dev) | Framework de UI nativa |
| [`rfd`](https://crates.io/crates/rfd) | Diálogo de seleção de pasta |
| [`dirs`](https://crates.io/crates/dirs) | Pasta Downloads padrão do usuário |
| [`winresource`](https://crates.io/crates/winresource) | Embutir ícone no `.exe` |
| [`image`](https://crates.io/crates/image) | Converter ICO → PNG (build) |

---

## Créditos

- **yt-dlp** — [github.com/yt-dlp/yt-dlp](https://github.com/yt-dlp/yt-dlp)
- **FFmpeg** — [ffmpeg.org](https://ffmpeg.org)
- Ícone por **Hopstarter** (3D Cartoon Vol.2)

---

## Licença

MIT
