# Extensões

O backend WebKitGTK suporta um subconjunto controlado de extensões Chrome/WebExtension descompactadas.

## Instalação

Crie uma pasta por extensão em:

```text
~/.local/share/aegis-browser/extensions/<id-da-extensao>/
```

A pasta precisa conter um `manifest.json` com `manifest_version` 2 ou 3. Nesta primeira versão são carregados:

- `content_scripts.matches` e `exclude_matches`;
- arquivos `js` e `css` dos content scripts;
- `run_at` (`document_start`, `document_end` e `document_idle`);
- `all_frames`.

Exemplo mínimo:

```json
{
  "manifest_version": 3,
  "name": "Minha extensão",
  "version": "1.0.0",
  "content_scripts": [
    {
      "matches": ["https://example.com/*"],
      "js": ["content.js"],
      "css": ["content.css"],
      "run_at": "document_start"
    }
  ]
}
```

Também é possível usar outro diretório durante o desenvolvimento definindo `AEGIS_EXTENSIONS_DIR`.
As extensões são descobertas quando cada aba é criada; reinicie o browser para aplicar mudanças no manifesto.
Também é possível adicionar uma extensão pela aba **Configurações → Extensões**. Navegue até
a pasta e selecione o arquivo `manifest.json`; a pasta pai será copiada para o diretório de
extensões do Aegis.

## Extensão incluída

O repositório contém a extensão de exemplo `extensions/aegis-privacy-guard`. Ela remove
parâmetros conhecidos de rastreamento dos links e oculta sobreposições comuns de cookies e
newsletter. Para instalá-la durante o desenvolvimento, selecione
`extensions/aegis-privacy-guard/manifest.json` em **Configurações → Extensões**.

Também acompanha o repositório a extensão `extensions/aegis-tracker-blocker`, que remove
do DOM scripts, pixels, iframes e outros recursos de domínios conhecidos de publicidade,
analytics e telemetria. Ela também limpa parâmetros de rastreamento dos links. A lista de
domínios fica no próprio `content.js`, sem baixar listas externas ou enviar dados de navegação.
Para instalá-la manualmente, selecione
`extensions/aegis-tracker-blocker/manifest.json` em **Configurações → Extensões**.

Ao usar `packaging/install.sh`, ela é copiada automaticamente para o diretório de extensões
do usuário e seu ícone aparece na barra superior. O instalador não sobrescreve uma cópia que já
tenha sido gerenciada pelo usuário.

Como o Aegis ainda não expõe `webRequest` ou APIs `chrome.*`, essa extensão não bloqueia
requisições de rede. Ela funciona apenas com o subconjunto de content scripts e popup suportado
atualmente.

O Tracker Blocker segue a mesma limitação: recursos inseridos por scripts podem ser removidos
depois que o navegador já iniciou a requisição, e chamadas de rede feitas diretamente por APIs
JavaScript da página não são interceptadas. O bloqueio completo de rede exige suporte do
navegador a `webRequest`, `declarativeNetRequest` ou a um filtro de conteúdo nativo.

## Limites atuais

Não são executados `background` pages, service workers, popups, opções, native messaging,
`chrome.*` APIs, `webRequest` ou `declarativeNetRequest`. Padrões `http://` e `https://`
são aceitos; acesso `file://`, `chrome://` e outros esquemas privilegiados é recusado.

O código da extensão é executado em um mundo isolado do conteúdo da página. A implementação usa
as APIs de user content do WebKitGTK, portanto uma extensão completa da Chrome Web Store não é
compatível automaticamente.
