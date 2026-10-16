# PawCut: editor de vídeo simples em Rust

## Contexto

O Luis quer um editor de vídeo **para uso pessoal** no Linux (Ubuntu 26.04), em **Rust**, para edições simples sem o peso do DaVinci/Premiere. A interface deve se parecer com a **aba Edit do DaVinci Resolve**, que ele já usa no dia a dia, com **painéis acopláveis** e **atalhos editáveis dentro do app**. A pasta `/home/luis/Documents/PawCut` está vazia e ainda não é um repositório git.

**Entra:** cortar/aparar/mover, várias camadas de vídeo e áudio, overlays (posição, escala e opacidade fixas), geradores (Solid Color e Text), imagens PNG/JPG, fade in/out, velocidade do clipe, volume por clipe, separar áudio do vídeo, waveform na timeline, exportar MP4 (H.264/AAC), salvar e abrir projeto `.pawcut`.

**Fica de fora:** keyframes, correção de cor, efeitos, transições entre clipes, vários formatos de exportação, plugins.

**Critério de sucesso:** importar um vídeo, cortar, colocar um logo PNG e um texto por cima, adicionar uma música numa faixa de áudio, exportar, e o MP4 sair igual ao que o Timeline Viewer mostrava.

## Ambiente

- Já instalados: Node 24, Python 3, ffmpeg 8.0.1 com as libs de desenvolvimento (`libavcodec 62`, `libavformat 62`, `libswscale 9`, `libswresample 6`) e libclang 21 (necessária para o bindgen do `ffmpeg-next`).
- **Falta o Rust.** O próprio Luis instala com `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`.
- Na implementação, confirmar no crates.io a versão do `ffmpeg-next` compatível com o FFmpeg 8. Se os bindings não compilarem, o plano B é decodificar chamando o executável `ffmpeg` com `rawvideo`/`f32le` via pipe.

## Arquitetura (workspace Cargo)

Um **compositor único** gera o preview e a exportação, e é isso que garante WYSIWYG. A exportação só pede frames RGBA e áudio PCM ao compositor e manda tudo por pipe para um processo `ffmpeg`.

| Crate | Responsabilidade | Depende de |
|---|---|---|
| `crates/pawcut-core` | Modelo, operações de edição (`Command`), desfazer/refazer, serialização `.pawcut` (JSON com `version`) | `serde`, `serde_json` |
| `crates/paw-media` | Ler metadados, decodificar vídeo com seek e cache LRU, carregar imagens, decodificar áudio para PCM 48 kHz estéreo f32 (cache em `~/.cache/pawcut/`), áudio com velocidade via `atempo` (libavfilter), picos da waveform, miniaturas | `ffmpeg-next`, `image`, `lru` |
| `crates/pawcut-render` | `render_frame(project, frame, resolution) -> RgbaImage` (camadas de baixo para cima, transform, opacidade, fade, cor sólida, texto) e `mix_audio(project, sample_range) -> Vec<f32>` (volume em dB, fades, mute/solo) | core, media, `rayon`, `fast_image_resize`, `cosmic-text` |
| `crates/pawcut-export` | Laço de exportação, pipe para `ffmpeg` (`libx264`/`aac`, `-pix_fmt yuv420p`), progresso, cancelamento, log de erro | render |
| `crates/pawcut-app` | Interface `eframe`/`egui` + `egui_dock`, estado central do app, reprodução (`cpal`), keymap, preferências, autosave | todos |

Os painéis nunca conversam entre si. Todos leem e alteram o `AppState` (projeto, seleção, cursor, estado de reprodução), e as mudanças no projeto passam sempre por um `Command`.

## Modelo de dados (`pawcut-core`)

- O tempo é guardado em **frames inteiros do projeto** (`i64`). Resolução e FPS são do projeto (padrão 1920×1080 a 30 fps). O áudio converte para amostras só quando é usado.
- `Project { version, settings, media: Vec<MediaRef>, video_tracks, audio_tracks }`
- `Track { id, name, locked, enabled (vídeo) | muted/solo (áudio), clips }`. Clipes não se sobrepõem na mesma faixa.
- `Clip { id, source, start, in_point, duration, speed, volume_db, fade_in, fade_out, transform{x,y,scale,opacity}, link_id: Option<LinkId> }`
- `ClipSource = Media(MediaId) | SolidColor(Rgba) | Text { text, font, size, color }`
- Ao importar um vídeo com áudio, são criados dois clipes (V e A) com o mesmo `link_id`. Unlink remove a ligação.
- Operações, todas como `Command` reversível: `Overwrite`, `Insert` (com ripple), `Append`, `Move`, `TrimStart/End`, `BladeAt`, `Delete`, `RippleDelete`, `SetProperty`, `Link/Unlink`, `SetSpeed` (recalcula a duração), `AddTrack/RemoveTrack`. Quando um clipe ligado é editado, o par recebe a mesma edição se Linked Selection estiver ligado.

## Interface estilo Resolve (`pawcut-app`)

Layout padrão no `egui_dock`, igual à aba Edit do Resolve:
- À esquerda, **Media Pool** (miniaturas) e, embaixo dele, **Effects → Toolbox** (Titles: Text; Generators: Solid Color).
- No centro, **Source Viewer** (marcas I/O) e **Timeline Viewer** lado a lado. Arrastar dentro do Timeline Viewer move o clipe selecionado.
- À direita, **Inspector** com as seções Video (Transform, Speed, Fade) e Audio (Volume, Fade).
- Embaixo, **Timeline**: barra de ferramentas (Selection `A`, Blade `B`, Snapping `N`, Linked Selection), cabeçalhos das faixas (lock, liga/desliga em V, Mute/Solo em A), régua `HH:MM:SS:FF`, waveform nos clipes de áudio, "Media Offline" em vermelho.
- Rodapé com as abas **Edit** e **Deliver** (exportar: arquivo, resolução e FPS do projeto, qualidade CRF, barra de progresso, cancelar).
- Menu **Workspace**: liga e desliga painéis, "Reset UI Layout". O layout fica salvo em `~/.config/pawcut/layout.json`.

**Atalhos:**
- Enum `Action` central, e o keymap mapeia `KeyChord -> Action`. O padrão imita o Resolve: `Space`, `J/K/L`, `I/O`, `←/→` (andar um frame), `↑/↓` (corte anterior/seguinte), `Ctrl+B` (cortar no cursor), `A`/`B` (ferramentas), `N`, `Ctrl+Shift+L`, `Backspace`/`Del` (apagar), `Shift+Backspace` (ripple delete), `F9`/`F10`/`Shift+F12` (insert/overwrite/append), `Ctrl+Z`/`Ctrl+Shift+Z`, `Ctrl+S`, `Ctrl+O`, `Ctrl+I` (importar), `Ctrl+roda` (zoom).
- A tela **Preferences → Keyboard** lista as ações. Você clica numa, pressiona a nova tecla, o app avisa sobre conflito e há a opção "Restaurar padrão". Tudo salvo em `~/.config/pawcut/keymap.json`.

## Reprodução

- **O áudio é o relógio mestre.** O callback do `cpal` chama `mix_audio`, e o cursor vem das amostras já tocadas.
- Uma thread de render recebe o frame alvo e devolve a textura mais recente. Tocando, a resolução cai para 1/2 e há prefetch. Pausado, a resolução é cheia. Se o vídeo atrasar, descarta frames, e o áudio nunca é bloqueado.
- Decodificadores por mídia com cache LRU: leitura sequencial na reprodução, seek no keyframe mais próximo seguido de decode até o alvo no scrub.

## Erros e robustez

- Mídia faltando: clipe "Media Offline" e diálogo **Relink**.
- Arquivo não suportado: aviso (toast) e o app segue.
- Exportação: em caso de falha, mostra o fim do stderr do ffmpeg. Cancelar apaga o arquivo parcial.
- Autosave a cada 5 minutos em `~/.local/share/pawcut/autosave/`, com oferta de recuperação na abertura.
- Toda leitura e gravação de config/projeto usa `Result` com mensagens claras. Sem `unwrap` em caminho de I/O.

## Etapas de implementação

0. **Preparação:** Rust instalado pelo Luis, `git init`, workspace com os 5 crates, `.gitignore`, `cargo test` rodando vazio.
1. **core (TDD):** modelo, todos os `Command`s, desfazer/refazer, serialização. Cada operação começa por um teste.
2. **media:** metadados, decode de vídeo com seek e cache, imagens, cache de PCM, `atempo`, picos da waveform, miniaturas. As fixtures são geradas com `ffmpeg -f lavfi testsrc/sine` no `build`/setup dos testes.
3. **render:** compositor de frames e mixer de áudio, com testes de pixel e de amostra.
4. **Esqueleto do app:** eframe + egui_dock com os painéis vazios, `AppState`, importar para o Media Pool, Timeline Viewer mostrando um frame parado.
5. **Timeline:** desenho das faixas e clipes, selecionar/mover/aparar/snap, Blade, Linked Selection, waveform, cabeçalhos das faixas.
6. **Reprodução:** relógio do `cpal`, thread de render, J/K/L, scrub na régua.
7. **Source Viewer** (I/O, Insert/Overwrite/Append), **Inspector**, geradores Text/Solid arrastáveis da Toolbox.
8. **Keymap** com registro de `Action`s, persistência e tela Preferences → Keyboard. Persistência do layout e menu Workspace.
9. **Deliver** (exportar), salvar e abrir `.pawcut`, autosave/recuperação, Media Offline/Relink.

## Depois que este plano for aprovado

1. Salvar este design como spec em `docs/superpowers/specs/2026-10-11-pawcut-design.md` e fazer o commit (depois do `git init`).
2. Invocar o skill `superpowers:writing-plans` para quebrar as etapas acima em tarefas detalhadas, e só então implementar.

## Verificação

- `cargo test --workspace`: testes unitários do core, decode com fixtures geradas pelo ffmpeg, testes de pixel e áudio do render.
- Teste de integração de exportação: um projeto de 2 s com cor sólida + texto + `testsrc` + `sine`, exportado e conferido com `ffprobe` (1920×1080, 30 fps, duração de 2 s ±1 frame, streams h264 e aac).
- `cargo clippy --workspace -- -D warnings`.
- Roteiro manual com `cargo run -p pawcut-app`: importar um vídeo, cortar com `Ctrl+B`, fazer ripple delete, pôr um logo PNG em V2 com escala e opacidade, criar um Text em V3, separar o áudio e pôr uma música em A2, mudar a velocidade de um clipe para 2x, aplicar fade out no final, remapear um atalho, rearrumar painéis e reabrir o app (layout e atalhos devem persistir), salvar, fechar, reabrir o projeto e exportar. Comparar frames do MP4 (`ffmpeg -ss T -frames:v 1`) com o Timeline Viewer nos mesmos instantes.
