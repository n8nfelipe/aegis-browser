# Fronteiras de segurança

## Componentes planejados

| Componente | Privilégio esperado | Regra principal |
| --- | --- | --- |
| Browser/UI | alto | não processa conteúdo web como confiável |
| Renderer | baixo | sem acesso direto ao disco, credenciais ou sockets arbitrários |
| Network service | baixo/restrito | aplica proxy, DNS, HTTPS e filtros antes de entregar dados |
| Storage service | controlado | chaves de armazenamento incluem o perfil e o site superior |
| Download service | controlado | grava em quarentena e exige validações antes de expor o arquivo |
| Update service | separado | aceita somente artefatos assinados e compatíveis |
| Extension host | mínimo | somente `manifest_version` 2/3, content scripts JS/CSS, padrões `http/https`, isolamento por mundo e sem APIs privilegiadas |
| URL adapter | baixo | usa parser WHATWG; rejeita esquemas não web e credenciais embutidas |

## Regras não negociáveis

1. O renderer não decide permissões.
2. O renderer não escolhe livremente o destino de uma navegação privilegiada.
3. URLs devem ser analisadas por um único componente confiável antes de qualquer
   comparação de origem, host ou esquema.
4. Mensagens entre processos devem validar tipo, tamanho, origem e estado.
5. Interfaces Mojo/IPC novas devem documentar o chamador, o callee e os dados
   sensíveis envolvidos.
6. Logs não devem conter URLs completas, cookies, tokens ou conteúdo de páginas.
7. Flags de debug que desabilitem sandbox, Site Isolation ou verificação de
   certificados não podem chegar a builds de distribuição.
8. Extensões devem carregar somente recursos relativos ao próprio diretório
   canônico; caminhos absolutos e escapes por symlink devem ser recusados.

## Regra de mudança

Qualquer alteração que aumente privilégio, conectividade, persistência ou
superfície de extensão deve atualizar este documento e incluir um teste de
regressão.
