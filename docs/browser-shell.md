# Browser shell

`browser-shell` é a camada de aplicação independente de GUI. Ela possui abas,
aba ativa, navegação pendente e delegação para o núcleo de permissões.

A GUI não deve analisar URLs, decidir se HTTP pode prosseguir ou gravar uma
permissão diretamente. Ela apenas chama o shell e exibe os resultados:

```text
navigate -> Navigated
         -> ConfirmationRequired -> confirm/cancel
         -> BlockedInsecure
         -> InvalidUrl
```

O shell também não renderiza páginas. A futura engine será conectada através de
um adaptador que receba a URL já aprovada e devolva eventos de carregamento,
título, erro e permissões solicitadas.

O trait `NavigationEngine` formaliza a primeira parte dessa fronteira. A engine
só recebe uma navegação depois da política aprová-la; se a engine rejeitar o
carregamento, a aba não muda de URL.

Eventos posteriores de carregamento atualizam apenas estado de apresentação da
aba: progresso, título e erro. O shell limita o título a 512 caracteres e
remove caracteres de controle antes de entregá-lo à GUI.

Redirects recebidos da engine devem passar por `evaluate_engine_redirect`. Como
redirect não representa um novo gesto do usuário, um redirect para HTTP é
bloqueado mesmo que a navegação original tenha sido iniciada em HTTPS.
