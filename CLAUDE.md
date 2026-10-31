# Nemux

App desktop (Windows) para usar vários terminais em abas, com uma memória
persistente em SQLite que as sessões do Claude Code compartilham via MCP.

## Estrutura (workspace Cargo)

- `memory/` (`nemux-memory`): banco SQLite com busca FTS5. Banco padrão em
  `%APPDATA%\nemux\nemux.db`, trocável pela variável `NEMUX_DB`.
- `mcp-server/` (`nemux-mcp`): servidor MCP via stdio (crate `rmcp`) com as
  ferramentas `memory_save`, `memory_search`, `memory_recent`, `memory_delete`.
  Ao iniciar dentro de um painel do Nemux (`NEMUX_PANE` no ambiente), registra
  na tabela `claude_panes` a sessão do Claude (`CLAUDE_CODE_SESSION_ID`, que o
  Claude Code passa aos servidores MCP), a pasta e o próprio PID; ao terminar,
  apaga o registro.
  `nemux-mcp statusline [banco]` (`src/statusline.rs`) é o comando do `statusLine`
  do Claude Code: lê o JSON do stdin (contexto e `rate_limits` de 5 h e 7 dias),
  grava em `claude_usage` e devolve a linha da barra do Claude. O Claude o roda
  por um shell (`claude` → `sh` → comando): o PID gravado é o do ancestral chamado
  `claude`/`node`, e o app só mostra o contexto enquanto ele vive (o
  `claude_panes` não serve: depende do MCP registrado e o `/clear` troca a sessão).
  `save_usage` não grava se nada mudou, senão cada atualização da tela do Claude
  faria o app recarregar. O JSON é interno do Claude Code e pode mudar.
- `app/` (`nemux-app`): interface Slint.
  - `src/app.rs`: estado do app (`App`): abas, painéis (cada um com seu
    `Terminal`), seleção, área de transferência, atalhos e `render()`.
  - `src/layout.rs`: árvore de painéis de uma aba (`Node`), retângulo de cada
    painel e vizinho numa direção. O Slint não tem componentes recursivos, então
    a interface recebe só a lista de painéis já posicionados (`PaneView`).
  - `src/terminal.rs`: PTY (`portable-pty`, ConPTY) + parser `vt100`; `snapshot()`
    transforma a tela em trechos (`Run`) com o mesmo estilo.
  - `src/keys.rs`: teclas do Slint → sequências de terminal.
  - `src/selection.rs`: seleção de texto com o mouse. As linhas contam do topo do
    histórico (como na busca), para a seleção seguir o texto quando a tela rola;
    `text` lê o histórico uma tela por vez (`Terminal::with_screen_mut`). Arrastar
    para fora do painel rola o histórico: um timer de 50 ms (`main.rs`, ligado no
    `select-start`) chama `App::autoscroll`, porque o ponteiro parado não gera
    eventos. Triplo clique = clique até 500 ms depois do duplo, na mesma linha
    (`App::last_word`); o Slint só tem `double-clicked`.
  - `src/search.rs`: busca no histórico (`Ctrl+Shift+F`, `ui/search-bar.slint`).
    O `vt100` só expõe as linhas à vista, então `find_all` percorre o histórico
    mudando a posição de rolagem e a devolve no fim. As ocorrências são contadas a
    partir do topo do histórico (estável com saída nova, até o histórico encher).
    `App::search` guarda o texto e a ocorrência escolhida; o `render()` só destaca
    as que estão à vista no painel ativo (`PaneView.matches`/`current-match`).
  - `src/memory_view.rs`: painel de memória (lê o mesmo banco do servidor MCP).
  - `src/sessions.rs`: lê as sessões do Claude Code em
    `~/.claude/projects/<pasta>/<id>.jsonl` (ou `CLAUDE_CONFIG_DIR`). Usa só o
    começo do arquivo (`cwd`, primeira mensagem) e o fim (`ai-title`,
    `last-prompt`); os arquivos chegam a vários MB. Esse formato é interno do
    Claude Code e pode mudar: se a lista vier vazia ou sem título, comece por aqui.
  - `src/usage_view.rs`: uso do Claude (o que o `nemux-mcp statusline` grava):
    limites na barra de abas (`usage-meters`, detalhe em `usage-detail`) e o
    contexto de cada painel (`App::contexts`, pelo `NEMUX_PANE`, em
    `PaneView.context`). Recarrega quando o `data_version` muda e a cada 30 s.
  - `src/claude_settings.rs`: "Uso do Claude na barra" (engrenagem) põe ou tira o
    `statusLine` no `settings.json` do Claude (`serde_json` com `preserve_order`,
    para não mexer na ordem). Não troca uma barra que não é do Nemux. Só aparece
    com `Controllers::use_claude_settings`, que os testes não chamam.
  - `src/session_view.rs`: lista "Retomar sessão" (`ui/session-picker.slint`);
    retomar abre `powershell -NoExit -Command "claude --resume <id>"` na pasta da sessão.
  - `src/config.rs`: `config.toml` do usuário (`%APPDATA%\nemux\config.toml` ou
    `NEMUX_CONFIG`): shell, fonte e cores. Fonte e cores da interface vão para o
    global `Theme` (por isso as propriedades dele são `in-out`); as do terminal
    vão para `terminal::Colors`, usado no `snapshot()`. Lido só ao abrir; um
    arquivo com erro vira a configuração padrão e um aviso (`config-error`).
    As cores partem de um tema pronto (`src/themes.toml`, embutido; o primeiro é
    o padrão e tem de bater com `Colors::default()`), e `[colors]`/`[app]` ficam
    por cima (`Scheme`). O menu da engrenagem (`ui/settings-menu.slint`) muda na
    hora tema (`ui/theme-picker.slint`), fonte (`Ctrl+=`/`-`/`0`) e `restore`, e
    grava no arquivo com `toml_edit` (`config::edit_file`), sem mexer no resto. A
    lista de atalhos fica em `ShortcutsCard`: mantenha-a igual à do README.
    "Abrir config.toml" usa `Controllers::opener` (os testes trocam).
  - `src/workspace.rs`: abas entre execuções (`%APPDATA%\nemux\session.json`):
    árvore de painéis, pasta de cada um (a de quando abriu, ou a que o shell
    informou por OSC 7/9;9) e a sessão do Claude dele. Grava ao fechar a janela
    (antes de encerrar os terminais) e a cada 30 s; `restore = false` desliga.
    A sessão de um painel é a mais antiga em `claude_panes` cujo `nemux-mcp`
    ainda roda: um `claude -p` rodado de dentro do Claude herda o `NEMUX_PANE`
    e também se registra. Nos testes, `App::resume` troca o `claude --resume`.
  - `src/main.rs`: `init_controllers`, que liga os callbacks da interface ao `App`.
  - `ui/app-window.slint` (janela, abas, área dos painéis), `ui/terminal-pane.slint`
    (um painel: grade, seleção, cursor, mouse), `ui/memory-panel.slint`,
    `ui/theme.slint` (cores e fonte).
  - `ui/icons/nemux.svg` (e `nemux-small.svg`, para 16 e 24 px): o ícone. É a
    única fonte: a janela usa o SVG direto, e o `build.rs` gera o `.ico` e o
    embute no `.exe` (precisa do `rc.exe` do Windows SDK; sem ele o build só avisa).

## Comandos

```bash
cargo run -p nemux-app                    # abre o app
cargo build --release -p nemux-mcp        # servidor MCP usado pelo Claude Code
cargo fmt --all
cargo clippy --workspace --all-targets    # deve terminar sem avisos
```

## Testes

```bash
cargo test-model   # só a lógica (memória, MCP, terminal, teclas, seleção)
cargo test-ui      # só a interface (app/src/ui_tests)
cargo capturas     # só os testes #[ignore] que geram PNGs em target/ui-capturas
cargo test --workspace   # tudo, antes do commit
```

Os atalhos ficam em `.cargo/config.toml`.

Testes de UI ficam em `app/src/ui_tests/`, um arquivo por tela, e rodam sem
janela (backend de testes do Slint + renderizador por software):

- `i-slint-backend-testing` precisa ter **exatamente** a versão do `slint` no
  `Cargo.lock`; ao atualizar um, atualize o outro.
- Elementos são achados pelo id qualificado pelo componente onde foram
  declarados (`"AppWindow::term-area"`). Os ids só existem em debug (`build.rs`).
- Botões próprios (Rectangle + TouchArea) precisam de `accessible-role`,
  `accessible-label` e `accessible-action-default` para serem clicados nos testes;
  os testes os acham pelo rótulo.
- Testes que só geram capturas levam `#[ignore]` e não têm asserções; testes com
  asserções nunca levam `#[ignore]`.
- **Nunca use o banco de memória padrão nos testes** (é o do usuário): use
  `abrir_com_controllers(caminho)` com um banco temporário. O mesmo vale para o
  `config.toml`: `abrir_com_controllers` não lê nenhum, e os testes de
  configuração (`ui_tests/config.rs`) escrevem um arquivo temporário.
- Os testes de seleção rodam um shell de verdade (`cmd.exe` no ConPTY, `sh` no Linux).
- No backend de testes, uma linha do terminal atualizada várias vezes (digitar
  num painel) pode ficar com o texto antigo na árvore do Slint, embora o modelo
  (`PaneView.lines`) esteja certo; no app real não acontece. As asserções leem o
  modelo e não são afetadas; o `capturar` regrava cada linha antes da foto.

## Cuidados

- O stdout do `nemux-mcp` é o canal do protocolo MCP: logs vão para o stderr.
- O servidor está registrado no Claude Code (escopo de usuário) apontando para
  `target\release\nemux-mcp.exe`. O Windows trava o `.exe` enquanto alguma sessão
  do Claude o usa: feche-as antes de recompilar em release.
- **ConPTY novo em `vendor/conpty`** (`conpty.dll` + `OpenConsole.exe` da Microsoft,
  MIT; ver o README de lá), **embutido no executável** (`src/conpty.rs`): no
  primeiro terminal ele é extraído para `%LOCALAPPDATA%\nemux\conpty\<versão>` e
  carregado pelo caminho completo; o `portable-pty`, que pede `conpty.dll` pelo
  nome, recebe essa DLL já carregada. O do Windows 10 redesenha a tela em vez de
  repassar a saída: ele esconde que o Claude Code usa tela alternativa e mouse
  (`?1049h`, `?1000h`, `?1006h`), então a roda não chegava ao Claude. Antes ele
  era copiado para junto do `.exe`, e um `.exe` copiado sozinho para outra pasta
  perdia a correção. `NEMUX_SYSTEM_CONPTY=1` usa o do Windows (o teste
  `ui_tests::mouse::roda...` falha com ele, e isso é esperado).
- Avisos: `Event::Attention` vem do BEL e dos OSC 9 (com texto; `OSC 9;<número>`
  são comandos do ConEmu, como 9;4 progresso e 9;9 pasta), 99 e 777;notify.
  O Claude Code no modo `auto` não manda nada a terminais que não conhece (só
  iTerm2, kitty, Ghostty e o Terminal do macOS): o usuário precisa de
  `"preferredNotifChannel": "terminal_bell"` (ou `iterm2`) no
  `~/.claude/settings.json`. Um pedido marca o painel se ninguém o está olhando (`App::watched`: foco + janela na frente,
  `window::is_foreground`) e chama `App::on_attention`, que pisca a barra de
  tarefas (`window::flash`, `FlashWindowEx`). `render()` limpa o que está à vista.
  Saída nos 3 primeiros segundos de um painel não marca a aba (abas restauradas).
- A roda do mouse (`App::wheel`) vai para o programa se ele ligou o mouse; na
  tela alternativa sem mouse vira setas; senão rola o histórico do Nemux.
- O ConPTY pergunta a posição do cursor (`ESC[6n`) ao iniciar e espera a
  resposta; `terminal::Callbacks` responde a isso (e a DSR 5 e DA1). Respostas
  ao ConPTY precisam ir num único `write`: em pedaços, ele não as reconhece.
- No `FocusScope` do terminal, `focus-on-click: false` é necessário: com o padrão
  (`true`) ele engole os cliques e a seleção com o mouse para de funcionar.
- O banco usa WAL e `busy_timeout`, porque várias sessões do Claude e o app
  gravam no mesmo arquivo. O painel recarrega quando `PRAGMA data_version` muda.
- No Windows, AltGr chega como Ctrl+Alt: `keys::encode` só trata como Ctrl
  quando a tecla é uma letra.
- Shift+Enter envia `ESC CR`. Testado no ConPTY do Windows 10: `CSI 13;2u`,
  modifyOtherKeys e win32-input-mode são descartados antes de chegar ao programa;
  `ESC CR` chega ao Node como `\x1b\r` (o Claude Code quebra a linha) e ao
  PowerShell como Alt+Enter (não executa nada).
- Os testes de UI que precisam de terminal usam `abrir_com_cmd` (em
  `ui_tests/mod.rs`): `cmd.exe` em vez do PowerShell, inclusive nos painéis novos;
  no Linux, `sh`. Rodam nos dois sistemas: o que muda entre os shells fica nos
  helpers de lá (`cmd`, `cmd_linhas`, `cmd_que_termina`, `PROMPT`, `PROMPT_FIM`);
  não escreva comandos do cmd direto num teste.
- Arquivos soltos na janela: o Slint não repassa o arrastar e soltar de fora, então
  `watch_file_drops` (`main.rs`) pega `HoveredFile`/`DroppedFile` direto do winit
  (`on_winit_window_event`, feature `unstable-winit-030`; por isso o `slint` está
  com `~1.18.1`: essa API pode mudar numa versão menor). O Windows manda um evento
  por arquivo e sem a posição do ponteiro: os arquivos são juntados num timer de 0 s
  e o painel é o que está sob `window::cursor_position` (`GetCursorPos`). Nos testes
  não há winit: eles chamam `Controllers::drop_files`.
- Abas (`TabButton`): a aba é escolhida ao **apertar** o botão, não ao soltar, para
  o arraste já começar com ela ativa. O arraste calcula a aba sob o ponteiro
  dividindo a posição pela largura da aba: todas têm a mesma largura (mesmas
  restrições, sem espaço entre elas); se isso mudar, o cálculo precisa mudar.
  `Tab::name` (duplo clique) fica no lugar do título do programa e vai para o
  `session.json`.
- Linux: roda (testado no Linux Mint; `cargo test --workspace` passa). Ainda não
  conferidos lá: arrastar arquivos, `window::flash`/`is_foreground` e a roda do
  mouse no Claude Code. O que muda fica atrás de
  `cfg`: shell (`app::default_shell`, `$SHELL`), `TERM` (`app::terminal_env`),
  retomar sessão (`sessions::resume_shell`), abrir arquivo (`xdg-open`), pasta do
  painel (`Terminal::cwd`, pelo `/proc`) e processo vivo (`workspace::process_alive`).
  A fonte padrão no Linux vem do `fc-match monospace` (`config::default_font_family`):
  sem a Consolas o Slint usaria uma proporcional, e ele não entende "monospace".
  No Linux a área dos painéis mede 0 até a janela aparecer: `restore_or_open_tab`
  espera o `area-resized` (`App::sized`), senão o shell nasce com 1 coluna e o que
  ele escreve primeiro (um banner do `.bashrc`) fica quebrado.
  Os testes de UI rodam no Linux também (com `sh`); só os que rodam PowerShell
  (fora dos `ui_tests`) e o ConPTY são do Windows. Para
  conferir que ainda compila sem uma máquina Linux (sem compilador C para Linux, o
  SQLite e o fontconfig vêm "do sistema"; o `cargo check` não linka):
  `LIBSQLITE3_SYS_USE_PKG_CONFIG=1 PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG=<um .bat que
  só faz echo -lfontconfig> cargo clippy --workspace --all-targets --target
  x86_64-unknown-linux-gnu`. Apague `target/x86_64-unknown-linux-gnu` depois.
- O Slint não formata `.slint` com `cargo fmt`: mantenha a indentação à mão.
