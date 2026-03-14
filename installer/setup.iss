#define MyAppName      "AMS YouTube Downloader"
#define MyAppPublisher "amsilvestre"
#define MyAppURL       "https://github.com/amsilvestre/AMS-Yt-dw"
#define MyAppExeName   "AMS_YT_Downloader.exe"

; Versão pode ser sobrescrita via linha de comando: /DMyAppVersion=1.2.0
#ifndef MyAppVersion
  #define MyAppVersion "1.0.0"
#endif

; ── Configurações gerais ──────────────────────────────────────────────────────
[Setup]
AppId={{C3F7A2B1-D849-4E6F-A012-3BC8E9F54D70}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}/releases

; Instala em C:\Program Files\AMS YouTube Downloader
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}

; Saída
OutputDir=output
OutputBaseFilename=AMS_YT_Downloader_Setup_v{#MyAppVersion}

; Visual
SetupIconFile=..\assets\icon.ico
WizardStyle=modern
WizardSmallImageFile=..\assets\icon_window.png

; Compressão máxima
Compression=lzma2/ultra64
SolidCompression=yes

; Requer administrador (instala em Program Files)
PrivilegesRequired=admin
ArchitecturesInstallIn64BitMode=x64compatible

; Mínimo Windows 10
MinVersion=10.0

; ── Idiomas ───────────────────────────────────────────────────────────────────
[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

; ── Tarefas opcionais ─────────────────────────────────────────────────────────
[Tasks]
Name: "desktopicon"; Description: "Criar ícone na Área de Trabalho"; GroupDescription: "Ícones adicionais:"; Flags: unchecked

; ── Arquivos instalados ───────────────────────────────────────────────────────
[Files]
; App principal
Source: "..\AMS_YT_Downloader.exe"; DestDir: "{app}"; Flags: ignoreversion

; Ferramentas (baixadas pelo CI ou presentes na pasta tools\)
Source: "..\tools\yt-dlp.exe";  DestDir: "{app}"; Flags: ignoreversion
Source: "..\tools\ffmpeg.exe";  DestDir: "{app}"; Flags: ignoreversion

; Ícone
Source: "..\assets\icon.ico"; DestDir: "{app}"; Flags: ignoreversion

; ── Atalhos ───────────────────────────────────────────────────────────────────
[Icons]
Name: "{group}\{#MyAppName}";                         Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\icon.ico"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}";   Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}";                   Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\icon.ico"; Tasks: desktopicon

; ── Executa o app ao final da instalação ─────────────────────────────────────
[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Executar {#MyAppName} agora"; Flags: nowait postinstall skipifsilent
