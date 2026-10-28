# Nemux

App desktop (Windows) para usar vários terminais em abas, com uma memória
persistente em SQLite que as sessões do Claude Code compartilham via MCP.

## Estrutura (workspace Cargo)

- `memory/` (`nemux-memory`): banco SQLite com busca FTS5. Banco padrão em
  `%APPDATA%\nemux\nemux.db`, trocável pela variável `NEMUX_DB`.
- `mcp-server/` (`nemux-mcp`): servidor MCP via stdio (crate `rmcp`) com as
  ferramentas `memory_save`, `memory_search`, `memory_recent`, `memory_delete`.
- `app/` (`nemux-app`): interface Slint.
  - `src/app.rs`: estado do app (`App`): abas, painéis (cada um com seu
    `Terminal`), seleção, área de transferência, atalhos e `render()`.
  - `src/layout.rs`: árvore de painéis de uma aba (`Node`), retângulo de cada
    painel e vizinho numa direção. O Slint não tem componentes recursivos, então
    a interface recebe só a lista de painéis já posicionados (`PaneView`).
  - `src/terminal.rs`: PTY (`portable-pty`, ConPTY) + parser `vt100`; `snapshot()`
    transforma a tela em trechos (`Run`) com o mesmo estilo.
  - `src/keys.rs`: teclas do Slint → sequências de terminal.
  - `src/selection.rs`: seleção de texto com o mouse.
  - `src/memory_view.rs`: painel de memória (lê o mesmo banco do servidor MCP).
  - `src/sessions.rs`: lê as sessões do Claude Code em
    `~/.claude/projects/<pasta>/<id>.jsonl` (ou `CLAUDE_CONFIG_DIR`). Usa só o
    começo do arquivo (`cwd`, primeira mensagem) e o fim (`ai-title`,
    `last-prompt`); os arquivos chegam a vários MB. Esse formato é interno do
    Claude Code e pode mudar: se a lista vier vazia ou sem título, comece por aqui.
  - `src/session_view.rs`: lista "Retomar sessão" (`ui/session-picker.slint`);
    retomar abre `powershell -NoExit -Command "claude --resume <id>"` na pasta da sessão.
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
  `abrir_com_controllers(caminho)` com um banco temporário.
- Os testes de seleção rodam um `cmd.exe` de verdade no ConPTY.

## Cuidados

- O stdout do `nemux-mcp` é o canal do protocolo MCP: logs vão para o stderr.
- O servidor está registrado no Claude Code (escopo de usuário) apontando para
  `target\release\nemux-mcp.exe`. O Windows trava o `.exe` enquanto alguma sessão
  do Claude o usa: feche-as antes de recompilar em release.
- O ConPTY pergunta a posição do cursor (`ESC[6n`) ao iniciar e espera a
  resposta; `terminal::Callbacks` responde a isso (e a DSR 5 e DA1).
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
  `ui_tests/mod.rs`): `cmd.exe` em vez do PowerShell, inclusive nos painéis novos.
- O Slint não formata `.slint` com `cargo fmt`: mantenha a indentação à mão.
