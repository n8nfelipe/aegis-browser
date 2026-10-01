# Manifesto do Aegis

<!-- impeccable:product-schema 1 -->

## Plataforma

Navegador para desktop, com Linux como primeira plataforma. O projeto começou
com um protótipo em egui e hoje usa GTK3 e WebKitGTK no backend Linux.

## Tecnologia

Aplicativo desktop escrito em Rust. O protótipo de interface usa egui; no Linux,
a interface e o motor de navegação são integrados por GTK3 e WebKitGTK.

## Para quem

Para quem quer navegar com mais privacidade e entender o que o navegador faz em
seu nome. Os controles de segurança precisam ser claros — e, quando possível,
fáceis de rever.

## Por que existe

O Aegis parte de escolhas prudentes: proteger por padrão, explicar decisões que
afetam a navegação e não coletar telemetria. Segurança não deve depender de
opções escondidas nem de o usuário adivinhar o que está acontecendo.

## O que orienta o projeto

- Linux é a primeira plataforma desktop atendida.
- Preferimos HTTPS; uma exceção para HTTP deve ser explícita.
- No backend atual, o contexto do WebKitGTK é temporário.
- Uma configuração só vale de verdade se mudar a política do navegador, não
  apenas o que aparece na tela.
- Extensões e isolamento completo de dados por site ainda não fazem parte da
  versão inicial.

## Princípios

- Privacidade é o ponto de partida, não um recurso opcional.
- Decisões de segurança devem ser visíveis e, sempre que possível, reversíveis.
- Regras pequenas e testáveis são mais fáceis de entender e manter.
- O navegador não deve ampliar silenciosamente o acesso à rede, aos arquivos ou
  à execução de código.

## Acessibilidade

As configurações devem usar controles nativos, rótulos claros e navegação por
teclado. Cor não basta para comunicar uma escolha: cada controle precisa de uma
explicação que faça sentido por si só.
