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

## Limites atuais

Não são executados `background` pages, service workers, popups, opções, native messaging,
`chrome.*` APIs, `webRequest` ou `declarativeNetRequest`. Padrões `http://` e `https://`
são aceitos; acesso `file://`, `chrome://` e outros esquemas privilegiados é recusado.

O código da extensão é executado em um mundo isolado do conteúdo da página. A implementação usa
as APIs de user content do WebKitGTK, portanto uma extensão completa da Chrome Web Store não é
compatível automaticamente.
