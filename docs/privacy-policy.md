# Política de privacidade técnica

## Padrões do MVP

- nenhuma telemetria ou identificador persistente enviado pelo navegador;
- cookies de terceiros bloqueados ou particionados;
- cache, DNS, conexões e armazenamento de terceiros particionados por site
  superior quando suportado pela engine;
- permissões negadas por padrão quando não forem necessárias para a navegação;
- dados de sessão descartável apagados ao encerrar o perfil;
- listas de bloqueio atualizadas por artefatos autenticados;
- sincronização e serviços remotos não serão adicionados sem opt-in explícito.

## Transparência

O usuário deve conseguir ver, por origem:

- permissões concedidas;
- cookies e armazenamento persistente;
- requisições bloqueadas por política;
- exceções de HTTPS;
- extensões e suas permissões.

## Trade-offs

Privacidade pode quebrar login federado, widgets incorporados e fluxos de
pagamento. Exceções devem ser temporárias, específicas e visíveis, nunca uma
liberação global silenciosa.
