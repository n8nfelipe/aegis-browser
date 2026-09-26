# Modelo de ameaça

## Escopo

O Aegis protege um usuário desktop contra conteúdo web não confiável e contra
rastreamento cross-site. O modelo inicial assume que o sistema operacional,
kernel, firmware e o binário distribuído não foram comprometidos.

## Adversários

### Página maliciosa

Pode controlar HTML, JavaScript, CSS, WebAssembly, iframes, redirects,
downloads e respostas de rede do próprio domínio. Pode tentar explorar bugs no
renderer, confundir a UI, abusar de permissões ou correlacionar o usuário.

### Rastreador cross-site

Pode controlar recursos incorporados em vários sites e tentar usar cookies,
cache, armazenamento, conexões, APIs de mídia ou características do dispositivo
para reconhecer o mesmo usuário.

### Extensão comprometida

Uma extensão autorizada pode observar páginas correspondentes ao manifesto,
modificar conteúdo e tentar escapar de suas permissões. O primeiro suporte do
Aegis limita-se a content scripts JS/CSS, usa um mundo isolado, recusa esquemas
privilegiados e valida que os recursos permaneçam dentro do diretório da
extensão.

### Download malicioso

Pode tentar induzir execução, explorar o visualizador de arquivos ou ocultar sua
origem e tipo real.

## Objetivos de segurança

- um renderer comprometido não deve acessar livremente o sistema de arquivos;
- conteúdo de sites diferentes deve permanecer em processos e contextos
  isolados;
- decisões privilegiadas devem ocorrer fora do renderer;
- permissões devem ser específicas por origem, explícitas e revogáveis;
- estado de terceiros não deve ser reutilizável entre sites superiores;
- atualizações devem ser autenticadas e permitir rollback;
- falhas em uma aba devem causar o menor impacto possível nas demais.

## Fora do escopo inicial

- anonimato contra um observador global de rede;
- proteção contra sistema operacional ou conta local comprometidos;
- resistência a fingerprinting perfeito;
- garantia de compatibilidade com todas as extensões existentes;
- engine de renderização escrita do zero.

## Propriedades verificáveis

Cada objetivo deve ter um teste, uma inspeção automatizada ou uma evidência de
build associada. Alegações de privacidade não devem depender apenas de texto de
marketing.
