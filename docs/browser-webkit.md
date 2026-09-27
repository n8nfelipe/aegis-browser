# Backend WebKitGTK

`browser-webkit` é o primeiro backend Linux com engine real. Ele usa WebKitGTK
2.0.2 sobre a API WebKitGTK 4.1 disponível no sistema e cria uma janela GTK
com um `WebView`.

O perfil `Pessoal` usa um `WebsiteDataManager` persistente, portanto cookies,
cache e demais dados do WebKit são mantidos entre execuções em
`~/.local/share/aegis-browser/profiles/Pessoal` e
`~/.cache/aegis-browser/profiles/Pessoal`. Navegações passam pelo
`browser-shell` antes de chamar `load_uri`; eventos de início, progresso,
título, falha e conclusão retornam ao shell. Redirects e novas janelas passam
por uma decisão de política; downloads passam por autorização explícita.
Downloads agora pedem autorização, iniciam com `curl` (ou `wget` como fallback)
e usam `~/Downloads` como destino inicial para evitar o caminho nativo instável
do WebKitGTK em respostas grandes, como imagens ISO. O botão de downloads lista
os arquivos e persiste o histórico em `downloads-history.json`.
Para que a WebKitGTK 4.1 consiga expor favicons de sites como o Globo, cada
contexto usa um banco de ícones temporário, privado e com permissão `0700`; o
diretório é removido quando o browser é encerrado.
O botão `Configurações` abre o painel de segurança e aplica as mudanças ao
shell e ao filtro de navegação da engine.
Cada aba possui um `WebView`, um identificador de shell e uma engine próprios;
O contexto persistente é compartilhado pelo perfil entre as abas. O cabeçalho de
cada aba mostra o título da página, o domínio como fallback e o favicon da
`WebView` correspondente quando o site o disponibiliza. O ícone é limpo no
início de uma nova navegação e atualizado tanto pelo sinal `notify::favicon`
quanto ao final do carregamento, evitando reaproveitar o favicon do site
anterior. A superfície recebida é redimensionada proporcionalmente para
16×16 pixels antes de ser exibida na aba. JavaScript, WebGL e rolagem suave
ficam habilitados
para compatibilidade com interfaces modernas, com aceleração de hardware em
modo sob demanda. Os recursos HTML5 Media, MediaStream, Media Source Extensions
e WebAudio também ficam habilitados; vídeos inline podem ser reproduzidos após
uma ação do usuário, mas autoplay continua bloqueado por padrão.
As configurações também possuem páginas para tema GTK, buscador padrão (DuckDuckGo,
Brave Search, Startpage, Qwant, Bing ou Google) e perfis. Os temas Claro e Escuro
forçam variantes próprias do GTK; Sistema restaura a preferência do desktop.
Perfis adicionais criam contextos persistentes separados para novas abas;
abas já abertas não são movidas entre perfis nesta etapa.

A reprodução depende dos codecs GStreamer instalados no sistema. O backend não
instala plugins automaticamente: quando a WebKitGTK reporta um codec ausente,
a aba exibe essa causa em vez de conceder uma permissão silenciosa. Câmera,
microfone, DRM e outras capacidades privilegiadas de mídia também são negadas
até existir uma tela de decisão explícita. Para builds sem decodificador AVIF,
o backend aplica um fallback limitado a miniaturas WordPress AVIF, tentando
variantes raster no mesmo domínio (`.jpg`, `.jpeg`, `.png` e `.webp`) para o
nome original e para o nome redimensionado.

## Executar

```bash
cargo run -p aegis-browser-webkit
```

## Limitações atuais

- backend apenas Linux/GTK3;
- ainda não há particionamento real do WebKit por site superior;
- histórico, restauração de sessão e arrastar abas ainda não estão completos;
- permissões de câmera/microfone e certificados ainda precisam de handlers
  dedicados;
- compatibilidade de formatos de vídeo depende dos plugins GStreamer presentes
  na distribuição;
- downloads têm aprovação explícita e destino inicial em `~/Downloads`, mas
  ainda não passam por quarentena ou validação antimalware.
- o buscador padrão é persistido; as demais preferências ainda valem somente
  para a execução atual;
- cookies e armazenamento de sites são persistentes, mas ainda não há uma tela
  dedicada para limpar dados por site ou por perfil.

Este backend é um protótipo de integração, não uma versão pronta para
distribuição de segurança.
