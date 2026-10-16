# Filosofia de design da PawSuite

Este documento vale para todos os apps da suíte (PawCut, PawVoice, PawWrite, PawPaint e os
próximos). Quando um app novo nascer, ou uma tela nova for desenhada, a pergunta é sempre:
"isso parece e se comporta como o resto da suíte?". Se a resposta for não, o app errado é o
que está mudando, não a suíte.

O código que torna isso possível mora em `crates/paw-ui` (tema, ícones, avisos, backups,
helpers de layout) e `crates/paw-audio`. Regra de ouro: **se dois apps precisam da mesma coisa,
ela vai para um crate compartilhado**, nunca é copiada.

## 1. Princípios

1. **Ferramenta, não vitrine.** O app some e o trabalho aparece. Nada de animação decorativa,
   gradiente, sombra pesada ou cor que não signifique algo.
2. **Familiar antes de original.** Cada app imita o programa que ele substitui (PawCut é a aba
   Edit do Resolve, PawVoice é o Audacity, PawPaint é o Krita). Atalhos, nomes e lugares dos
   controles seguem o original. Quem já sabe usar o original não aprende nada de novo.
3. **Escuro e calmo.** Cinzas escuros de baixo contraste, um único laranja de destaque. O
   conteúdo do usuário (vídeo, onda, imagem, texto) é a coisa mais clara da tela.
4. **Previsível.** O mesmo gesto faz a mesma coisa em todos os apps. Nada muda de tamanho,
   de lugar ou de ordem sozinho.
5. **Nunca perder trabalho.** Autosave, recuperação depois de fechar de repente, backups das
   versões anteriores e desfazer em tudo.
6. **Rápido de verdade.** A interface nunca espera por disco, decodificação ou rede. Trabalho
   pesado vai para outra thread, a tela mostra o que já tem e atualiza quando chega.
7. **Em português, direto.** Texto de interface curto, no imperativo ou no substantivo
   ("Salvar", "Remover silêncios"), sem jargão que o original em inglês não use.

## 2. Visual

### Cores (`paw_ui::theme`)

Use sempre as constantes, nunca um `Color32::from_rgb` solto para cor de interface.

| Constante | Uso |
|---|---|
| `BG` | fundo dos painéis |
| `BG_DARK` | fundo de áreas de conteúdo e campos de texto |
| `BAR` / `BAR_LIGHT` | barras de ferramentas, abas, botões em repouso |
| `BORDER` | divisórias de 1 px (quase preto) |
| `TEXT` | texto normal |
| `TEXT_DIM` | rótulos e texto secundário (contraste mínimo de 4,5:1 sobre `BG` e `BAR`) |
| `ACCENT` | seleção, item ativo, foco, ação principal. **Uma cor só.** |
| `PLAYHEAD` | cursor de reprodução |
| `TIMECODE` | números de tempo |
| `OFFLINE` | mídia ou arquivo que sumiu |

Cores de tipo de conteúdo (`CLIP_VIDEO`, `CLIP_AUDIO`, `CLIP_TITLE`...) são dados, não
decoração: a mesma cor significa a mesma coisa em todo app.

Laranja (`ACCENT`) indica "isto está ativo ou selecionado". Vermelho indica erro ou perigo.
Nenhuma outra cor carrega significado.

### Tipografia

- Noto Sans (ou DejaVu Sans se faltar). Corpo e botões em 12,5 px, texto pequeno 10,5 px,
  títulos 16 px, tempo e números alinhados em monoespaçada.
- Rótulos de linha em `TEXT_DIM`, valores em `TEXT`. Títulos de seção em branco.
- Nunca use emoji como ícone: a fonte do sistema pode não ter. Use `paw_ui::icons` (vetoriais,
  desenhados com o painter) ou um símbolo comum (↺ ◆ ×).

### Grade

Tudo é múltiplo de 4 px.

| Constante | Valor | Uso |
|---|---|---|
| `TOOLBAR_H` | 28 | altura de toda barra de ferramentas |
| `ROW_H` | 24 | altura de toda linha de propriedade |
| `PAD` | 8 | margem interna padrão |
| `ICON` | 16 | tamanho de ícone em botão |
| `LABEL_W` | 104 | coluna dos rótulos no Inspector |
| `VALUE_W` | 84 | coluna dos valores |
| `NUM_W` | 64 | campo numérico solto (fora de uma linha de propriedades) |

Cantos com raio 3 (widgets) e 4 (janelas). Bordas de 1 px em `BORDER`. Espaço entre itens 8 x 4.

### Ícones e botões

- Botão de ícone: `paw_ui::icons::button(ui, icon, size, ligado, dica)`. **Todo** botão sem
  texto tem dica (tooltip), e a dica mostra o atalho: "Cortar no cursor (Ctrl+B)".
- Estado ligado = fundo mais claro + ícone em `ACCENT`. Desligado = `TEXT_DIM`. Passar o mouse
  clareia. Desativado não responde e fica apagado.

## 3. Estrutura da tela

- **Painéis acopláveis** (`egui_dock`): o usuário arruma, fecha, divide e o layout fica salvo.
  Sempre existe um jeito de reabrir um painel fechado (menu Workspace ou Configurações) e um
  "Resetar layout".
- **Barra de ferramentas** de cada painel: `paw_ui::theme::toolbar`. Mesma altura, mesma margem,
  alinhamento vertical central em todos.
- **Seções recolhíveis**: `theme::section`, com setinha. O estado aberto ou fechado é lembrado.
- **Propriedades em grade fixa**: `theme::row(rótulo, valor, extras)`. Rótulo alinhado à direita,
  valor, e até dois botões no fim (◆ keyframe, ↺ voltar ao padrão) **sempre nas mesmas colunas**.
  Se uma linha não usa o segundo botão, reserve o espaço (`ui.add_space(22.0)`) para o ↺ não
  andar para o lado.
- **Barra de estado** embaixo: informação que muda sempre (tempo, contagens, zoom), nunca
  botão perigoso.
- **Diálogos** são janelas pequenas, sem colapsar, com largura fixa. Confirmação de perigo usa
  `Modal` com os botões na ordem Salvar, Não salvar, Cancelar.
- **Avisos** vão para `paw_ui::toasts` (canto de baixo, somem sozinhos, erro em vermelho e fica
  mais tempo). Diálogo bloqueante só para o que exige decisão.

## 4. Campos de entrada (a regra que mais quebra)

Um campo **nunca** pode mudar de tamanho enquanto o usuário usa ele, nem empurrar o que está
em volta, nem vazar para fora da linha.

1. **Largura fixa.** O `DragValue` do egui cresce e encolhe com os dígitos e o sufixo. Dentro
   de `theme::row` isso já é resolvido (`VALUE_W`). Fora dela, embrulhe:
   ```rust
   theme::fixed_width(ui, theme::NUM_W, |ui| {
       ui.add(DragValue::new(&mut w).range(16..=8192));
   });
   ```
2. **Nunca passe de uma largura fixa maior que o espaço.** Para `TextEdit` e `ComboBox`, use
   `theme::fit_width(ui, 140.0)`: ele usa a largura desejada, mas encolhe em painel estreito.
3. **Altura fixa em texto de várias linhas.** Um `TextEdit::multiline` cresce a cada linha e
   empurra o painel. Ponha em um `ScrollArea` com `max_height`.
4. **Velocidade do arrasto em unidades da tela.** `DragValue::speed` é "quanto muda por pixel",
   na unidade que o usuário vê (%, px, °, dB). Para porcentagem, 0,5 por pixel. Nunca
   multiplique pela escala interna.
5. **Sempre com `range`.** Todo campo numérico tem mínimo e máximo, e o modelo (core) valida de
   novo: a interface ajuda, o core protege.
6. **Digitar e arrastar dão o mesmo resultado**, com sufixo (`" %"`, `" dB"`, `" fr"`) e o mesmo
   número de casas decimais.
7. **Um passo de desfazer por gesto.** Arrastar um valor aplica a edição com
   `apply_coalescing(chave)`; soltar o mouse, Enter ou Tab fecham o passo.
8. **Campo de texto foca = atalhos calam.** Com um campo de texto em foco, nenhum atalho de
   tecla simples (Espaço, letras, setas) pode disparar. O app consulta `ctx.text_edit_focused()`.
9. **Valor misto.** Com vários itens selecionados e valores diferentes, mostre o do primeiro e
   a etiqueta "vários". Mudar o valor muda todos.
10. **Enter confirma, Esc cancela**, em todo campo que valida o que foi digitado (timecode,
    duração, nome).

## 5. Interação

- **Atalhos editáveis** e iguais entre apps: Espaço toca e pausa, Ctrl+Z / Ctrl+Shift+Z,
  Ctrl+S / Ctrl+Shift+S, Ctrl+N, Ctrl+O, Ctrl+C / Ctrl+X / Ctrl+V, Delete, Ctrl+roda dá zoom, a
  roda rola. Um atalho novo nunca rouba o de outro app; se o original (Resolve, Audacity, Krita)
  usa uma tecla, a nossa usa a mesma.
- **Paleta de comandos** (Ctrl+K) lista toda ação com o atalho ao lado.
- **Botão direito** abre o que se faz com aquilo (menu de contexto), nunca é o único caminho.
- **Arrastar e soltar** arquivos na janela importa/abre.
- **Cursor do mouse** diz o que vai acontecer (redimensionar, texto, mão).
- **Feedback imediato.** Todo clique tem resposta na mesma tela (< 100 ms). Se a operação é
  mais lenta, mostre progresso e permita cancelar.

## 6. Áudio

Regras para qualquer app que toque ou grave som (`paw-audio` e o player do PawCut):

1. **Editar durante o play não reinicia o som.** Trocar o projeto/parâmetros muda só o que vem
   a seguir. Nunca esvazie a fila da placa (`flush`) por causa de uma edição: isso repete
   trechos e picota o áudio a cada ajuste (era o bug do volume no PawCut). `flush` só em
   play, seek, scrub e stop.
2. **Sem degrau.** Quando um parâmetro muda no meio da reprodução, passe do som antigo para o
   novo com um crossfade curto (um bloco de ~20 ms), senão dá estalo.
3. **Buffer curto.** ~60 a 100 ms de fila na placa (`BufferAttr`), para que o que o usuário
   fez seja ouvido quase na hora. O padrão do PulseAudio (~250 ms) parece lento.
4. **O áudio é o relógio.** O cursor de reprodução vem da posição que já saiu da caixa de som
   (posição escrita menos a latência), não de um cronômetro.
5. **A thread de áudio nunca espera pela interface.** Estado compartilhado por atômicos ou
   `Mutex` com trava curta, e a decodificação pesada é feita antes (`prepare`).
6. **Medidores** sobem na hora e descem devagar (x0,85 por bloco). Pico em 0..1, escala em dB.
7. **Loudness** em LUFS (EBU R128), alvo de −14 para YouTube, e nunca passa de −1 dB de pico.

## 7. Arquivos e segurança do trabalho

- Um formato de projeto por app, em texto (JSON) ou zip simples, com `version` para migrar.
- **Autosave** periódico em `~/.local/share/<app>/autosave/` e oferta de **recuperar** ao
  abrir depois de um fechamento brusco.
- **Backups**: ao salvar por cima, a versão anterior vai para `paw_ui::backups` (10 versões).
- Configuração em `~/.config/<app>/` (respeitando `XDG_CONFIG_HOME`), cache em
  `~/.cache/<app>/` (pode apagar à vontade).
- **Fechar com alterações** pergunta (Salvar, Não salvar, Cancelar). Fechar durante uma
  exportação pergunta também.
- Arquivo que sumiu aparece como offline (vermelho, `OFFLINE`) com Reconectar, nunca derruba o app.
- Erros de arquivo e de mídia viram aviso legível, nunca `panic`. `unwrap` em dado do usuário
  é bug.

## 8. Desempenho

- A thread da interface só desenha e reage. Decodificar, renderizar, calcular picos,
  exportar e analisar vão para threads com pedidos que se substituem (só o mais novo importa).
- Caches com limite (LRU) e chaves que incluem tudo que muda o resultado (arquivo, velocidade,
  tamanho).
- Estruturas imutáveis compartilhadas (`Arc<Project>`) entre interface, preview e áudio, em vez
  de copiar.
- Pedir repaint só quando algo muda ou algo está tocando.
- Medir antes de otimizar, e deixar um teste ou exemplo (`examples/`) que reproduz o gargalo.

## 9. Qualidade

- `cargo clippy --workspace --all-targets -- -D warnings` e `cargo fmt` limpos antes de commitar.
- Lógica de edição no `*-core`, sem egui, testada por unidade e por propriedade.
- A interface tem testes com cliques e arrastos simulados (`egui_kittest`): um bug de
  interface corrigido ganha um teste.
- Mensagem de commit: `tipo(app): o que mudou` (`feat`, `fix`, `perf`, `polish`).

## 10. Checklist para uma tela nova

- [ ] Só usa cores e medidas de `paw_ui::theme`.
- [ ] Barras e linhas usam `toolbar`, `section` e `row`.
- [ ] Todo botão de ícone tem dica com o atalho.
- [ ] Todo campo tem largura fixa (ou `fit_width`), `range` e sufixo.
- [ ] Nenhum campo muda de tamanho ao arrastar ou digitar, nem em painel estreito.
- [ ] Com um campo de texto em foco, os atalhos não disparam.
- [ ] Um gesto = um passo de desfazer.
- [ ] Trabalho pesado fora da thread da interface.
- [ ] Funciona com o painel pequeno e com o painel maximizado.
- [ ] Texto em português, curto, sem traço longo.
