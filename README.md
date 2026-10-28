<p align="center">
  <img src="app/ui/icons/nemux.svg" width="128" height="128" alt="Ícone do Nemux">
</p>

<h1 align="center">Nemux</h1>

<p align="center">
  Terminais em abas com uma memória compartilhada para o Claude Code.
</p>

---

O Nemux é um app desktop para Windows, feito em Rust com [Slint](https://slint.dev),
para rodar vários terminais lado a lado e dar às sessões do
[Claude Code](https://claude.com/claude-code) uma **memória persistente em comum**.

O que o Claude aprende numa aba (uma decisão, uma preferência sua, um detalhe
do projeto) fica salvo num banco SQLite local e pode ser encontrado por qualquer
outra sessão, hoje ou amanhã.

```
Aba 1: claude ─┐
Aba 2: claude ─┼──► nemux-mcp ──► nemux.db (SQLite + busca FTS5)
Aba 3: claude ─┘                     ▲
                                     └── painel de memória do app
```

## Recursos

- **Terminais reais em abas**: cada aba roda um shell no ConPTY do Windows
  (PowerShell por padrão), com cores de 16, 256 e 24 bits, negrito, itálico,
  sublinhado, caracteres largos e 10 mil linhas de histórico.
- **Painéis divididos**: divida qualquer aba ao lado ou abaixo, quantas vezes
  quiser, e veja vários terminais (ou várias sessões do Claude) ao mesmo tempo.
- **Seleção e cópia de texto**: arraste para selecionar, dê duplo clique para
  pegar uma palavra ou um caminho de arquivo inteiro.
- **Shift+Enter** quebra a linha no prompt do Claude Code sem enviar.
- **Retomar sessões do Claude**: liste as conversas salvas do Claude Code (com
  título, pasta e quando foram usadas) e continue qualquer uma numa aba nova,
  inclusive as que foram abertas em outro terminal.
- **Memória compartilhada via MCP**: um servidor MCP que o Claude Code usa para
  salvar, buscar e apagar memórias.
- **Painel de memória**: veja, busque, filtre por projeto e apague o que o Claude
  salvou. A lista atualiza sozinha quando alguma sessão grava algo novo.

## Requisitos

- Windows 10 (1809) ou mais recente.
- [Rust](https://rustup.rs) 1.88 ou mais recente, com a toolchain MSVC.
- Para o ícone no `.exe`: o compilador de recursos `rc.exe`, que vem com o
  Windows SDK (instalado junto com as Build Tools do Visual Studio). Sem ele o
  app compila normalmente, só sem ícone no Explorer.
- Para a memória: o [Claude Code](https://claude.com/claude-code).

## Começando

```bash
git clone <url-do-repositório> nemux
cd nemux
cargo run --release -p nemux-app
```

A primeira compilação leva alguns minutos por causa do Slint.

### Ligando a memória ao Claude Code

Compile o servidor MCP e registre-o no Claude Code (vale para todos os seus
projetos):

```bash
cargo build --release -p nemux-mcp
claude mcp add --scope user nemux -- C:\caminho\para\nemux\target\release\nemux-mcp.exe
```

Abra uma sessão nova do Claude e confira com `/mcp`. A partir daí o Claude tem
as ferramentas:

| Ferramenta | O que faz |
|---|---|
| `memory_save` | Salva um fato associado a um projeto |
| `memory_search` | Busca por palavras (acentos são ignorados), com filtro opcional de projeto |
| `memory_recent` | Lista as memórias mais recentes de um projeto |
| `memory_delete` | Apaga uma memória errada ou desatualizada |

Para remover: `claude mcp remove nemux -s user`.

> **Dica:** o Windows trava o `nemux-mcp.exe` enquanto alguma sessão do Claude o
> usa. Feche essas sessões antes de recompilar o servidor.

## Atalhos

| Atalho | Ação |
|---|---|
| `Ctrl+Shift+T` | Nova aba |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Próxima / aba anterior |
| `Alt+Shift+=` | Dividir o painel ao lado (também pelo botão na barra de abas) |
| `Alt+Shift+-` | Dividir o painel abaixo |
| `Alt+←` `→` `↑` `↓` | Ir para o painel vizinho (com um painel só, vão para o programa) |
| `Ctrl+Shift+W` | Fechar o painel (o último painel fecha a aba) |
| `Shift+Enter` | Nova linha no prompt do Claude Code, sem enviar |
| `Ctrl+Shift+C` | Copiar a seleção |
| `Ctrl+C` | Copiar, se houver seleção; senão interrompe o programa |
| `Ctrl+V` / `Ctrl+Shift+V` | Colar |
| Botão direito | Copiar a seleção, ou colar se não houver seleção |
| `Shift+PageUp` / `Shift+PageDown` | Rolar o histórico (também com a roda do mouse ou a barra de rolagem de cada painel) |
| `Ctrl+Shift+M` | Abrir / fechar o painel de memória |
| `Ctrl+Shift+R` | Retomar uma sessão do Claude Code (também pelo botão "Sessões") |

### Trazendo uma sessão do Claude de outro terminal

Um programa já rodando em outra janela não pode ser movido para o Nemux (o
Windows não deixa trocar o console de um processo). Mas a conversa do Claude Code
fica salva em disco: abra **Sessões**, escolha a conversa e ela continua numa aba
nova, na pasta certa, com `claude --resume`. Sessões marcadas como **em uso**
foram gravadas nos últimos 2 minutos: feche a janela antiga para não haver duas
cópias da mesma conversa.

## Configuração

| Variável | Padrão | Para quê |
|---|---|---|
| `NEMUX_SHELL` | `powershell.exe -NoLogo` | Shell de cada aba (ex.: `cmd.exe`, `pwsh.exe`) |
| `NEMUX_DB` | `%APPDATA%\nemux\nemux.db` | Caminho do banco de memória (app e servidor MCP) |

O servidor MCP também aceita o caminho do banco como argumento:
`nemux-mcp.exe C:\outro\lugar\nemux.db`.

## Estrutura

```
nemux/
├── app/          nemux-app: interface Slint, terminais, seleção e painel de memória
│   ├── src/
│   └── ui/       arquivos .slint e o ícone (ui/icons)
├── memory/       nemux-memory: banco SQLite com busca FTS5
└── mcp-server/   nemux-mcp: servidor MCP (stdio) usado pelo Claude Code
```

Detalhes de arquitetura e cuidados de desenvolvimento estão no
[CLAUDE.md](CLAUDE.md).

## Desenvolvimento

```bash
cargo test --workspace                  # todos os testes
cargo test-model                        # só a lógica
cargo test-ui                           # só a interface (sem abrir janela)
cargo capturas                          # gera capturas PNG da interface em target/ui-capturas
cargo clippy --workspace --all-targets
cargo fmt --all
```

Os testes de interface rodam no backend de testes do Slint, sem janela. Os
testes de seleção abrem um `cmd.exe` de verdade no ConPTY, e os do painel de
memória usam sempre um banco temporário.

## Limitações conhecidas

- Só funciona no Windows por enquanto.
- Arrastar a seleção além da borda não rola o histórico, e não há triplo clique.
- Os painéis sempre dividem o espaço ao meio; ainda não dá para arrastar a divisória.
- `Shift+Enter` envia `ESC CR` (o mesmo que `Alt+Enter`): é o que o Claude Code
  entende como nova linha, e o ConPTY do Windows 10 descarta as codificações
  modernas da tecla. Programas que esperam um Shift+Enter "de verdade" não o recebem.
- O painel de memória ainda não cria nem edita memórias, só lista, busca e apaga.

## Licença

MIT.
