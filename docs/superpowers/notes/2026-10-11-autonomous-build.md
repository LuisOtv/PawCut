# Build autônoma (noite de 2026-10-11)

O Luis pediu para eu seguir sem pedir confirmações e entregar uma build funcional.
Aqui ficam as decisões que tomei no caminho, com o custo de cada uma se estiver errada.

## Ambiente
- **libclang sem headers:** `.cargo/config.toml` aponta `BINDGEN_EXTRA_CLANG_ARGS` para os headers do GCC 15. Depois de `sudo apt install libclang-dev` dá para apagar. Custo se estiver errado: o build quebra se o GCC mudar de versão.
- **Sem headers do ALSA:** o `cpal` não compila, então o áudio sai pelo PulseAudio (`libpulse-simple-binding`), que roda em cima do PipeWire. Custo: depende de um servidor Pulse/PipeWire, o padrão no Ubuntu.
- **`ffmpeg-next` sem as features padrão:** não usa o `libavdevice`, que não está instalado.

## Mídia
- **Conversão de áudio pelo grafo de filtros** (`aresample` + `aformat` + `atempo`) em vez do `swr`. O flush do `swr` perdia uns 0,24 s no fim.
- **Velocidade abaixo de 0,5x:** encadeia `atempo=0.5`. Cada `atempo` perde uns ms no fim (0,25x deu 189174 frames de 192000), e o teste aceita ±4000.
- **`swscale` não é `Send`:** fica num invólucro com `unsafe impl Send`. Cada `VideoSource` tem dono único e é acessado só por `&mut`.

## Render e exportação
- **Um compositor só**, na CPU com rayon, usado pelo preview e pelo export. A exportação manda RGBA por pipe para o `ffmpeg` CLI. O áudio é mixado antes, num arquivo temporário f32. No teste, 1080p saiu a ~37 fps.
- **Tamanho base das camadas:**
  - vídeo: "scale to fit", como no Resolve;
  - imagem e texto: tamanho nativo, reduzidos só se não couberem;
  - cor sólida: o quadro inteiro.
  - Custo: quem espera que a imagem preencha a tela precisa dar zoom no Inspector.
- **Eixo Y do transform aponta para cima**, como no Resolve.
- **Mídia faltando:** aparece como retângulo vermelho ("Media Offline") e a exportação para com o caminho do arquivo.

## App
- **Áudio pelo PulseAudio**, com a posição tocada servindo de relógio do preview. Só 1x para frente tem som. J, 2x/4x/8x e a reprodução do Source Viewer andam sem áudio, pelo relógio do sistema. Custo: shuttle mudo.
- **Insert empurra todas as faixas destravadas** (como no Resolve). In/Out da timeline existem, mas o Deliver exporta a timeline inteira.
- **Faixas não têm nome guardado**, o rótulo V1/A2 é calculado. O spec listava `name`. Custo: renomear faixas exigiria mudar o formato do `.pawcut`.
- **Ícones dos botões de faixa viraram letras** (L/V/M/S), porque os emoji ficavam ilegíveis no tamanho pequeno.
- **Bug achado pelos testes de interface:** a pegada do arrasto usava a posição depois do limiar de drag e passou a usar `press_origin`.
- **Ganchos de depuração** (`PAWCUT_SCREENSHOT`, `PAWCUT_ACTIONS`, `PAWCUT_PLAYHEAD`) permitem tirar screenshots automáticos da janela.
- **Preview em até 1920×1080:** a biblioteca de mídia do preview reduz os frames para caber nesse tamanho. Projetos 4K aparecem reduzidos no viewer, mas exportam em 4K.
