# Plano de porte para Linux — AMS YouTube Downloader

Status: **Fases 1–6 concluídas.** Versão `1.3.0`.

> ✅ `cargo build --release` conclui em x86_64 Fedora (rustc 1.98.1) — binário de 17 MB,
> `cargo clippy` limpo no código novo.
>
> ⚠️ Ressalvas honestas sobre essa validação:
> o link foi feito com um **shim local** de `fontconfig.pc` (o `fontconfig-devel`
> exige root) — falta um build limpo com o pacote de verdade; e o processo
> sobreviveu 10 s sem escrever nada em stderr, o que **não prova** que a janela
> foi desenhada. Conferência visual pendente com o usuário.
>
> Não entregue na Fase 1, por dependerem de execução: **1.6(d)** (renderização de
> emoji) e **1.6(e)** (`ffmpeg` órfão ao cancelar — adiado, não é regressão).

---

## 0. Diagnóstico rápido

A base já é quase multiplataforma: Rust + Slint + `std::process::Command`. Não há
nenhuma API exclusiva do Windows no núcleo — o app apenas monta uma linha de comando
para o `yt-dlp` e lê o `stdout`/`stderr`.

O que **realmente** amarra o porte é uma decisão de arquitetura já existente:
`find_tool()` ([src/main.rs:21](../src/main.rs#L21)) procura as ferramentas
(1) ao lado do próprio executável, (2) no diretório atual, (3) no `PATH`.
Essa ordem é o que define qual formato de pacote faz sentido no Linux (ver Fase 3).

---

## Fase 1 — Correções de código (obrigatórias) — ✅ IMPLEMENTADA

Ordenadas por gravidade. As três primeiras **compilam e rodam**, mas se comportam
errado no Linux — não aparecem em revisão de código, só em teste manual.

### 1.1 Nomes das ferramentas com `.exe` fixo 🔴 bloqueante — ✅ feito

`find_tool("ffmpeg.exe")` ([src/main.rs:100](../src/main.rs#L100)) e
`find_tool("yt-dlp.exe")` ([src/main.rs:247](../src/main.rs#L247), [:334](../src/main.rs#L334))
nunca encontram nada no Linux. Cai no fallback do `PATH` procurando literalmente
`yt-dlp.exe` → falha sempre.

```rust
// src/main.rs
#[cfg(windows)]
const YTDLP: &str  = "yt-dlp.exe";
#[cfg(windows)]
const FFMPEG: &str = "ffmpeg.exe";

#[cfg(not(windows))]
const YTDLP: &str  = "yt-dlp";
#[cfg(not(windows))]
const FFMPEG: &str = "ffmpeg";
```

E trocar as três chamadas para `find_tool(YTDLP)` / `find_tool(FFMPEG)`.

### 1.2 Clipboard esvazia sozinho (chave Pix) 🔴 — ✅ feito

`app.on_copy_pix_key` ([src/main.rs:481](../src/main.rs#L481)) cria o
`arboard::Clipboard` dentro do `if let` e o **descarta na mesma linha**.

No Windows isso funciona (o clipboard é um buffer do sistema). No Linux o backend é
X11 (o `Cargo.lock` traz só `x11rb`, sem `wl-clipboard-rs`, ou seja
`wayland-data-control` está desligado — no GNOME/Wayland roda via XWayland).
No X11 o clipboard é **baseado em posse**: o processo dono precisa continuar vivo
para servir a seleção. Ao dropar o `Clipboard`, o conteúdo some.

**Sintoma:** o botão pisca "Copiado!", e o Ctrl+V em outro app cola o conteúdo antigo.

Correção — manter **uma** instância viva pelo tempo do processo:

```rust
// em main(), antes dos callbacks
let clipboard = Arc::new(Mutex::new(arboard::Clipboard::new()));

app.on_copy_pix_key(move || {
    match clipboard.lock().unwrap().as_mut() {
        Ok(cb) => { let _ = cb.set_text(PIX_KEY); /* feedback visual */ }
        Err(e)  => { /* logar: hoje o erro é engolido pelo `if let Ok` */ }
    }
});
```

Ponto secundário no mesmo trecho: `if let Ok(mut ctx) = ...` **engole a falha**.
Numa sessão sem XWayland o botão não faz nada e não escreve nada no log.

**Teste que discrimina:** copiar a chave e colar em outro aplicativo.
`set_text()` retornar `Ok` não prova nada.

### 1.3 Botão do GitHub é no-op 🟠 — ✅ feito

`app.on_open_github` ([src/main.rs:470](../src/main.rs#L470)) só tem o braço
`#[cfg(target_os = "windows")]` — no Linux o corpo do closure compila vazio.
Curiosamente `on_open_output_folder` ([:512](../src/main.rs#L512)) **já** tem o
braço `xdg-open`. Basta espelhar, de preferência extraindo um helper:

```rust
fn open_path_or_url(target: &str) {
    #[cfg(target_os = "windows")]
    let _ = Command::new("cmd").args(["/c", "start", "", target])
        .creation_flags(CREATE_NO_WINDOW).spawn();
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(target).spawn();
    #[cfg(target_os = "linux")]
    let _ = Command::new("xdg-open").arg(target).spawn();
}
```

### 1.4 `--js-runtimes` pode quebrar todo download 🟠 — ✅ feito

`build_args` ([src/main.rs:95](../src/main.rs#L95)) adiciona `--js-runtimes`
incondicionalmente quando encontra node/deno. Se o `yt-dlp` instalado for anterior
a essa flag, ele sai com erro de argumento desconhecido e **nenhum download funciona**.

No Windows isso nunca aparece porque o CI empacota o yt-dlp mais recente. No Linux,
com yt-dlp vindo do repositório da distro (que costuma ficar atrás), é exatamente
onde morde. Duas saídas, complementares:

- Detectar suporte uma vez no startup (`yt-dlp --help` e procurar `--js-runtimes`,
  guardado num `OnceLock`) e só então usar a flag.
- Declarar versão mínima de `yt-dlp` nas dependências do pacote (Fase 3).

Bônus barato: `find_js_runtime` ([:59](../src/main.rs#L59)) testa `node.exe`/`deno.exe`
— inofensivo, mas dá para gatear por `cfg`.

### 1.5 Pasta de saída pode virar `/` 🟠 — ✅ feito

`ui/app.slint:115` declara `in-out property <string> output-folder;` **sem default**,
e `main()` só a preenche se `dirs::download_dir()` devolver `Some`
([src/main.rs:463](../src/main.rs#L463)). No Linux, `download_dir()` depende do
`xdg-user-dirs` — que pode estar ausente (servidor, container, instalação mínima,
locale sem `~/Downloads`).

Se retornar `None`, `build_args` monta `-o "/%(title)s.%(ext)s"` e o yt-dlp tenta
escrever na **raiz do sistema de arquivos**. Com usuário comum dá "permission denied"
e o erro que aparece no log não tem relação óbvia com a causa.

```rust
let dl = dirs::download_dir()
    .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
    .or_else(dirs::home_dir)
    .unwrap_or_else(|| std::path::PathBuf::from("."));
app.set_output_folder(dl.to_string_lossy().into_owned().into());
```

E uma guarda defensiva em `build_args`: pasta vazia → `.`.

### 1.7 Ícone genérico na dock (Wayland) — ✅ feito

Descoberto ao empacotar, não previsto no plano original. O backend winit do Slint
só chama `xdg_toplevel.set_app_id` se `WindowInner::xdg_app_id()` estiver
preenchido — e nada o preenche por padrão. Sem `app_id` o compositor não consegue
casar a janela com o `ams-yt-dw.desktop`, e o GNOME mostra ícone genérico por mais
correto que o `.desktop` esteja.

**Medido, não deduzido** (`WAYLAND_DEBUG=1` imprime o protocolo):

| | `set_app_id` | `set_title` |
|---|---|---|
| antes | **0** | 2 |
| depois | **1** — `xdg_toplevel.set_app_id("ams-yt-dw")` | 2 |

A correção é `slint::set_xdg_app_id("ams-yt-dw")`, mas **a posição da chamada é
uma armadilha**: colocada antes de `AppWindow::new()` ela retorna
`Err(NoPlatform)` em silêncio, porque `with_global_context` ainda não tem
contexto. Tem que ficar depois de `AppWindow::new()` e antes de `run()`. Na
primeira tentativa eu errei isso e a medição pegou (continuava 0).

No X11 o `WM_CLASS` já era `"ams-yt-dw"` mesmo antes (confirmado com `xprop`),
então o problema era exclusivo do Wayland.

### 1.6 Itens menores — ✅ (a) (b) (c) · ⏳ (d) (e)

| # | Item | Onde | Ação |
|---|---|---|---|
| a | `output_folder.replace('\\', "/")` | [:105](../src/main.rs#L105) | Windows-ism. No Linux `\` é caractere **legal** em nome de pasta — isso corrompe o caminho. Gatear com `#[cfg(windows)]`. |
| b | `windows_subsystem` | [:1](../src/main.rs#L1) | Ignorado fora do Windows. Nada a fazer. |
| c | `font-family: "Consolas"` | [ui/app.slint](../ui/app.slint) | ✅ Virou a propriedade `mono-font`, preenchida pelo Rust. **Confirmado nesta máquina:** `DejaVu Sans Mono` **não** está instalado (`fc-list \| grep -c dejavu` → 0) e `fc-match monospace` responde `Noto Sans Mono`. Chutar a família teria reproduzido exatamente o desalinhamento que se queria corrigir — por isso a família é resolvida em runtime via `fc-match`, com `Liberation Mono` como fallback. |
| d | Emojis na UI (`✅ ❌ ⛔ 🚀 🔍 ▸`) | vários | ⏳ **Pendente — só verificável rodando o app.** Podem virar quadrados sem `Noto Color Emoji`. Se falhar: recomendar o pacote de fontes ou trocar por glifos ASCII. |
| e | `child.kill()` no cancelamento | [src/main.rs](../src/main.rs) | ⏳ **Adiado de propósito.** Mata só o `yt-dlp`; um `ffmpeg` filho pode sobreviver. Comportamento idêntico ao do Windows hoje, então não é regressão do porte. |

---

## Fase 2 — Compilar localmente — 🟡 VALIDADA VIA SHIM

Backend padrão do Slint = `winit` + `femtovg`.

**Resultado do primeiro build real (não é estimativa):** a única dependência de
sistema que o build cobrou foi o **fontconfig**. `libxkbcommon`, Wayland, X11,
GL/EGL e cursores foram todos resolvidos via `dlopen` ou por crates em Rust puro
(`x11rb`) — nenhum `-devel` foi necessário. A lista longa que eu havia previsto
era exagerada.

> ⚠️ **Mas essa lista ainda não é final.** O link passou por um `fontconfig.pc`
> escrito à mão (ver "becos sem saída" abaixo), que declara menos do que o `.pc`
> real. O arquivo oficial pode arrastar `freetype2`/`expat` via `Requires:`, e aí
> o `pkg_config::find_library` cobraria mais pacotes. **Confirmar com um build
> limpo** depois de `sudo dnf install fontconfig-devel`.

**Fedora:**
```bash
sudo dnf install fontconfig-devel
```
(`gcc`, `make` e `pkgconf` já vêm na maioria das instalações; confira com `which gcc make pkg-config`.)

**Debian/Ubuntu (o que o CI vai usar)** — extrapolado do Fedora, **não testado**:
```bash
sudo apt install build-essential pkg-config libfontconfig-dev
```

Toolchain, sem root:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
cargo build --release
```

### Becos sem saída já mapeados

- `RUST_FONTCONFIG_DLOPEN=1` **não** funciona. O `yeslogic-fontconfig-sys` compila,
  mas o crate `fontique` (layout de texto do Slint) importa os símbolos
  diretamente e a variante dlopen expõe outra superfície de API → erro `E0432`
  com ~28 imports não resolvidos.
- `--features slint/renderer-software` não dispensa o fontconfig: quem o exige é o
  `fontique`, não o renderizador.
- Se o build precisar rodar sem root (CI restrito, container), dá para satisfazer o
  `pkg-config` com um `fontconfig.pc` local + symlink
  `libfontconfig.so → libfontconfig.so.1`, apontando `PKG_CONFIG_PATH` e
  `RUSTFLAGS="-L ..."`. Só o **link** precisa disso; o binário final carrega a
  `libfontconfig.so.1` do sistema normalmente (confirmado com `ldd`).

Observação: `i-slint-backend-qt` está no `Cargo.lock`. Se houver Qt dev instalado, o
Slint pode escolher o backend Qt e a aparência muda. Para builds reprodutíveis,
fixar `SLINT_BACKEND=winit` no ambiente de build/CI.

`build.rs` já estava correto: o `winresource` está sob `#[cfg(target_os = "windows")]`
e a conversão ICO→PNG é multiplataforma.

---

## Fase 3 — Empacotamento — ✅ FEITA (tarball + .deb + .rpm)

Decisões tomadas conforme a recomendação: **sem bundlar** yt-dlp/ffmpeg (dependência
de sistema, para o yt-dlp continuar recebendo atualização da distro), **Flatpak
descartado**, **AppImage adiado**.

Artefatos gerados e conferidos nesta máquina:

| Arquivo | Tamanho | Como gerar |
|---|---|---|
| `AMS_YT_Downloader-1.1.0-linux-x86_64.tar.gz` | ~5 MB | `packaging/build-tarball.sh` |
| `ams-yt-dw_1.1.0-1_amd64.deb` | 4,7 MB | `cargo deb` |
| `ams-yt-dw-1.1.0-1.x86_64.rpm` | 5,1 MB | `cargo generate-rpm` |

Metadados em `[package.metadata.deb]` / `[package.metadata.generate-rpm]` no
`Cargo.toml`. Ferramentas: `cargo install cargo-deb cargo-generate-rpm`.

### Dependências — medidas, não estimadas

`ldd` mostra só o que está **ligado**; a pilha de janela/GL é aberta via `dlopen`
e não aparece ali. Rodei o app e li `/proc/PID/maps` para pegar o conjunto real:

| Biblioteca | Como entra | Pacote Fedora | Pacote Debian *(extrapolado)* |
|---|---|---|---|
| `libfontconfig.so.1` | ligada | `fontconfig` | `libfontconfig1` |
| `libxkbcommon.so.0` | dlopen | `libxkbcommon` | `libxkbcommon0` |
| `libwayland-client.so.0` | dlopen | `libwayland-client` | `libwayland-client0` |
| `libwayland-egl.so.1` | dlopen | `libwayland-egl` | `libwayland-egl1` |
| `libEGL.so.1` | dlopen | `libglvnd-egl` | `libegl1` |

`libX11`/`libxcb` também aparecem no `maps`, mas entram pela pilha EGL do driver,
não pelo app (o `x11rb` é Rust puro e fala o protocolo por socket) — por isso não
foram declarados.

No RPM, o ffmpeg entra como **dependência rica**: `(ffmpeg or ffmpeg-free)`.
É necessário porque no Fedora o ffmpeg pleno está no RPM Fusion, mas o
`ffmpeg-free` do repositório principal já resolve — assim o pacote instala sem
forçar repositório de terceiros. Verificado com `rpm -qpR`. No `.deb` o ffmpeg é
dependência normal (está no main do Debian). `yt-dlp` fica como
`Recommends` nos dois, com o `install.sh` avisando se faltar.

### Ainda não feito

- **AppImage** — adiado. Se for em frente, `find_tool` precisa sondar `$APPIMAGE`
  e `$APPDIR` (ver 3.1 no histórico deste doc), senão "colocar o yt-dlp na mesma
  pasta" para de funcionar.
- Nenhum dos pacotes foi **instalado** de verdade (`dnf`/`apt` pedem root). O que
  foi validado: conteúdo, dependências e metadados via `rpm -qlp`, `rpm -qpR` e o
  `control` do `.deb`.
- Só `x86_64`. `aarch64` fica para depois.

---

## Fase 4 — Integração com o desktop — ✅ FEITA

`packaging/ams-yt-dw.desktop`, aprovado sem ressalvas pelo `desktop-file-validate`.
`Categories=AudioVideo;Recorder;` — a primeira versão tinha cinco categorias
principais e o validador avisou que o app apareceria repetido no menu.

`StartupWMClass=ams-yt-dw` **conferido com `xprop`**, não chutado:
`WM_CLASS(STRING) = "ams-yt-dw", "ams-yt-dw"`. E o `app_id` do Wayland só passou a
ser emitido com a correção da 1.7 — sem ela o ícone da dock seria genérico nos
dois casos.

Ícone: `assets/icon_window.png` (256×256, gerado pelo `build.rs`) instalado em
`hicolor/256x256/apps/ams-yt-dw.png`.

O `install.sh` do tarball instala em `~/.local` sem root, reescreve o `Exec` para
caminho absoluto (`~/.local/bin` não costuma estar no PATH de sessão gráfica — o
compositor não lê o `.bashrc`), atualiza os caches de `.desktop`/ícone e avisa se
`yt-dlp`/`ffmpeg` faltarem. **Testado de ponta a ponta nesta máquina**, inclusive
o `--uninstall`.

---

## Fase 5 — CI/CD — ✅ FEITA

`.github/workflows/release.yml` reescrito. Antes era **um** job Windows que
buildava e publicava. Agora:

```
check-version ─┬─→ build-windows ─┐
               └─→ build-linux   ─┴─→ release (needs: ambos)
```

O job `release` é separado de propósito: dois jobs publicando na mesma release
competem entre si e o resultado depende de quem termina primeiro. Cada build
sobe `actions/upload-artifact`; o `release` faz `download-artifact` e publica
**uma vez**.

### `check-version` — guard novo

Falha o pipeline se a tag não bater com a versão do `Cargo.toml`. Isso corrige um
problema real que já existia: o job Windows tira a versão **da tag** para nomear o
instalador, mas o diálogo "Sobre" do app lê `CARGO_PKG_VERSION`. Uma tag `v1.2.0`
com `Cargo.toml` em `1.1.0` gera um `Setup_v1.2.0.exe` que se apresenta como
1.1.0 — sem nenhum aviso.

**Isso não é hipotético: já aconteceu neste repositório.** A release `v1.2.0`,
publicada em 16/03/2026, foi construída a partir do commit `8233b92`, cujo
`Cargo.toml` está em `1.1.0`. Quem instalou aquela versão vê "1.1.0" no diálogo
Sobre. Foi essa descoberta que empurrou o porte para `v1.3.0` — a `v1.2.0` já
estava ocupada. Lógica do guard testada localmente nos dois sentidos.

### Job Linux

- **`ubuntu-22.04`, não `ubuntu-latest`.** É a imagem com o glibc mais antigo
  ainda oferecida — confirmado no README do `actions/runner-images`, não de
  memória. O binário não roda em distro com glibc anterior ao da máquina de
  build, então buildar no mais novo estreitaria o alcance dos pacotes à toa.
- Instala só `libfontconfig-dev` (o resto é dlopen) e `rpm`, este último pelo
  `/usr/lib/rpm/find-requires`, que o `cargo-generate-rpm` usa para resolver as
  dependências de soname.
- `SLINT_BACKEND=winit` fixado para o build não variar caso a imagem passe a
  trazer Qt.
- Um passo `Conferir metadados` roda `rpm -qpR` e `rpm -qlp` no artefato — assim
  uma regressão de dependência aparece no log do CI, não na máquina do usuário.

O corpo da release ganhou a tabela de Linux e a explicação de por que yt-dlp e
ffmpeg não vêm embutidos.

### Primeira execução — resultado

Rodou na tag `v1.3.0` (run `34167365068`, 07/09/2026). **Os quatro jobs passaram**;
uma release única com os 5 artefatos foi publicada.

| Job | Resultado | Duração |
|---|---|---|
| `check-version` | ✅ | 3 s |
| `build-linux` | ✅ | 7 min 48 s |
| `build-windows` | ✅ | 8 min 21 s |
| `release` | ✅ | 14 s |

As duas incógnitas que restavam, resolvidas:

- **`libfontconfig-dev`** era o nome certo no Ubuntu 22.04 — e já vinha na imagem
  do runner (`is already the newest version`).
- **O `find-requires` foi encontrado.** O RPM publicado traz **28 dependências de
  soname** além das explícitas, ou seja o `auto-req` funcionou de verdade e não
  degradou.

A escolha do `ubuntu-22.04` se pagou: o RPM exige no máximo `GLIBC_2.35`. Se
tivesse sido buildado no `ubuntu-latest` (24.04) o piso subiria e excluiria
distros mais antigas sem necessidade — para efeito de comparação, esta Fedora
tem glibc 2.43.

**Pendência nova:** o run emitiu avisos de que `actions/checkout@v4`,
`actions/cache@v4`, `actions/upload-artifact@v4`, `actions/download-artifact@v4`
e `softprops/action-gh-release@v2` ainda miram Node.js 20, que está depreciado e
sendo forçado para o Node 24. Não quebra hoje; vai quebrar quando o suporte cair.

---

## Fase 6 — Documentação — ✅ FEITA

README reescrito: deixou de ser "interface gráfica Windows". Ganhou seção de
**Instalação** por plataforma (rpm/deb/tarball), tabela de requisitos com coluna
Windows e Linux, instruções de build separadas (Linux precisa só de
`fontconfig`), como gerar os pacotes, e `packaging/` + `docs/` na estrutura do
projeto. Screenshot do Linux capturado e adicionado ao lado do de Windows.

---

## Checklist de aceitação (testar no Fedora/GNOME/Wayland)

- [x] `cargo build --release` conclui sem erro (Fedora, rustc 1.98.1)
- [x] Janela abre e o ícone na dock é o correto (confirmado pelo usuário após
      instalar o RPM)
- [ ] Download simples 1080p conclui e o arquivo aparece na pasta
- [ ] Somente áudio MP3 conclui (prova que o `--ffmpeg-location` resolveu)
- [ ] Recorte por trecho (`--download-sections`) conclui
- [x] Fonte monoespaçada aplicada de fato no log (visível no screenshot). O
      alinhamento da tabela do `-F` em si ainda não foi exercitado.
- [ ] Cancelar interrompe o download de verdade
- [ ] Botão "abrir pasta" abre o Arquivos
- [ ] Botão GitHub abre o navegador
- [ ] **Copiar chave Pix → colar em OUTRO aplicativo** (não só verificar o "Copiado!")
- [ ] Seletor de pasta abre (exige `xdg-desktop-portal` + backend; ausente numa
      instalação mínima, o `pick_folder()` devolve `None` em silêncio)
- [x] Glifos da UI (▶ ♥ ↗ ⬇ ▸ ✓) renderizam — visto no screenshot. Os emojis
      coloridos das mensagens de log (✅ ❌ ⛔ 🚀) não apareceram na captura
      porque o log estava vazio; o `Noto Color Emoji` está instalado.
- [x] Pasta padrão resolveu para `/home/amsilvestre/Downloads` (visto no
      screenshot). O caso `XDG_DOWNLOAD_DIR` ausente segue sem teste.
- [x] Lógica do `--js-runtimes` conferida contra o yt-dlp real (2026.08.19,
      instalado pelo `Recommends` do RPM): suporta a flag, e há node no PATH →
      a flag é passada. O caminho do yt-dlp antigo continua sem teste real.

---

## Dívida técnica encontrada de passagem (fora do escopo do porte)

`cargo clippy` aponta três `.lines().flatten()` em `spawn_download` e
`spawn_fetch_formats`: se a leitura do pipe falhar de forma persistente, o
`flatten()` gira para sempre em vez de encerrar. Pré-existente e independente de
plataforma; correção é trocar por `.map_while(Result::ok)`. Não mexi — não é
Fase 1.

---

## Ordem de execução sugerida

1. **Fase 1** inteira (1 sessão) → app funcional via `cargo run` no Fedora
2. **Fase 2** + checklist manual → validar comportamento real
3. Tarball portátil + `.desktop` (Fases 3/4, versão barata)
4. `.deb` + `.rpm`
5. Reestruturar o CI (Fase 5)
6. AppImage (só se ainda fizer sentido depois de 4)
7. README + release `v1.3.0`

---

## Em aberto / decisões suas

- Empacotar (a) só tarball, (b) tarball + deb + rpm, ou (c) tudo incluindo AppImage?
- Bundlar yt-dlp junto (como no instalador Windows) ou depender do sistema?
  Bundlar dá paridade com o Windows, mas o yt-dlp embutido envelhece e quebra —
  no Linux a dependência de sistema é a opção mais saudável.
- Manter um binário único multiplataforma (recomendado) ou abrir um fork/branch Linux?

---

## O que ficou sem teste

Registrado para não virar falsa sensação de cobertura:

- **Instalar/desinstalar os pacotes** — o `.rpm` 1.1.0 foi instalado pelo usuário
  e o ícone apareceu certo, mas `.deb` e tarball não passaram por um ciclo
  completo, e o RPM 1.3.0 publicado é posterior a esse teste.
- **Nomes de pacotes Debian** — extrapolados do Fedora. O `libfontconfig-dev` do
  build foi confirmado pelo CI, mas os `Depends` declarados no `.deb`
  (`libegl1`, `libwayland-egl1`, …) só serão exercitados quando alguém instalar
  o pacote num Debian/Ubuntu de verdade.
- **Um download de verdade** pela UI: yt-dlp e ffmpeg estão instalados, mas
  nenhum download foi exercitado ponta a ponta.
- **Clipboard da chave Pix** — a correção-âncora da Fase 1. Nunca foi feito o
  teste de colar em outro aplicativo.
- **AppImage** e **aarch64** — fora de escopo, ver Fase 3.
