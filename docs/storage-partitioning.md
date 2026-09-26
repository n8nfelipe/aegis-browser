# Particionamento de armazenamento

## Regra

Estado de terceiros deve ser indexado por:

```text
(perfil, site superior, origem do recurso)
```

Assim, o mesmo recurso incorporado em dois sites superiores não recebe a mesma
partição. Perfis regular e privado também nunca compartilham a mesma chave.

## Responsabilidades

O adaptador da engine é responsável por:

1. fazer parsing completo da URL;
2. normalizar esquema, host, porta, IPv6 e IDNA;
3. calcular o site registrável usando uma Public Suffix List atualizada;
4. rejeitar origens opacas ou esquemas não suportados;
5. construir `Origin` e `TopLevelSite` para o `policy-core`.

O crate `url-adapter` já cobre os itens 1, 2 e 4 para URLs `http` e `https`.
O cálculo do site registrável (item 3) continuará separado até integrarmos uma
Public Suffix List versionada e atualizada.

O `policy-core` é responsável apenas por representar e comparar identidades já
normalizadas. Ele não deve tentar calcular o domínio registrável por conta
própria.

## Estado coberto

O mesmo particionamento deve ser aplicado de forma consistente a cookies,
localStorage, IndexedDB, Cache Storage, Service Workers, HTTP cache, conexões,
DNS cache, HSTS, prefetch, WebRTC device IDs e quaisquer outros mecanismos que
possam criar identificadores reutilizáveis.

O crate `storage-core` fornece uma implementação em memória dessa semântica e
testa que sites superiores, áreas de armazenamento e perfis não compartilham
estado por acidente. Uma implementação persistente deverá manter exatamente a
mesma chave composta.

## Exceções

Uma exceção de acesso não particionado deve ser:

- específica para uma origem e um site superior;
- temporária ou revogável;
- visível ao usuário;
- registrada sem armazenar conteúdo sensível;
- coberta por teste de compatibilidade e de regressão de privacidade.
