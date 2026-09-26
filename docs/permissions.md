# Permissões por origem

## Estado padrão

Toda permissão começa em `Ask`. O browser só deve exibir prompt depois que a
requisição passar pelas condições de contexto seguro, origem e gesto do usuário.

## Regras do MVP

- contexto HTTP não recebe permissões sensíveis;
- iframes e sub-recursos não recebem permissões sensíveis diretamente;
- gesto explícito do usuário é obrigatório;
- concessões e negações ficam vinculadas ao perfil e à origem;
- limpar os dados de uma origem também deve oferecer limpar suas permissões;
- o renderer nunca grava diretamente o estado de permissões.

## Permissões cobertas

O núcleo já modela câmera, microfone, geolocalização, notificações e leitura ou
escrita na área de transferência. A engine deverá adaptar cada API para esse
mesmo fluxo antes de expô-la à página.
