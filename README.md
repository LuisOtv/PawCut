# PawCut

Editor de vídeo simples para Linux, com a cara da página **Edit do DaVinci Resolve**.
Serve para cortar, empilhar camadas, pôr overlays e títulos, mexer no áudio e exportar MP4,
com keyframes e ajuste de cor no estilo do Ctrl+U do Photoshop.

Feito em Rust: interface em egui, mídia com ffmpeg. Um único compositor gera o preview e a
exportação, então o MP4 sai igual ao que aparece no viewer.

## Rodar

```bash
scripts/install.sh            # compila e instala (menu de aplicativos + ~/.local/bin/pawcut)
pawcut                        # ou: pawcut projeto.pawcut / pawcut video.mp4 musica.mp3
```

Para rodar sem instalar: `cargo run --release -p pawcut-app`.

Requisitos (Ubuntu): Rust (rustup), `ffmpeg` com as bibliotecas de desenvolvimento
(`libavcodec-dev`, `libavformat-dev`, `libavfilter-dev`, `libswscale-dev`), PulseAudio/PipeWire
e libclang. Se faltar o `libclang-dev`, o `.cargo/config.toml` aponta o bindgen para os headers do GCC.

## O que dá para fazer

- **Media Pool:** importar vídeo, áudio e imagem pelo botão, com Ctrl+I ou arrastando arquivos para a janela.
  Arquivo que sumiu aparece como *Media Offline*; clique com o botão direito para reconectar (Relink).
- **Effects → Toolbox:** arraste *Texto* ou *Cor sólida* para uma faixa de vídeo.
- **Source Viewer:** clique numa mídia, marque In/Out (I/O) e use Insert (F9), Overwrite (F10) ou Append (Shift+F12).
- **Timeline:** várias faixas de vídeo (a de cima cobre a de baixo) e de áudio. Dá para arrastar
  clipes no tempo e entre faixas (soltar em "+ vídeo"/"+ áudio" cria a faixa), aparar pelas bordas,
  usar a lâmina (B) ou cortar no cursor (Ctrl+B), apagar com ou sem ripple, e usar snapping (N) e seleção ligada.
  Cada faixa tem botões para travar (L), ligar/desligar o vídeo (V) e mudo/solo no áudio (M/S).
  Ctrl+roda dá zoom e a roda rola a timeline.
- **Timeline Viewer:** arraste a imagem para mover o clipe selecionado.
- **Inspector:** posição, zoom e opacidade, velocidade (10%–1000%, o áudio mantém o tom), fade in/out,
  volume em dB, texto (conteúdo, tamanho, fonte, cor) e cor do gerador.
- **Áudio:** cada vídeo entra com o áudio ligado. "Separar áudio e vídeo" (Ctrl+Alt+L) desliga os dois.
  A waveform aparece na timeline.
- **Deliver:** exporta MP4 (H.264 + AAC) com barra de progresso e opção de cancelar.
- **Projeto:** salva em `.pawcut` (JSON). Há autosave a cada 5 minutos e recuperação se o app fechar sem salvar.
- **Layout:** os painéis são abas que podem ser arrastadas, divididas e fechadas.
  O menu *Workspace* reabre painéis e tem *Reset UI Layout*.
- **Atalhos editáveis:** *Editar → Preferences*. Os padrões são os do Resolve.

## Atalhos padrão

| Ação | Tecla | Ação | Tecla |
|---|---|---|---|
| Tocar/pausar | Espaço | J / K / L | trás / parar / frente (repita para acelerar) |
| Frame anterior/seguinte | ← / → | Corte anterior/seguinte | ↑ / ↓ |
| Início / fim | Home / End | In / Out | I / O |
| Seleção / Lâmina | A / B | Snapping | N |
| Cortar no cursor | Ctrl+B | Seleção ligada | Ctrl+Shift+L |
| Apagar | Backspace / Delete | Apagar com ripple | Shift+Backspace |
| Insert / Overwrite / Append | F9 / F10 / Shift+F12 | Ligar/separar A/V | Ctrl+Alt+L |
| Desfazer / Refazer | Ctrl+Z / Ctrl+Shift+Z | Salvar / Salvar como | Ctrl+S / Ctrl+Shift+S |
| Novo / Abrir | Ctrl+N / Ctrl+O | Importar | Ctrl+I |
| Exportar (Deliver) | Ctrl+Shift+E | Zoom + / − / caber | Ctrl+= / Ctrl+- / Shift+Z |
| Cortar até o cursor (ripple) | Shift+[ / Shift+] | Copiar / Colar | Ctrl+C / Ctrl+V |
| Marcador / anterior / próximo | M / Shift+↑ / Shift+↓ | Cross Dissolve | Ctrl+T |

## Recursos de edição

- **Prévia ao passar o mouse:** passe o mouse numa mídia do Media Pool e o viewer mostra o vídeo naquela posição, como o hover scrub do Resolve. Clique para usar como Source (In/Out, Insert, Overwrite).
- **Cortes rápidos:** Shift+[ e Shift+] cortam do início até o cursor (ou do cursor até o fim) e fecham o buraco.
- **Copiar e colar** clipes com Ctrl+C/Ctrl+V. O áudio e o vídeo ligados vão juntos.
- **Marcadores:** M cria, Shift+↑/↓ pula entre eles, dois cliques na bandeirinha editam nota e cor.
- **Fades:** arraste os quadradinhos nos cantos de cima do clipe.
- **Cross Dissolve:** Ctrl+T no corte mais perto do cursor, ou arraste da Toolbox até um corte. O áudio ganha um crossfade.
- **Keyframes:** o botão ◆ no Inspector anima posição, zoom, opacidade, volume e cor. Os losangos aparecem no clipe selecionado.
- **Cor no estilo Ctrl+U do Photoshop:** matiz, saturação e luminosidade por clipe.
- **Recortar e espelhar** a imagem no Inspector.
- **Títulos:** negrito, alinhamento, contorno, sombra e caixa de fundo. Há presets de Título, Lower third e Legenda.
- **Áudio:**
  - medidores L/R no viewer;
  - "Normalizar" (pico em −1 dB);
  - "Remover silêncios", que corta as pausas da fala e fecha os buracos, com o vídeo junto.
- **Exportação:**
  - presets YouTube 1080p e 4K (H.265), 720p rápido, e só áudio em MP3 ou WAV;
  - exportar só o trecho In/Out;
  - salvar o frame atual em PNG.
- **Mídia otimizada (proxy):** clique direito na mídia, para vídeos 4K de celular rodarem lisos no preview. A exportação usa sempre o original.
- **Configurações do projeto:** mude para vertical 9:16, quadrado ou 4K. Os projetos recentes ficam no menu Arquivo e na tela vazia do Media Pool.

## Onde ficam os arquivos

- Atalhos e layout: `~/.config/pawcut/`
- Autosave: `~/.local/share/pawcut/autosave/`
- Cache de áudio decodificado: `~/.cache/pawcut/` (pode apagar à vontade)

## Desenvolvimento

```bash
cargo test --workspace        # 140+ testes, incluindo cliques e arrastos simulados na interface
cargo clippy --workspace --all-targets -- -D warnings
```

Os crates são:
- `pawcut-core`: modelo, edições, desfazer/refazer e arquivo `.pawcut`.
- `paw-media`: lê arquivos com ffmpeg.
- `pawcut-render`: compositor e mixer.
- `pawcut-export`: MP4.
- `pawcut-app`: a interface.

Exemplos de linha de comando: `cargo run -p pawcut-export --example render_project -- projeto.pawcut saida.mp4`.
