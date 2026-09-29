# Aegis Browser

Navegador desktop para Linux com foco em privacidade, segurança explícita e
transparência. O backend executável usa Rust, GTK3 e WebKitGTK.

> Status: protótipo navegável. O projeto ainda não deve ser tratado como um
> navegador de produção ou como uma implementação completa das extensões do
> Chrome.

## Destaques

- **Leve:** interface nativa em GTK3, arquitetura modular e baixo número de
  componentes executados por padrão;
- **Rápido:** inicialização direta, navegação baseada em WebKitGTK e decisões de
  segurança mantidas em um núcleo pequeno;
- **Seguro:** HTTPS obrigatório por padrão, downloads exigem autorização, permissões
  privilegiadas controladas e ausência de telemetria por padrão;
- **Em evolução:** o Aegis ainda não contempla todas as funcionalidades de um
  navegador completo. Recursos como favoritos persistentes, quarentena de downloads,
  permissões avançadas e suporte completo a
  extensões Chrome ainda estão em desenvolvimento.

## Sumário

- [Objetivos](#objetivos)
- [Requisitos](#requisitos)
- [Instalação](#instalação)
- [Execução](#execução)
- [Uso](#uso)
- [Segurança e privacidade](#segurança-e-privacidade)
- [Extensões](#extensões)
- [Bitwarden](#bitwarden)
- [Desenvolvimento](#desenvolvimento)
- [Solução de problemas](#solução-de-problemas)
- [Limitações conhecidas](#limitações-conhecidas)
- [Documentação complementar](#documentação-complementar)

## Objetivos

- reduzir rastreamento entre sites por padrão;
- bloquear navegação HTTP insegura por padrão;
- limitar o impacto de páginas e extensões comprometidas;
- tornar decisões de segurança visíveis e reversíveis;
- manter o núcleo de políticas pequeno, determinístico e testável;
- não coletar telemetria por padrão;
- evitar privilégios de extensão que não sejam necessários para o uso básico.

## Requisitos

O ambiente de desenvolvimento precisa ter:

- Linux desktop x86_64;
- Rust e Cargo;
- GTK3 e os pacotes de desenvolvimento GTK3;
- WebKitGTK 4.1 e os pacotes de desenvolvimento correspondentes;
- `pkg-config`;
- GStreamer e os codecs desejados para reprodução de mídia.

O instalador verifica `gtk+-3.0` e `webkit2gtk-4.1` antes de compilar. Os
nomes dos pacotes variam por distribuição. Em sistemas baseados em Debian,
normalmente são necessários pacotes equivalentes a `build-essential`,
`cargo`, `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, `pkg-config` e os plugins
GStreamer.

## Instalação

Na raiz do projeto:

```bash
bash packaging/install.sh
```

O instalador:

1. compila `aegis-browser-webkit` em modo release;
2. instala o executável no diretório de binários do usuário;
3. instala o ícone e o arquivo `.desktop`;
4. cria o diretório de extensões;
5. não usa `sudo` e não inicia o navegador automaticamente.

Por padrão, os arquivos são instalados em:

```text
~/.local/bin/aegis-browser
~/.local/share/applications/org.aegis.Browser.desktop
~/.local/share/icons/hicolor/
~/.local/share/aegis-browser/extensions/
```

Para instalar um binário já compilado:

```bash
cargo build --release -p aegis-browser-webkit
AEGIS_SKIP_BUILD=1 bash packaging/install.sh
```

Os diretórios podem ser personalizados:

```bash
AEGIS_BIN_DIR="$HOME/.local/bin" \
AEGIS_DATA_DIR="$HOME/.local/share" \
bash packaging/install.sh
```

Para remover o executável, o atalho e os ícones instalados:

```bash
bash packaging/uninstall.sh
```

Se a instalação usou `AEGIS_BIN_DIR` ou `AEGIS_DATA_DIR`, use os mesmos valores
na desinstalação. O diretório de extensões e outros dados do usuário não são
apagados automaticamente.

## Execução

Após a instalação:

```bash
/home/$USER/.local/bin/aegis-browser
```

Ou abra **Aegis Browser** pelo menu de aplicativos. Durante o desenvolvimento,
é possível executar diretamente:

```bash
cargo run -p aegis-browser-webkit
```

`cargo run` não registra um novo lançador no menu. Para associar o processo ao
ícone e ao `application ID` `org.aegis.Browser`, execute o instalador uma vez.

## Uso

### Navegação

- digite uma URL na barra de endereço e pressione `Enter` ou clique em **Ir**;
- textos que não parecem URLs são enviados ao buscador configurado;
- os botões **voltar** e **avançar** controlam o histórico da aba;
- links que pedem uma nova janela, como muitos links do Gmail, são abertos na
  aba atual porque o protótipo ainda não cria WebViews de popup;
- downloads aguardam sua autorização e a escolha explícita do destino;
- URLs `mailto:`, `javascript:`, `file:` e outros esquemas não web não são
  tratados como navegação normal.

### Indicador de conexão

O indicador ao lado da barra de endereço mostra:

- cadeado verde: página HTTPS;
- alerta amarelo: página HTTP ou navegação insegura bloqueada;
- indicador neutro: nenhuma página ativa.

O indicador acompanha a URL real da WebView após redirecionamentos e ao trocar
de aba. HTTPS indica apenas o esquema da conexão; não é uma garantia de que o
conteúdo do site seja confiável.

### Abas e perfis

Cada aba possui uma WebView e um identificador próprios. O botão `+` cria uma
aba, e o botão `×` fecha a aba atual. Perfis adicionais recebem contextos
persistentes separados para novas abas.

### Barra de favoritos

Em **Configurações → Aparência**, ative **Mostrar barra de favoritos**. O botão
de estrela salva a página atual na barra. Use o botão do endereço para navegar
até o favorito e o botão **Remover** ao lado dele para excluí-lo.

Os favoritos são persistidos em `bookmarks.json` dentro do diretório de dados
do usuário.

### Configurações

As configurações atuais incluem:

- tema do GTK: sistema, claro ou escuro;
- buscador padrão: DuckDuckGo, Brave, Startpage, Qwant, Bing ou Google;
- perfis separados;
- política HTTPS e exceções HTTP;
- barra de favoritos;
- instalação e remoção de extensões compatíveis.

O buscador padrão é salvo em `~/.local/share/aegis-browser/preferences.json`;
as demais preferências ainda valem somente para a execução atual, salvo a
instalação de extensões no diretório de dados.

## Segurança e privacidade

### Política de navegação

O navegador começa com HTTPS obrigatório. HTTP pode ser:

- permitido para loopback, como `localhost`, quando a opção estiver ativa;
- confirmado explicitamente pelo usuário, quando a exceção estiver ativa;
- bloqueado sem carregamento em qualquer outro caso.

Toda navegação de nível superior passa pelo `browser-shell` e pelo filtro do
backend WebKitGTK. Recursos internos e sub-recursos não recebem automaticamente
as mesmas decisões de uma navegação de nível superior.

### Dados do WebKit

O perfil padrão `Pessoal` usa armazenamento persistente. Cookies, cache,
armazenamento local e sessões de sites são mantidos entre execuções em:

```text
~/.local/share/aegis-browser/profiles/Pessoal/
~/.cache/aegis-browser/profiles/Pessoal/
```

Perfis adicionais usam diretórios separados. Isso permite manter contas e
sessões isoladas por perfil, mas também significa que os dados persistem no
disco até serem removidos pelo usuário.

O banco temporário de favicons é criado com permissão restrita e removido ao
encerrar o processo. Não há telemetria implementada por padrão.

### Mídia e permissões

JavaScript, WebGL, WebAudio, MediaStream e Media Source Extensions ficam
habilitados para compatibilidade. Autoplay continua exigindo gesto do usuário.
Câmera, microfone, DRM e outras capacidades privilegiadas não recebem permissão
silenciosa. A reprodução depende dos codecs GStreamer disponíveis no sistema.

### Clipboard

O WebKit está configurado para permitir a API JavaScript de clipboard usada por
sites como o Codex. Em um terminal Linux, copiar a saída do terminal normalmente
usa `Ctrl+Shift+C`, e colar usa `Ctrl+Shift+V`; `Ctrl+C` interrompe o processo.

## Extensões

O Aegis carrega um subconjunto controlado de extensões Chrome/WebExtension
descompactadas. A instalação é feita por **Configurações → Extensões**:

1. clique em **Adicionar extensão**;
2. selecione um ou mais arquivos `manifest.json`;
3. confirme o caminho da extensão;
4. reinicie o navegador ou crie novas abas para aplicar content scripts;
5. use **Remover** para desinstalar uma extensão.

Também é possível instalar manualmente em:

```text
~/.local/share/aegis-browser/extensions/<id-da-extensao>/
```

Durante o desenvolvimento, use outro diretório com:

```bash
AEGIS_EXTENSIONS_DIR="$PWD/extensions" cargo run -p aegis-browser-webkit
```

### Manifesto mínimo

O arquivo precisa usar os nomes oficiais do manifesto Chrome, incluindo
`manifest_version`, `content_scripts`, `exclude_matches`, `run_at` e
`all_frames`:

```json
{
  "manifest_version": 3,
  "name": "Minha extensão",
  "version": "1.0.0",
  "content_scripts": [
    {
      "matches": ["https://example.com/*"],
      "exclude_matches": ["https://example.com/private/*"],
      "js": ["content.js"],
      "css": ["content.css"],
      "run_at": "document_start",
      "all_frames": false
    }
  ]
}
```

São carregados:

- `manifest_version` 2 e 3;
- `content_scripts.matches`;
- `exclude_matches`;
- arquivos JavaScript e CSS;
- `run_at` `document_start`, `document_end` e `document_idle`;
- `all_frames`;
- ícones declarados pelo manifesto;
- popups declarados em `action.default_popup` ou
  `browser_action.default_popup`, quando aplicável.

### Limites das extensões

Não são executados:

- páginas `background` e service workers;
- popups completos e páginas de opções com runtime Chrome;
- APIs `chrome.*`;
- `webRequest`;
- `declarativeNetRequest`;
- native messaging;
- acesso `file://`, `chrome://` ou outros esquemas privilegiados.

O código de content script é inserido em um mundo isolado do conteúdo da
página. Portanto, uma extensão completa da Chrome Web Store não é compatível
automaticamente. A extensão oficial do Bitwarden para Chrome, por exemplo,
precisa do runtime completo; para o Aegis, use a integração nativa descrita na
seção seguinte.

Mais detalhes estão em [docs/extensions.md](docs/extensions.md).

## Bitwarden

O Aegis possui uma integração nativa com o Bitwarden CLI. Ela não executa a
extensão Chrome do Bitwarden; consulta o cofre pelo CLI e injeta a credencial
selecionada no formulário da página.

### Servidor europeu

O servidor usado pela integração é:

```text
https://vault.bitwarden.eu
```

Configure o CLI uma vez:

```bash
bw config server https://vault.bitwarden.eu
bw login
```

Antes de iniciar o navegador, desbloqueie o cofre e exporte a sessão no mesmo
terminal:

```bash
export BW_SESSION="$(bw unlock --raw)"
/home/$USER/.local/bin/aegis-browser
```

O navegador iniciado pelo menu gráfico não herda automaticamente uma variável
`BW_SESSION` criada em outro terminal. Por isso, use o mesmo terminal durante o
teste ou crie um lançador que exporte a sessão antes de iniciar o browser.

Verifique o estado com:

```bash
bw status
```

O resultado esperado é semelhante a:

```json
{
  "serverUrl": "https://vault.bitwarden.eu",
  "status": "unlocked"
}
```

Com o cofre desbloqueado, clique no botão **Bitwarden**. O diálogo lista todas
as credenciais de login, colocando primeiro as que correspondem à origem da
página atual. A lista possui barra de rolagem.

O preenchimento procura campos visíveis por `name`, `id`, `autocomplete`,
placeholder e `aria-label`, dispara eventos `input`/`change` e tenta novamente
por alguns instantes para formulários dinâmicos. Se a página não tiver um
formulário de login, não haverá campo para preencher; isso é esperado na caixa
de entrada já autenticada do Gmail. Em fluxos de login em duas etapas, selecione
a credencial novamente após a página exibir o segundo campo.

O Bitwarden mantém cofres separados por região. Uma conta criada no servidor
americano não aparece automaticamente no servidor europeu.

Documentação oficial: [Bitwarden CLI](https://bitwarden.com/help/cli/).

## Desenvolvimento

### Estrutura do workspace

```text
crates/policy-core/       decisões determinísticas de navegação
crates/url-adapter/       parsing de URLs e origens web
crates/storage-core/      semântica de armazenamento particionado
crates/permissions-core/  permissões conservadoras por origem
crates/browser-shell/     estado de abas e coordenação de navegação
crates/browser-ui/        protótipo visual egui
crates/browser-webkit/    backend Linux real com GTK3/WebKitGTK
assets/                   ícones e metadados do desktop
docs/                     threat model, limites e políticas
packaging/                instalação, desinstalação e arquivo .desktop
```

### Compilar e testar

Testes de todo o workspace:

```bash
cargo test --workspace
```

Testes e build do backend executável:

```bash
cargo fmt --all
cargo test -p aegis-browser-webkit --offline
cargo build --release -p aegis-browser-webkit --offline
```

O backend executável usa o membro padrão do workspace. O pacote
`aegis-browser-ui` é apenas um protótipo visual egui e não renderiza páginas web.

### Variáveis de ambiente

| Variável | Finalidade |
| --- | --- |
| `AEGIS_BIN_DIR` | diretório do executável instalado |
| `AEGIS_DATA_DIR` | diretório de dados, ícones e lançador |
| `AEGIS_SKIP_BUILD=1` | instala um release já compilado |
| `AEGIS_EXTENSIONS_DIR` | diretório alternativo de extensões |
| `AEGIS_BITWARDEN_BIN` | caminho alternativo para o executável `bw` |
| `BW_SERVER` | servidor do Bitwarden em comandos manuais; o Aegis força `https://vault.bitwarden.eu` |
| `BW_SESSION` | sessão do cofre desbloqueado usada pelo Bitwarden CLI |

### Regras para mudanças

Mudanças que ampliem privilégios, acesso à rede, acesso ao disco ou execução de
código devem atualizar:

- [docs/security-boundaries.md](docs/security-boundaries.md);
- [docs/threat-model.md](docs/threat-model.md), quando o modelo de ameaça mudar;
- testes do crate afetado;
- este README, quando o comportamento visível do usuário mudar.

## Solução de problemas

### O ícone não aparece no dock ou no menu

Reinstale o pacote e abra pelo lançador instalado:

```bash
bash packaging/install.sh
/home/$USER/.local/bin/aegis-browser
```

Não mantenha uma instância antiga aberta ao testar um novo binário.

### O Gmail informa que o navegador não é seguro

O backend configura um User-Agent compatível com Chrome para evitar o bloqueio
preliminar do Gmail contra WebKitGTK identificado como navegador incorporado.
Feche todas as janelas e abra o Aegis novamente após uma atualização. O motor
continua sendo WebKitGTK, portanto algumas APIs específicas do Chromium podem
continuar indisponíveis.

### Links do Gmail não abrem

Links que pedem nova janela são redirecionados para a aba atual. Se o problema
persistir, recarregue o Gmail depois de reiniciar o navegador e confirme que a
URL começa com `https://`.

### O Bitwarden mostra cofre bloqueado

Execute no mesmo terminal que inicia o navegador:

```bash
bw config server https://vault.bitwarden.eu
export BW_SESSION="$(bw unlock --raw)"
/home/$USER/.local/bin/aegis-browser
```

Se `bw status` mostrar `unauthenticated`, faça `bw login`. Se mostrar uma conta
incorreta, confirme a região e o e-mail usado no servidor europeu.

### O Bitwarden não encontra ou não preenche uma credencial

Confirme que:

1. o estado é `unlocked`;
2. a credencial é um item do tipo login, com usuário e senha;
3. a página possui um campo de usuário ou senha visível;
4. em um login de duas etapas, o botão Bitwarden foi acionado novamente após o
   segundo campo aparecer;
5. o navegador foi reiniciado depois da instalação da atualização.

### Uma extensão não funciona

Verifique se o manifesto usa `manifest_version` e `content_scripts` com nomes
snake_case. O Aegis executa content scripts, mas não executa service workers,
background pages nem APIs privilegiadas do Chrome.

### Vídeos ou imagens não carregam

Instale os plugins GStreamer adequados à distribuição. O Aegis não instala
codecs automaticamente. Downloads exigem confirmação e escolha explícita do destino.

## Limitações conhecidas

- somente Linux/GTK3 nesta etapa;
- apenas o buscador padrão persiste entre execuções; as demais preferências
  ainda são temporárias;
- não há sincronização de sessão ou restauração de abas;
- não há arrastar/reordenar abas completo;
- não há suporte completo à WebExtension Chrome;
- não há runtime para service workers, background pages ou APIs `chrome.*`;
- popups são convertidos para navegação na aba atual;
- downloads exigem aprovação e escolha explícita do destino (com `~/Downloads`
  como padrão), mas ainda não passam por quarentena ou validação antimalware;
- o botão de downloads mostra os arquivos encontrados e mantém um histórico em
  `~/.local/share/aegis-browser/downloads-history.json`;
- permissões de câmera, microfone, certificados e DRM ainda precisam de telas
  de decisão dedicadas;
- compatibilidade de mídia depende dos codecs da distribuição;
- não há particionamento completo do WebKit por site superior;
- a integração Bitwarden depende de uma sessão CLI desbloqueada e preenche
  apenas campos acessíveis no documento principal da página.

Este backend é uma integração experimental e não uma versão pronta para
distribuição em ambientes que exigem garantias de segurança de produção.

## Documentação complementar

- [Backend WebKitGTK](docs/browser-webkit.md)
- [Extensões](docs/extensions.md)
- [Limites de segurança](docs/security-boundaries.md)
- [Modelo de ameaça](docs/threat-model.md)
- [Interface do navegador](docs/browser-ui.md)

## Licença

Consulte os arquivos de licença e os cabeçalhos dos componentes distribuídos
com o projeto.
