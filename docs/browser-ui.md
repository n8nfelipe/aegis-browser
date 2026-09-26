# GUI desktop

`browser-ui` é a primeira interface nativa do projeto. Ela usa `eframe/egui`
para desenhar abas, omnibox, indicador HTTPS e confirmação de HTTP.
Também oferece uma área lateral de configurações com as opções de HTTPS,
exceções HTTP e HTTP local.

A GUI não recebe acesso ao parser de URL nem às estruturas internas de
armazenamento. Toda ação passa pelo `browser-shell`, que continua responsável
por política, navegação e permissões.

## Executar

```bash
cargo run -p aegis-browser-ui
```

Esta janela é apenas um shell visual e mostra um placeholder no conteúdo. Para
navegar com uma engine real, use `cargo run` ou `cargo run -p
aegis-browser-webkit`. As preferências da interface visual são uma prévia do
mesmo conjunto de políticas usado pelo backend WebKitGTK.
