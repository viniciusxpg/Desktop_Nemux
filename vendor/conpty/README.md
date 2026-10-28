# ConPTY da Microsoft (versão nova)

`conpty.dll` e `OpenConsole.exe` (x64) do pacote NuGet oficial
[`Microsoft.Windows.Console.ConPTY`](https://www.nuget.org/packages/Microsoft.Windows.Console.ConPTY),
versão `1.25.260930003`, do projeto [microsoft/terminal](https://github.com/microsoft/terminal).
Licença MIT (ver `LICENSE`). Os dois arquivos têm assinatura digital da Microsoft Corporation.

| Arquivo | SHA-256 |
|---|---|
| `conpty.dll` | `feeef341d891643c62d30b6b07800bc70f0bc148f44cb8c3bee84aa557ae805a` |
| `OpenConsole.exe` | `3d66b23d0a71bb8eed2b77edc8b9df9bf54ce6c8fb74c863a30e760997f80586` |

## Por que estão aqui

O `portable-pty` usa um `conpty.dll` já carregado (ou ao lado do executável) no
lugar do ConPTY embutido no Windows. O do Windows 10 é antigo: ele redesenha a tela
em vez de repassar a saída do programa, e por isso esconde do Nemux que o
Claude Code usa a tela alternativa e o mouse. Sem a versão nova, a roda do
mouse não chega ao Claude e a conversa não rola. O Windows Terminal e o WezTerm
usam a mesma solução.

Os dois arquivos ficam **embutidos** no `nemux-app.exe` (`app/src/conpty.rs`):
ao abrir o primeiro terminal, o Nemux os extrai para
`%LOCALAPPDATA%\nemux\conpty\<versão>\` e carrega o `conpty.dll` de lá. Assim o
`.exe` funciona sozinho, copiado para qualquer pasta. Se a extração falhar, o app
continua funcionando com o ConPTY do Windows. `NEMUX_SYSTEM_CONPTY=1` força o do
Windows, para comparar.

## Atualizar

Baixe o `.nupkg` mais novo do link acima, extraia
`runtimes/win-x64/native/conpty.dll` e `build/native/runtimes/x64/OpenConsole.exe`,
confira a assinatura (`Get-AuthenticodeSignature`) e atualize a versão e os
hashes desta página e a constante `VERSION` em `app/src/conpty.rs` (ela nomeia a
pasta da extração).
