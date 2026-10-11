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
